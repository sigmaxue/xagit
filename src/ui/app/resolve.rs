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
n / p    move to the next / previous conflict (also C-n / C-p, arrows)
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
        self.resolve = Some(ResolveEdit { file: parsed, cursor: 0, cc: false, modified: false });
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
            // movement between conflicts
            (false, "n" | "down") | (true, "n") => self.resolve_move(1),
            (false, "p" | "up") | (true, "p") => self.resolve_move(-1),
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

    fn resolve_move(&mut self, d: isize) {
        let Some(ed) = self.resolve.as_mut() else { return };
        let n = ed.file.regions.len() as isize;
        ed.cursor = (ed.cursor as isize + d).clamp(0, n - 1).max(0) as usize;
    }

    fn resolve_set(&mut self, choice: Choice) {
        let Some(ed) = self.resolve.as_mut() else { return };
        if choice == Choice::Base && ed.file.regions[ed.cursor].base.is_none() {
            self.set_msg("No base version (not a diff3 conflict)");
            return;
        }
        ed.file.regions[ed.cursor].choice = choice;
        ed.modified = true;
        // auto-advance to the next undecided conflict
        let n = ed.file.regions.len();
        let from = ed.cursor;
        for step in 1..=n {
            let i = (from + step) % n;
            if ed.file.regions[i].choice == Choice::Undecided {
                ed.cursor = i;
                break;
            }
        }
        self.set_msg(&format!("Conflict {}: {}", from + 1, choice.label()));
    }

    /// The rows of the resolution view.  Each conflict is a header (with
    /// its decision) followed by the A and B versions, labelled.
    pub(super) fn resolve_rows(&self, body: usize, scroll: &mut usize) -> (Vec<Row>, String) {
        let t = &self.theme;
        let Some(ed) = &self.resolve else { return (Vec::new(), String::new()) };
        let mut rows: Vec<(bool, Row)> = Vec::new(); // (is the cursor's header, row)
        for (i, r) in ed.file.regions.iter().enumerate() {
            let current = i == ed.cursor;
            let head_base = if current { t.style(Face::CursorLine) } else { Style::default() };
            let on = |f: Face| {
                let s = t.style(f).over(head_base);
                Style { bg: head_base.bg.or(s.bg), ..s }
            };
            let gutter = if current { t.glyphs.cursor } else { ' ' };
            let mut head = Row::new(head_base);
            head.push(format!("{gutter} "), on(Face::Gutter));
            head.push(format!("Conflict {} ", i + 1), on(Face::Label));
            let decided = r.choice != Choice::Undecided;
            head.push(format!("[{}]", r.choice.label()), on(if decided { Face::Keyword } else { Face::Dim }));
            rows.push((current, head));
            // the two versions, prefixed to distinguish them
            let side = |label: &str, bytes: &[u8], face: Face| -> Vec<Row> {
                let text = String::from_utf8_lossy(bytes);
                let mut out = Vec::new();
                let lines: Vec<&str> = text.split('\n').collect();
                let shown = if lines.last() == Some(&"") { &lines[..lines.len() - 1] } else { &lines[..] };
                for (n, l) in shown.iter().enumerate() {
                    let mut r = Row::new(Style::default());
                    let tag = if n == 0 { label } else { "  " };
                    r.push(format!("  {tag} "), t.style(Face::Gutter));
                    r.push(l.to_string(), t.style(face));
                    out.push(r);
                }
                if shown.is_empty() {
                    let mut r = Row::new(Style::default());
                    r.push(format!("  {label} "), t.style(Face::Gutter));
                    r.push("(empty)", t.style(Face::Dim));
                    out.push(r);
                }
                out
            };
            for r in side("A", &r.upper, Face::Added) {
                rows.push((false, r));
            }
            for r in side("B", &r.lower, Face::Removed) {
                rows.push((false, r));
            }
            let mut sep = Row::new(Style::default());
            sep.push(String::new(), Style::default());
            rows.push((false, sep));
        }
        // keep the current conflict's header visible
        let cursor_row = rows.iter().position(|(c, _)| *c).unwrap_or(0);
        if cursor_row < *scroll {
            *scroll = cursor_row;
        } else if cursor_row >= *scroll + body {
            *scroll = cursor_row + 1 - body;
        }
        let visible: Vec<Row> = rows.into_iter().skip(*scroll).take(body).map(|(_, r)| r).collect();
        let n = ed.file.regions.len();
        (visible, format!("{}/{}", (ed.cursor + 1).min(n), n))
    }
}
