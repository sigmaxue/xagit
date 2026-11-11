//! The built-in conflict resolution view (an xagit addition): the A/B/both
//! part of Emacs Ediff's three-way merge session.  `e` (ediff dwim) opens it
//! when there are unmerged conflicts; each conflict region is shown with its
//! A (ours) and B (theirs) versions and a per-region decision.  a/b/d/u set
//! the decision, n/p move between conflicts, C-c C-c writes the file back
//! and stages it, C-c C-k cancels without touching the file.

use super::*;
use crate::ui::resolve::{Choice, ResolveFile};

/// The state of the conflict resolution view.
#[derive(Debug, Clone)]
pub struct ResolveEdit {
    pub file: ResolveFile,
    /// index of the conflict the cursor is on
    pub cursor: usize,
    /// first display row shown (free vertical scrolling)
    pub top: usize,
    /// C-c was typed
    pub cc: bool,
    /// any decision changed
    pub modified: bool,
}

const HELP: &str = "\
Resolve conflicts — choose a version for each conflict, then C-c C-c

C-c C-c  save the file and stage it (git add)
C-c C-k  cancel, leaving the file untouched
a        keep A (ours / upper) for the conflict at point
b        keep B (theirs / lower)
d        keep both versions, one after the other
s        keep the base version (diff3 conflicts only)
u        leave this conflict undecided (markers stay)
n / p    next / previous conflict (scrolls it into view)
SPC/DEL  page down / up (free scroll)
up/down  scroll one line
?        show this help";

impl App {
    /// `e` (magit-ediff-dwim) with conflicts: pick the unmerged file at
    /// point, else the first one, and open the resolution view.  Returns
    /// false when there are no conflicts (the caller shows a text diff).
    pub(super) fn resolve_open(&mut self) -> Result<bool> {
        let unmerged = self.git.unmerged_files();
        if unmerged.is_empty() {
            return Ok(false);
        }
        let file = self
            .current_line()
            .and_then(|l| match &l.kind {
                LineKind::File { path, .. } => unmerged.iter().find(|f| *f == path).cloned(),
                _ => None,
            })
            .or_else(|| unmerged.first().cloned())
            .unwrap();
        let abs = self.git.toplevel.join(&file);
        let text = std::fs::read(&abs).map_err(|e| anyhow::anyhow!("Cannot read {file}: {e}"))?;
        let parsed = ResolveFile::parse(&file, &text);
        if parsed.regions.is_empty() {
            self.set_msg(&format!("No conflict markers in {file} (use E m for git mergetool)"));
            return Ok(false);
        }
        self.resolve = Some(ResolveEdit { file: parsed, cursor: 0, top: 0, cc: false, modified: false });
        self.setup_buffer(BufferMode::Resolve, Vec::new());
        self.set_msg("a/b/d choose a side, C-c C-c saves and stages, ? for help");
        Ok(true)
    }

    /// The decisions of the conflicts being edited (tests).
    pub fn resolve_choices(&self) -> Option<Vec<Choice>> {
        self.resolve.as_ref().map(|e| e.file.regions.iter().map(|r| r.choice).collect())
    }

    pub fn resolve_cursor(&self) -> Option<usize> {
        self.resolve.as_ref().map(|e| e.cursor)
    }

    /// The resolved file bytes without writing (tests).
    pub fn resolve_result(&self) -> Option<String> {
        self.resolve.as_ref().map(|e| String::from_utf8_lossy(&e.file.resolved()).into_owned())
    }

    /// The rendered rows of the resolution view as (text, spans with their
    /// styles) for tests.  W is the simulated terminal width.
    pub fn resolve_view(&self, body: usize, w: usize) -> Vec<(String, Vec<(String, Style)>)> {
        let (rows, _) = self.resolve_rows(body, w);
        rows.into_iter()
            .map(|r| {
                let text = r.plain();
                let spans = r.spans.into_iter().map(|s| (s.text, s.style)).collect();
                (text, spans)
            })
            .collect()
    }

    /// The view's current scroll position (tests).
    pub fn resolve_top(&self) -> Option<usize> {
        self.resolve.as_ref().map(|e| e.top)
    }

    fn resolve_close(&mut self) -> Option<ResolveEdit> {
        let ed = self.resolve.take();
        if self.mode == BufferMode::Resolve {
            self.kill_buffer();
        }
        ed
    }

