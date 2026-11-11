//! The built-in conflict resolution view (an xagit addition): the A/B/both
//! part of Emacs Ediff's three-way merge session.  `e` (ediff dwim) opens it
//! when there are unmerged conflicts; the WHOLE file is shown — unchanged
//! lines as context in both columns, each conflict with its A (ours) and B
//! (theirs) versions side by side and a per-region decision.  a/b/d/u set
//! the decision, n/p jump between conflicts, SPC/DEL and arrows scroll the
//! page, C-c C-c writes the file back and stages it, C-c C-k cancels
//! without touching the file.

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
Resolve conflicts — Ediff merge-session keys (smerge-ediff / magit-ediff-resolve-rest)

n / p    next / previous difference (scrolls it into view)
j        jump to difference N (with a numeric prefix, e.g. C-u 3 j); else the first
a        copy A (ours / upper) into the merge result
b        copy B (theirs / lower) into the merge result
+        combine A and B (ediff-combine-diffs; d is an alias)
r        restore the difference: leave its conflict markers (u is an alias)
B        keep the base version (diff3 conflicts only; xagit extension)
!        recompute the differences from the file on disk
SPC/DEL  page down / up (also C-v / M-v, PgDn / PgUp)
up/down  scroll one line
q        quit: \"Conflict resolution finished; save FILE?\" (yes saves and,
         when no conflict markers remain, stages the file)
C-c C-c  save without asking; C-c C-k cancel without saving
?        show this help

