//! The built-in conflict resolution view (an xagit addition): the A/B/both
//! part of Emacs Ediff's three-way merge session.  `e` (ediff dwim) opens it
//! when there are unmerged conflicts; the WHOLE file is shown — unchanged
//! lines as context in both columns, each conflict with its A (ours) and B
//! (theirs) versions side by side.  a/b/d/u decide the whole conflict; `*`
//! refines it so a/b/d decide ONE side-by-side line at a time (Ediff's
//! refine), with n/p moving between lines.  SPC/DEL and arrows scroll the
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
    /// `*` refine: a/b/d decide the current LINE instead of the conflict
    pub refine: bool,
    /// index of the current side-by-side row inside the current conflict
    pub row: usize,
    /// first display row shown (free vertical scrolling)
    pub top: usize,
    /// C-c was typed
    pub cc: bool,
    /// any decision changed
    pub modified: bool,
    /// the names of the two versions: A (upper, the HEAD side) and B
    /// (lower, the side being merged / picked / rebased)
    pub a_name: String,
    pub b_name: String,
}

const HELP: &str = "\
Resolve conflicts — Ediff merge-session keys (smerge-ediff / magit-ediff-resolve-rest)

*        refine: toggle LINE-wise decisions (one choice per side-by-side row)
n / p    next / previous difference; with refine, next / previous LINE
j        jump to difference N (with a numeric prefix, e.g. C-u 3 j); else the first
a        copy A (upper, the HEAD side) into the merge result
b        copy B (lower, the side merged / picked / rebased) into the result
+        combine A and B (ediff-combine-diffs; d is an alias)
r        restore the difference: leave its conflict markers (u is an alias)
With refine, a/b/+/r decide the current line only and the gutter between
the columns shows A / B / + / ? for each line. A partly decided conflict
keeps its markers until every line has a decision.
B        keep the base version (diff3 conflicts only; whole conflict)
!        recompute the differences from the file on disk
The bottom window (C) shows the merge result as it will be saved: the
current difference is highlighted, undecided ones still show their markers.

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
        let mut parsed = ResolveFile::parse(&file, &text);
        if parsed.regions.is_empty() {
            self.set_msg(&format!("No conflict markers in {file} (use E t for git mergetool)"));
            return Ok(false);
        }
        let (mut a_name, mut b_name) = self.resolve_side_names(&parsed);
        // During a rebase, git writes the onto (target) branch as the upper
        // side of the conflict.  Show the target on the RIGHT, like Ediff's
        // "B = the newer version": swap the columns and their names once.
        if self.git.rebase_in_progress() {
            parsed.swap_sides();
            std::mem::swap(&mut a_name, &mut b_name);
        }
        self.resolve = Some(ResolveEdit {
            file: parsed,
            cursor: 0,
            row: 0,
            refine: false,
            top: 0,
            cc: false,
            modified: false,
            a_name,
            b_name,
        });
        self.setup_buffer(BufferMode::Resolve, Vec::new());
        self.set_msg("Type ? for help (n/p move, a/b/+ choose, * for line-wise, q to finish)");
        Ok(true)
    }

    /// The names of the two sides of the conflicts, from the operation in
    /// progress (the conflict marker labels are the fallback):
    /// rebase: A = the branch rebased onto, B = the branch being rebased;
    /// merge: A = the current branch, B = the merged branch;
    /// cherry-pick / revert: A = the current branch, B = the commit.
    fn resolve_side_names(&self, file: &ResolveFile) -> (String, String) {
        use crate::ui::view::{file_line, shortname};
        let g = &self.git;
        let abbrev = |rev: &str| -> String {
            g.git_string(&["log", "-1", "--no-walk", "--format=%h %s", rev, "--"])
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| g.rev_short(rev))
        };
        let head_branch = || g.current_branch().unwrap_or_else(|| "HEAD".to_string());
        let marker = |upper: bool| {
            file.regions
                .first()
                .map(|r| if upper { r.upper_label.clone() } else { r.lower_label.clone() })
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| (if upper { "A" } else { "B" }).to_string())
        };
        if g.rebase_in_progress() {
            let dir = if g.gitdir.join("rebase-merge").is_dir() { "rebase-merge" } else { "rebase-apply" };
            let onto = file_line(g, &format!("{dir}/onto")).map(|o| shortname(g, &o)).unwrap_or_else(|| marker(true));
            let branch = file_line(g, &format!("{dir}/head-name"))
                .map(|h| h.strip_prefix("refs/heads/").unwrap_or(&h).to_string())
                .unwrap_or_else(|| "HEAD".to_string());
            let stop = file_line(g, "rebase-merge/stopped-sha")
                .or_else(|| file_line(g, "rebase-apply/original-commit"))
                .or_else(|| file_line(g, "REBASE_HEAD"));
            let b = match stop {
                Some(s) => format!("{branch} ({})", abbrev(&s)),
                None => branch,
            };
            return (onto, b);
        }
        if g.merge_in_progress() {
            let b = file_line(g, "MERGE_HEAD").map(|h| shortname(g, &h)).unwrap_or_else(|| marker(false));
            return (head_branch(), b);
        }
        if g.cherry_in_progress() {
            let b = file_line(g, "CHERRY_PICK_HEAD").map(|h| abbrev(&h)).unwrap_or_else(|| marker(false));
            return (head_branch(), b);
        }
        if g.revert_in_progress() {
            let b = file_line(g, "REVERT_HEAD").map(|h| format!("revert {}", abbrev(&h))).unwrap_or_else(|| marker(false));
            return (head_branch(), b);
        }
        (marker(true), marker(false))
    }

    /// The names of the two sides (tests).
    pub fn resolve_side_names_shown(&self) -> Option<(String, String)> {
        self.resolve.as_ref().map(|e| (e.a_name.clone(), e.b_name.clone()))
    }

    /// The decisions of the conflicts being edited (tests).
    pub fn resolve_choices(&self) -> Option<Vec<Choice>> {
        self.resolve.as_ref().map(|e| e.file.regions.iter().map(|r| r.choice).collect())
    }

    pub fn resolve_cursor(&self) -> Option<usize> {
        self.resolve.as_ref().map(|e| e.cursor)
    }

    /// Whether line-wise decisions are active (tests).
    pub fn resolve_refine(&self) -> Option<bool> {
        self.resolve.as_ref().map(|e| e.refine)
    }

    /// The current side-by-side row inside the current conflict (tests).
    pub fn resolve_row(&self) -> Option<usize> {
        self.resolve.as_ref().map(|e| e.row)
    }

    /// The per-line choices of the current conflict (tests).
    pub fn resolve_line_choices(&self) -> Option<Vec<Choice>> {
        self.resolve.as_ref().map(|e| e.file.regions[e.cursor].line_choices())
    }

    /// The number of side-by-side rows of the current conflict (tests).
    pub fn resolve_line_count(&self) -> Option<usize> {
        self.resolve.as_ref().map(|e| e.file.regions[e.cursor].line_count())
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
            let remaining: usize = ed.file.regions.iter().map(|r| r.unresolved_units()).sum();
            self.set_msg(&format!("Wrote {path}: {remaining} line(s)/conflict(s) still unresolved"));
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
        let mut parsed = ResolveFile::parse(&path, &text);
        if parsed.regions.is_empty() {
            self.set_msg(&format!("No conflict markers left in {path}"));
            return Ok(());
        }
        // keep the rebase column order (target on the right) on a recompute
        if self.git.rebase_in_progress() {
            parsed.swap_sides();
        }
        let n = parsed.regions.len();
        let ed = self.resolve.as_mut().unwrap();
        ed.file = parsed;
        ed.cursor = ed.cursor.min(n - 1);
        ed.row = 0;
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
            // * (ediff-toggle-refine): decide one line at a time
            (false, "*") => self.resolve_toggle_refine(),
            // decisions (ediff-merge keys): the current line when refined
            (false, "a") => self.resolve_choose(Choice::A),
            (false, "b") => self.resolve_choose(Choice::B),
            (false, "+" | "d") => self.resolve_choose(Choice::Both),
            (false, "B") => self.resolve_set(Choice::Base),
            (false, "r" | "u") => self.resolve_choose(Choice::Undecided),
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
        let body = (h as usize).saturating_sub(3).max(1);
        Self::resolve_split(body).0.max(1) as isize
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

    /// The display-row index of line ROW of conflict IDX.
    fn resolve_line_row(ed: &ResolveEdit, idx: usize, row: usize) -> usize {
        use crate::ui::resolve::ViewLine;
        let mut seen = 0usize;
        for (i, vl) in ed.file.view().iter().enumerate() {
            if let ViewLine::Pair(id, ..) = vl {
                if *id == idx {
                    if seen == row {
                        return i;
                    }
                    seen += 1;
                }
            }
        }
        Self::resolve_conflict_row(ed, idx)
    }

    /// n/p: the next conflict, or the next line when refined.
    fn resolve_move(&mut self, d: isize) {
        if self.resolve.as_ref().is_some_and(|e| e.refine) {
            self.resolve_move_line(d);
            return;
        }
        let Some(ed) = self.resolve.as_mut() else { return };
        let n = ed.file.regions.len() as isize;
        ed.cursor = (ed.cursor as isize + d).clamp(0, n - 1).max(0) as usize;
        let row = Self::resolve_conflict_row(ed, ed.cursor);
        ed.top = row;
    }

    /// Move between side-by-side rows, crossing conflict boundaries.
    fn resolve_move_line(&mut self, d: isize) {
        let Some(ed) = self.resolve.as_mut() else { return };
        let count = ed.file.regions.len();
        let mut cur = ed.cursor as isize;
        let mut row = ed.row as isize + d;
        loop {
            let n = ed.file.regions[cur as usize].line_count() as isize;
            if row < 0 {
                if cur > 0 {
                    cur -= 1;
                    row = ed.file.regions[cur as usize].line_count() as isize - 1;
                    continue;
                }
                row = 0;
            } else if row >= n {
                if cur + 1 < count as isize {
                    cur += 1;
                    row = 0;
                    continue;
                }
                row = (n - 1).max(0);
            }
            break;
        }
        ed.cursor = cur as usize;
        ed.row = row.max(0) as usize;
        let (cursor, line) = (ed.cursor, ed.row);
        let top = Self::resolve_line_row(ed, cursor, line);
        ed.top = top;
    }

    /// `*`: toggle Ediff's refine (one decision per side-by-side row).
    fn resolve_toggle_refine(&mut self) {
        let refine = {
            let Some(ed) = self.resolve.as_mut() else { return };
            ed.refine = !ed.refine;
            ed.row = 0;
            let (cursor, row) = (ed.cursor, ed.row);
            let top = if ed.refine {
                Self::resolve_line_row(ed, cursor, row)
            } else {
                Self::resolve_conflict_row(ed, cursor)
            };
            ed.top = top;
            ed.refine
        };
        self.set_msg(if refine {
            "Line-wise: a/b/d choose this line (n/p next line, * whole conflict)"
        } else {
            "Whole conflict: a/b/d choose the block (* for line-wise)"
        });
    }

    /// a/b/d/r: the current line when refined, else the whole conflict.
    fn resolve_choose(&mut self, choice: Choice) {
        let refine = self.resolve.as_ref().is_some_and(|e| e.refine);
        if refine {
            self.resolve_set_line(choice);
        } else {
            self.resolve_set(choice);
        }
    }

    /// Decide the current side-by-side row.
    fn resolve_set_line(&mut self, choice: Choice) {
        let msg = {
            let Some(ed) = self.resolve.as_mut() else { return };
            if choice == Choice::Base {
                "Base is a whole-conflict choice".to_string()
            } else {
                let (idx, row) = (ed.cursor, ed.row);
                if ed.file.regions[idx].set_line(row, choice) {
                    ed.modified = true;
                    let text = Self::choice_text(ed, choice);
                    format!("Conflict {} line {}: {text}", idx + 1, row + 1)
                } else {
                    "No line at point".to_string()
                }
            }
        };
        self.set_msg(&msg);
    }

    fn resolve_set(&mut self, choice: Choice) {
        let Some(ed) = self.resolve.as_mut() else { return };
        if choice == Choice::Base && ed.file.regions[ed.cursor].base.is_none() {
            self.set_msg("No base version (not a diff3 conflict)");
            return;
        }
        // like ediff-copy-A/B-to-C and ediff-combine-diffs, point stays on
        // the current difference (n moves on)
        if choice == Choice::Undecided {
            ed.file.regions[ed.cursor].lines.clear();
        }
        ed.file.regions[ed.cursor].choice = choice;
        ed.modified = true;
        let n = ed.cursor + 1;
        let text = Self::choice_text(ed, choice);
        self.set_msg(&format!("Difference {n}: {text}"));
    }

    /// How a decision is shown, with the names of the sides.
    fn choice_text(ed: &ResolveEdit, choice: Choice) -> String {
        match choice {
            Choice::A => format!("A: {}", ed.a_name),
            Choice::B => format!("B: {}", ed.b_name),
            Choice::Both => format!("A+B: {} + {}", ed.a_name, ed.b_name),
            Choice::Base => "base".into(),
            Choice::Undecided => "undecided".into(),
        }
    }

    /// The heights of the A/B window and of the merge-result window (C)
    /// for a body of BODY rows (one row is the C title).
    fn resolve_split(body: usize) -> (usize, usize) {
        if body < 8 {
            return (body, 0);
        }
        let top = (body - 1) * 3 / 5;
        (top, body - 1 - top)
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
        let columns = |a: &str, b: &str, a_style: Style, b_style: Style, sep: String, base: Style| -> Row {
            let mut row = Row::new(base);
            row.push("  ", base);
            row.push(fit(a), a_style);
            row.push(sep, t.style(Face::Gutter).over(base));
            row.push(fit(b), b_style);
            row
        };
        let mut rows: Vec<Row> = Vec::new();
        let mut pair_row = 0usize;
        for vl in ed.file.view() {
            match vl {
                ViewLine::Context(s) => {
                    // the same text in both columns; no diff highlight
                    rows.push(columns(
                        &s,
                        &s,
                        t.style(Face::Context),
                        t.style(Face::Context),
                        " │ ".to_string(),
                        Style::default(),
                    ));
                }
                ViewLine::Header(i) => {
                    pair_row = 0;
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
                    let r = &ed.file.regions[i];
                    let resolved = r.unresolved_units() == 0;
                    let label = if ed.refine {
                        let n = r.line_count();
                        let done = if r.refined() { n - r.undecided_lines() } else { 0 };
                        format!("[line {}/{} · {}/{} decided]", ed.row + 1, n, done, n)
                    } else {
                        format!("[{}]", Self::choice_text(ed, r.choice))
                    };
                    head.push(label, on(if resolved { Face::Keyword } else { Face::Dim }));
                    rows.push(head);
                }
                ViewLine::Columns => {
                    let mut titles = Row::new(t.style(Face::SectionHeading));
                    titles.push(format!("  {}", fit(&format!("A: {}", ed.a_name))), t.style(Face::SectionHeading));
                    titles.push(" │ ", t.style(Face::Gutter));
                    titles.push(fit(&format!("B: {}", ed.b_name)), t.style(Face::SectionHeading));
                    rows.push(titles);
                }
                ViewLine::Pair(idx, a, b, changed) => {
                    let a_text = a.as_deref().unwrap_or("");
                    let b_text = b.as_deref().unwrap_or("");
                    let a_face = if changed && a.is_some() { Face::Added } else if a.is_none() { Face::Dim } else { Face::Context };
                    let b_face = if changed && b.is_some() { Face::Removed } else if b.is_none() { Face::Dim } else { Face::Context };
                    let current = ed.refine && idx == ed.cursor && pair_row == ed.row;
                    let (sep, base, a_style, b_style) = if ed.refine {
                        let c = ed.file.regions[idx].line_choices().get(pair_row).copied().unwrap_or(Choice::Undecided);
                        let ch = match c {
                            Choice::A => 'A',
                            Choice::B => 'B',
                            Choice::Both => '+',
                            Choice::Base => 'b',
                            Choice::Undecided => '?',
                        };
                        let base = if current { t.style(Face::CursorLine) } else { Style::default() };
                        let (a_style, b_style) = if current {
                            (t.style(a_face).over(base), t.style(b_face).over(base))
                        } else {
                            (t.style(a_face), t.style(b_face))
                        };
                        (format!(" {ch} "), base, a_style, b_style)
                    } else {
                        (" │ ".to_string(), Style::default(), t.style(a_face), t.style(b_face))
                    };
                    rows.push(columns(a_text, b_text, a_style, b_style, sep, base));
                    pair_row += 1;
                }
                ViewLine::Blank => rows.push(Row::text(String::new(), Style::default())),
            }
        }
        let (top_h, c_h) = Self::resolve_split(body);
        let total = rows.len();
        let top = ed.top.min(total.saturating_sub(1));
        let mut visible: Vec<Row> = rows.into_iter().skip(top).take(top_h).collect();
        if c_h > 0 {
            while visible.len() < top_h {
                visible.push(Row::text(String::new(), Style::default()));
            }
            // the C window: the merge result as it would be saved
            let mut title = Row::new(t.style(Face::HunkHeading));
            title.push(format!("  C: merge result — {} ", ed.file.path), t.style(Face::HunkHeading));
            let used = crate::ui::width::str_width(&format!("  C: merge result — {} ", ed.file.path));
            title.push(t.glyphs.rule.to_string().repeat(w.saturating_sub(used).max(1)), t.style(Face::Gutter));
            visible.push(title);
            let result = ed.file.result_lines();
            // keep the current difference near the top of the C window
            let first = result.iter().position(|(_, c, _)| *c == Some(ed.cursor)).unwrap_or(0);
            let c_top = first.saturating_sub(2).min(result.len().saturating_sub(c_h));
            let cw = w.saturating_sub(2);
            for (text, conflict, empty) in result.into_iter().skip(c_top).take(c_h) {
                let current = conflict == Some(ed.cursor);
                let base = if current { t.style(Face::CursorLine) } else { Style::default() };
                let undecided = conflict.is_some_and(|i| ed.file.regions[i].choice == Choice::Undecided);
                let face = if empty {
                    Face::Dim
                } else if undecided {
                    Face::Conflict
                } else if conflict.is_some() {
                    Face::Keyword
                } else {
                    Face::Context
                };
                let shown = if empty { "(empty)".to_string() } else { text };
                let shown = if crate::ui::width::str_width(&shown) > cw {
                    format!("{}…", crate::ui::width::truncate(&shown, cw.saturating_sub(1)))
                } else {
                    shown
                };
                let mut row = Row::new(base);
                let gutter = if conflict.is_some() { t.glyphs.region } else { ' ' };
                row.push(format!("{gutter} "), t.style(Face::Gutter).over(base));
                row.push(shown, t.style(face).over(base));
                visible.push(row);
            }
        }
        let n = ed.file.regions.len();
        (visible, format!("{}/{}", (ed.cursor + 1).min(n), n))
    }
}