    /// C-c C-c: write the resolved file and stage it.  Undecided conflicts
    /// keep their markers, so git still considers the file unresolved unless
    /// the user decided them all.
    fn resolve_finish(&mut self) -> Result<()> {
        let Some(ed) = self.resolve_close() else { return Ok(()) };
        let bytes = ed.file.resolved();
        let path = ed.file.path.clone();
        let abs = self.git.toplevel.join(&path);
        std::fs::write(&abs, &bytes).map_err(|e| anyhow::anyhow!("Cannot write {path}: {e}"))?;
        let remaining = ed.file.regions.iter().filter(|r| r.choice == Choice::Undecided).count();
        if remaining == 0 {
            self.git.stage(&path)?;
            self.set_msg(&format!("Resolved and staged {path} (r r to continue)"));
        } else {
            self.set_msg(&format!("Wrote {path}: {remaining} conflict(s) still undecided"));
        }
        self.refresh();
        Ok(())
    }

    fn resolve_cancel(&mut self) -> Result<()> {
        let modified = self.resolve.as_ref().is_some_and(|e| e.modified);
        if modified && !self.confirm_noabort("abort-resolve", "Discard conflict choices")? {
            return Ok(());
        }
        self.resolve_close();
        self.set_msg("Conflict resolution cancelled");
        self.refresh();
        Ok(())
    }

    /// A key in the resolution view. True if handled.
    pub(super) fn resolve_key(&mut self, key: &Key) -> Result<bool> {
        if self.mode != BufferMode::Resolve || self.view_text.is_some() || self.resolve.is_none() {
            return Ok(false);
        }
        if self.cx_prefix || (key.ctrl && key.code == "x") {
            return Ok(false);
        }
        if std::mem::take(&mut self.resolve.as_mut().unwrap().cc) {
            match (key.ctrl, key.code.as_str()) {
                (true, "c") => self.resolve_finish()?,
                (true, "k") => self.resolve_cancel()?,
                _ => self.set_msg(&format!("C-c {} is undefined", key.label())),
            }
            return Ok(true);
        }
        match (key.ctrl, key.code.as_str()) {
            (true, "c") => {
                self.resolve.as_mut().unwrap().cc = true;
                self.set_msg("C-c-");
            }
            // movement between conflicts (scrolls the new block into view)
            (false, "n") | (true, "n") => self.resolve_move(1),
            (false, "p") | (true, "p") => self.resolve_move(-1),
            // free vertical scrolling of the whole page
            (false, " ") => self.resolve_scroll(self.view_body_h(), true),
            (false, "backspace" | "DEL") => self.resolve_scroll(self.view_body_h(), false),
            (false, "pgdn") | (true, "v") => self.resolve_scroll(self.view_body_h(), true),
            (false, "pgup") | (false, "M-v") => self.resolve_scroll(self.view_body_h(), false),
            (false, "down") => self.resolve_scroll(1, true),
            (false, "up") => self.resolve_scroll(1, false),
            // decisions
            (false, "a") => self.resolve_set(Choice::A),
            (false, "b") => self.resolve_set(Choice::B),
            (false, "d") => self.resolve_set(Choice::Both),
            (false, "s") => self.resolve_set(Choice::Base),
            (false, "u") => self.resolve_set(Choice::Undecided),
            (false, "?") | (true, "h") => {
                self.view_text = Some(("Resolve conflicts".into(), HELP.into()));
                self.view_scroll = 0;
            }
            (false, "q") => self.set_msg("q is undefined (C-c C-k cancels, C-x C-c quits)"),
            _ => self.set_msg(&format!("{} is undefined", key.label())),
        }
        Ok(true)
    }

    /// Number of body rows available for scrolling a page.
    fn view_body_h(&self) -> isize {
        let (_, h) = crossterm::terminal::size().unwrap_or((80, 24));
        (h as isize).saturating_sub(3).max(1)
    }

    /// Free vertical scroll by DELTA display rows (down when DOWN).
    fn resolve_scroll(&mut self, delta: isize, down: bool) {
        let Some(ed) = self.resolve.as_mut() else { return };
        let total = Self::resolve_display_len(&ed.file) as isize;
        let d = if down { delta } else { -delta };
        ed.top = (ed.top as isize + d).clamp(0, (total - 1).max(0)) as usize;
    }

    /// Total number of display rows of the whole view.
    fn resolve_display_len(file: &ResolveFile) -> usize {
        // 1 header + 1 column header per conflict, plus its aligned rows,
        // plus a blank separator between conflicts
        file.aligned().iter().map(|a| 2 + a.rows.len().max(1) + 1).sum()
    }

    /// The display-row index of a conflict's header.
    fn resolve_conflict_row(ed: &ResolveEdit, idx: usize) -> usize {
        let aligned = ed.file.aligned();
        aligned.iter().take(idx).map(|a| 2 + a.rows.len().max(1) + 1).sum()
    }