Not supported in this single view: * (refine), ## / #h / #f (skip regions),
| (toggle split), $$ (show only conflicts), window and buffer commands.";

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
        self.resolve_open_file(&file)
    }

    /// The resolution view of the unmerged FILE (also `E m` / `E M`).
    pub(super) fn resolve_open_file(&mut self, file: &str) -> Result<bool> {
        let file = file.to_string();
        let abs = self.git.toplevel.join(&file);
        let text = std::fs::read(&abs).map_err(|e| anyhow::anyhow!("Cannot read {file}: {e}"))?;
        let parsed = ResolveFile::parse(&file, &text);
        if parsed.regions.is_empty() {
            self.set_msg(&format!("No conflict markers in {file} (use E t for git mergetool)"));
            return Ok(false);
        }
        self.resolve = Some(ResolveEdit { file: parsed, cursor: 0, top: 0, cc: false, modified: false });
        self.setup_buffer(BufferMode::Resolve, Vec::new());
        self.set_msg("Type ? for help (n/p move, a/b/+ choose, q to finish)");
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

    /// Write the merge result back to the file and, like the quit hook of
    /// magit-ediff-resolve-all, stage it when no line starts with
    /// "<<<<<<< " any more.
    fn resolve_finish(&mut self) -> Result<()> {
        let Some(ed) = self.resolve_close() else { return Ok(()) };
        let bytes = ed.file.resolved();
        let path = ed.file.path.clone();
        let abs = self.git.toplevel.join(&path);
        std::fs::write(&abs, &bytes).map_err(|e| anyhow::anyhow!("Cannot write {path}: {e}"))?;
        let markers = bytes.split(|&b| b == b'\n').any(|l| l.starts_with(b"<<<<<<< "));
        if markers {
            let remaining = ed.file.regions.iter().filter(|r| r.choice == Choice::Undecided).count();
            self.set_msg(&format!("Wrote {path}: {remaining} conflict(s) still unresolved"));
        } else {
            self.git.stage(&path)?;
            self.set_msg(&format!("Resolved and staged {path} (r r to continue)"));
        }
        self.refresh();
        Ok(())
    }

    /// q (ediff-quit in a merge job started by smerge-ediff /
    /// magit-ediff-resolve-rest): "Conflict resolution finished; save
    /// FILE?" — yes writes the result back, no leaves the file alone.
    fn resolve_quit(&mut self) -> Result<()> {
        let path = self.resolve.as_ref().map(|e| e.file.path.clone()).unwrap_or_default();
        if self.yes_or_no_p(&format!("Conflict resolution finished; save {path}? "))? {
            self.resolve_finish()
        } else {
            self.resolve_close();
            self.set_msg(&format!("{path} not saved"));
            self.refresh();
            Ok(())
        }
    }

    /// ! (ediff-update-diffs): parse the file again from disk.
    fn resolve_reload(&mut self) -> Result<()> {
        let Some(ed) = self.resolve.as_ref() else { return Ok(()) };
        let path = ed.file.path.clone();
        let text = std::fs::read(self.git.toplevel.join(&path)).map_err(|e| anyhow::anyhow!("Cannot read {path}: {e}"))?;
        let parsed = ResolveFile::parse(&path, &text);
        if parsed.regions.is_empty() {
            self.set_msg(&format!("No conflict markers left in {path}"));
            return Ok(());
        }
        let n = parsed.regions.len();
        let ed = self.resolve.as_mut().unwrap();
        ed.file = parsed;
        ed.cursor = ed.cursor.min(n - 1);
        ed.top = Self::resolve_conflict_row(ed, ed.cursor);
        ed.modified = false;
        self.set_msg(&format!("{n} difference(s) recomputed"));
        Ok(())
    }

    /// j (ediff-jump-to-difference): to difference N given as a numeric
    /// prefix argument, else to the first one.
    fn resolve_jump(&mut self) {
        let n = self.prefix_arg.map(|p| p.numeric()).unwrap_or(1);
        let Some(ed) = self.resolve.as_mut() else { return };
        let count = ed.file.regions.len();
        if n < 1 || n as usize > count {
            self.set_msg(&format!("Bad difference number, {n}. Valid numbers are 1 to {count}"));
            return;
        }
        ed.cursor = n as usize - 1;
        ed.top = Self::resolve_conflict_row(ed, ed.cursor);
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
            (false, "j") => self.resolve_jump(),
            // decisions (ediff-merge keys)
            (false, "a") => self.resolve_set(Choice::A),
            (false, "b") => self.resolve_set(Choice::B),
            (false, "+" | "d") => self.resolve_set(Choice::Both),
            (false, "B") => self.resolve_set(Choice::Base),
            (false, "r" | "u") => self.resolve_set(Choice::Undecided),
            (false, "!") => self.resolve_reload()?,
            (false, "?") | (true, "h") => {
                self.view_text = Some(("Resolve conflicts".into(), HELP.into()));
                self.view_scroll = 0;
            }
            (false, "q") => self.resolve_quit()?,
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

    /// Total number of display rows of the whole view (including context).
    fn resolve_display_len(file: &ResolveFile) -> usize {
        file.view().len()
    }

    /// The display-row index of a conflict's header.
    fn resolve_conflict_row(ed: &ResolveEdit, idx: usize) -> usize {
        ed.file
            .view()
            .iter()
            .position(|l| *l == crate::ui::resolve::ViewLine::Header(idx))
            .unwrap_or(0)
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
        // like ediff-copy-A/B-to-C and ediff-combine-diffs, point stays on
        // the current difference (n moves on)
        ed.file.regions[ed.cursor].choice = choice;
        ed.modified = true;
        let n = ed.cursor + 1;
        self.set_msg(&format!("Difference {n}: {}", choice.label()));
    }

    /// The rows of the resolution view: the WHOLE file is shown.  Ordinary
    /// (non-conflict) lines appear in both columns alike as context; each
    /// conflict is a header, a column-title row, its A/B pairs (diff-highlit
    /// on changed lines) and a blank separator.  The page scrolls freely
    /// via `ed.top`.  W is the terminal width.
    pub(super) fn resolve_rows(&self, body: usize, w: usize) -> (Vec<Row>, String) {
        use crate::ui::resolve::ViewLine;
        let t = &self.theme;
        let Some(ed) = &self.resolve else { return (Vec::new(), String::new()) };
        let col = (w.saturating_sub(3)) / 2; // two columns plus " │ "
        let col = col.max(8);
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
        let columns = |a: &str, b: &str, a_face: Face, b_face: Face| -> Row {
            let mut row = Row::new(Style::default());
            row.push("  ", Style::default());
            row.push(fit(a), t.style(a_face));
            row.push(" │ ", t.style(Face::Gutter));
            row.push(fit(b), t.style(b_face));
            row
        };
        let mut rows: Vec<Row> = Vec::new();
        for vl in ed.file.view() {
            match vl {
                ViewLine::Context(s) => {
                    // the same text in both columns; no diff highlight
                    rows.push(columns(&s, &s, Face::Context, Face::Context));
                }
                ViewLine::Header(i) => {
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
                    let decided = ed.file.regions[i].choice != Choice::Undecided;
                    head.push(
                        format!("[{}]", ed.file.regions[i].choice.label()),
                        on(if decided { Face::Keyword } else { Face::Dim }),
                    );
                    rows.push(head);
                }
                ViewLine::Columns => {
                    let mut titles = Row::new(t.style(Face::SectionHeading));
                    titles.push(format!("  {}", fit("A (ours/upper)")), t.style(Face::SectionHeading));
                    titles.push(" │ ", t.style(Face::Gutter));
                    titles.push(fit("B (theirs/lower)"), t.style(Face::SectionHeading));
                    rows.push(titles);
                }
                ViewLine::Pair(_, a, b, changed) => {
                    let a_text = a.as_deref().unwrap_or("");
                    let b_text = b.as_deref().unwrap_or("");
                    let a_face = if changed && a.is_some() { Face::Added } else if a.is_none() { Face::Dim } else { Face::Context };
                    let b_face = if changed && b.is_some() { Face::Removed } else if b.is_none() { Face::Dim } else { Face::Context };
                    rows.push(columns(a_text, b_text, a_face, b_face));
                }
                ViewLine::Blank => rows.push(Row::text(String::new(), Style::default())),
            }
        }
        let total = rows.len();
        let top = ed.top.min(total.saturating_sub(1));
        let visible: Vec<Row> = rows.into_iter().skip(top).take(body).collect();
        let n = ed.file.regions.len();
        (visible, format!("{}/{}", (ed.cursor + 1).min(n), n))
    }
}