    fn resolve_move(&mut self, d: isize) {
        let Some(ed) = self.resolve.as_mut() else { return };
        let n = ed.file.regions.len() as isize;
        ed.cursor = (ed.cursor as isize + d).clamp(0, n - 1).max(0) as usize;
        let row = Self::resolve_conflict_row(ed, ed.cursor);
        ed.top = row;
    }

    fn resolve_set(&mut self, choice: Choice) {
        let Some(ed) = self.resolve.as_mut() else { return };
        if choice == Choice::Base && ed.file.regions[ed.cursor].base.is_none() {
            self.set_msg("No base version (not a diff3 conflict)");
            return;
        }
        ed.file.regions[ed.cursor].choice = choice;
        ed.modified = true;
        // auto-advance to the next undecided conflict and scroll it into view
        let n = ed.file.regions.len();
        let from = ed.cursor;
        for step in 1..=n {
            let i = (from + step) % n;
            if ed.file.regions[i].choice == Choice::Undecided {
                ed.cursor = i;
                ed.top = Self::resolve_conflict_row(ed, i);
                break;
            }
        }
        self.set_msg(&format!("Conflict {}: {}", from + 1, choice.label()));
    }

    /// The rows of the resolution view: each conflict is a header, then a
    /// column header, then its A/B lines shown side by side.  Lines with no
    /// counterpart on the other side are highlighted as a diff; the whole
    /// page scrolls freely via `ed.top`.  W is the terminal width.
    pub(super) fn resolve_rows(&self, body: usize, w: usize) -> (Vec<Row>, String) {
        let t = &self.theme;
        let Some(ed) = &self.resolve else { return (Vec::new(), String::new()) };
        let col = (w.saturating_sub(3)) / 2; // two columns plus " | "
        let col = col.max(8);
        let aligned = ed.file.aligned();
        let fit = |s: &str| -> String {
            let tw = crate::ui::width::str_width(s);
            if tw <= col {
                format!("{s}{}", " ".repeat(col - tw))
            } else {
                let cut = crate::ui::width::truncate(s, col.saturating_sub(1)).to_string();
                let cw = crate::ui::width::str_width(&cut);
                format!("{cut}…{}", " ".repeat(col.saturating_sub(cw + 1)))
            }
        };
        let mut rows: Vec<Row> = Vec::new();
        for (i, (r, al)) in ed.file.regions.iter().zip(aligned.iter()).enumerate() {
            let current = i == ed.cursor;
            let head_base = if current { t.style(Face::CursorLine) } else { Style::default() };
            let on = |f: Face| {
                let s = t.style(f).over(head_base);
                Style { bg: head_base.bg.or(s.bg), ..s }
            };
            // conflict header with its decision
            let gutter = if current { t.glyphs.cursor } else { ' ' };
            let mut head = Row::new(head_base);
            head.push(format!("{gutter} "), on(Face::Gutter));
            head.push(format!("Conflict {} ", i + 1), on(Face::Label));
            let decided = r.choice != Choice::Undecided;
            head.push(format!("[{}]", r.choice.label()), on(if decided { Face::Keyword } else { Face::Dim }));
            rows.push(head);
            // column titles
            let mut titles = Row::new(t.style(Face::SectionHeading));
            titles.push(format!("  {}", fit("A (ours/upper)")), t.style(Face::SectionHeading));
            titles.push(" │ ", t.style(Face::Gutter));
            titles.push(fit("B (theirs/lower)"), t.style(Face::SectionHeading));
            rows.push(titles);
            // the aligned A/B lines side by side
            let pairs = if al.rows.is_empty() {
                vec![(None, None, false)]
            } else {
                al.rows.clone()
            };
            for (a, b, changed) in pairs {
                let mut row = Row::new(Style::default());
                let a_text = a.as_deref().unwrap_or("");
                let b_text = b.as_deref().unwrap_or("");
                let a_face = if changed && a.is_some() { Face::Added } else if a.is_none() { Face::Dim } else { Face::Context };
                let b_face = if changed && b.is_some() { Face::Removed } else if b.is_none() { Face::Dim } else { Face::Context };
                row.push("  ", Style::default());
                row.push(fit(a_text), t.style(a_face));
                row.push(" │ ", t.style(Face::Gutter));
                row.push(fit(b_text), t.style(b_face));
                rows.push(row);
            }
            // blank separator between conflicts
            rows.push(Row::text(String::new(), Style::default()));
        }
        let total = rows.len();
        let top = ed.top.min(total.saturating_sub(1));
        let visible: Vec<Row> = rows.into_iter().skip(top).take(body).collect();
        let n = ed.file.regions.len();
        (visible, format!("{}/{}", (ed.cursor + 1).min(n), n))
    }
}
