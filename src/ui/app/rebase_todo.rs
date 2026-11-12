//! git-rebase-mode (git-rebase.el): the todo list of an interactive rebase
//! edited in xagit instead of an external editor.  `r i` (and `r e` while
//! rebasing) open it; C-c C-c tells git to make it happen, C-c C-k aborts.

use super::*;
use crate::ui::rebase_todo::{LineType, Todo};

/// The bindings of git-rebase-mode-map shown by `?`.
const HELP: &str = "\
git-rebase-mode — edit the todo list, then C-c C-c to rebase

C-c C-c  tell Git to make it happen
C-c C-k  tell Git that you changed your mind, i.e., abort
p / n    move point to previous / next line (also C-p / C-n, up / down)
M-p M-n  move the commit at point up / down (also M-up / M-down)
RET      show the commit at point
c        pick      use commit
r / w    reword    use commit, but edit the commit message
e / m    edit      use commit, but stop for amending
s        squash    use commit, but meld into previous commit
S        fixup -c  like squash, but keep only the current message
f        fixup     like squash, but discard this commit's message
F / A    fixup -C  like fixup, but keep this commit's message
d        drop      remove commit
k / C-k  un-/comment current line
x        exec      run command (the rest of the line) using shell
b        break     stop here (continue rebase later with 'r r')
z        noop      add noop action at point
l t u    label / reset / update-ref
y        insert a line for an arbitrary commit
C-/ C-_  undo last change
C-SPC    set mark (the commands act on all lines of the region)";

impl App {
    /// magit-rebase-interactive-1 for COMMIT: the todo git offers, edited
    /// in a git-rebase-mode buffer.
    pub(super) fn rebase_todo_start(&mut self, commit: &str, args: &[String]) -> Result<()> {
        let text = match self.git.rebase_todo_capture(commit, args) {
            Ok(t) => t,
            Err(e) => {
                self.set_msg(&format!("{e:#}"));
                self.refresh();
                return Ok(());
            }
        };
        let target = RebaseTodoTarget::Start { commit: commit.to_string(), args: args.to_vec() };
        self.rebase_todo_open(&text, target);
        Ok(())
    }

    /// magit-rebase-edit (`r e` while rebasing): the rest of the todo.
    pub(super) fn rebase_todo_edit_in_progress(&mut self) -> Result<()> {
        match self.git.rebase_todo_in_progress() {
            Ok(text) => self.rebase_todo_open(&text, RebaseTodoTarget::InProgress),
            Err(e) => self.set_msg(&format!("{e:#}")),
        }
        Ok(())
    }

    fn rebase_todo_open(&mut self, text: &str, target: RebaseTodoTarget) {
        let cc = self
            .git
            .git_string(&["config", "core.commentChar"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "auto")
            .unwrap_or_else(|| "#".into());
        let todo = Todo::new(text, &cc);
        self.rebase_todo = Some(RebaseTodoEdit { todo, cursor: 0, mark: None, target });
        self.setup_buffer(BufferMode::RebaseTodo, Vec::new());
        self.set_msg("Type C-c C-c to finish, C-c C-k to cancel, ? for help");
    }

    /// The lines of the todo being edited (tests).
    pub fn rebase_todo_lines(&self) -> Option<Vec<String>> {
        self.rebase_todo.as_ref().map(|e| e.todo.lines.clone())
    }

    /// The line point is on in the todo being edited (tests).
    pub fn rebase_todo_cursor(&self) -> Option<usize> {
        self.rebase_todo.as_ref().map(|e| e.cursor)
    }

    /// Leave the todo buffer: the buffer below is shown.
    fn rebase_todo_close(&mut self) -> Option<RebaseTodoEdit> {
        let ed = self.rebase_todo.take();
        if self.mode == BufferMode::RebaseTodo {
            self.kill_buffer();
        }
        ed
    }

    /// with-editor-finish: run git with the todo list.
    fn rebase_todo_finish(&mut self) -> Result<()> {
        let Some(ed) = self.rebase_todo_close() else { return Ok(()) };
        let text = ed.todo.text();
        let res = match &ed.target {
            // reword / squash / edit messages are written in xagit's text
            // buffer; the todo itself comes from this buffer
            RebaseTodoTarget::Start { commit, args } => {
                let (commit, args) = (commit.clone(), args.clone());
                let (todo, file) = self.git.todo_install_editor(&text)?;
                let a = self.git.rebase_i_args(&commit, &args, &[])?;
                return self.start_edit_git(a, Some(todo), EditDone::Rebase, Some(file));
            }
            RebaseTodoTarget::InProgress => self.git.rebase_edit_todo_with(&text),
        };
        self.after_rebase(res);
        Ok(())
    }

    /// with-editor-cancel (git-rebase-cancel-confirm when modified).
    fn rebase_todo_cancel(&mut self) -> Result<()> {
        let modified = self.rebase_todo.as_ref().is_some_and(|e| e.todo.modified);
        if modified && !self.confirm_noabort("abort-rebase", "Abort this rebase")? {
            return Ok(());
        }
        let Some(ed) = self.rebase_todo_close() else { return Ok(()) };
        match ed.target {
            // nothing was started: the capture left the repository alone
            RebaseTodoTarget::Start { .. } => self.set_msg("Rebase aborted"),
            // the todo left as it is; the rebase is still in progress
            RebaseTodoTarget::InProgress => self.set_msg("Todo list unchanged (r r continue, r a abort)"),
        }
        self.refresh();
        Ok(())
    }

    /// A key in the git-rebase-mode buffer. True if it was handled.
    pub(super) fn rebase_todo_key(&mut self, key: &Key) -> Result<bool> {
        if self.mode != BufferMode::RebaseTodo || self.view_text.is_some() || self.rebase_todo.is_none() {
            return Ok(false);
        }
        // the global C-x keys (C-x C-c quits) stay available
        if self.cx_prefix || (key.ctrl && key.code == "x") {
            return Ok(false);
        }
        if std::mem::take(&mut self.rebase_todo_cc) {
            match (key.ctrl, key.code.as_str()) {
                (true, "c") => self.rebase_todo_finish()?,
                (true, "k") => self.rebase_todo_cancel()?,
                _ => self.set_msg(&format!("C-c {} is undefined", key.label())),
            }
            return Ok(true);
        }
        let arg = self.prefix_arg.is_some();
        let ding = |app: &mut App| app.set_msg("");
        match (key.ctrl, key.code.as_str()) {
            (true, "c") => {
                self.rebase_todo_cc = true;
                self.set_msg("C-c-");
            }
            // movement
            (false, "p" | "up") | (true, "p") => self.rebase_todo_forward(-1),
            (false, "n" | "down") | (true, "n") => self.rebase_todo_forward(1),
            (false, "M-p" | "M-up") => self.rebase_todo_move(-1),
            (false, "M-n" | "M-down") => self.rebase_todo_move(1),
            (false, "<" | "home") | (false, "M-<") => self.rebase_todo_goto(0),
            (false, ">" | "end") | (false, "M->") => {
                let n = self.rebase_todo.as_ref().map_or(0, |e| e.todo.lines.len());
                self.rebase_todo_goto(n.saturating_sub(1));
            }
            (true, " ") => {
                if let Some(ed) = self.rebase_todo.as_mut() {
                    ed.mark = Some(ed.cursor);
                }
                self.set_msg("Mark set");
            }
            (true, "g") => {
                if let Some(ed) = self.rebase_todo.as_mut() {
                    ed.mark = None;
                }
                self.set_msg("Quit");
            }
            // actions of the commit lines
            (false, "c") => self.rebase_todo_set(Some("pick")),
            (false, "r" | "w") => self.rebase_todo_set(Some("reword")),
            (false, "e" | "m") => self.rebase_todo_set(Some("edit")),
            (false, "s") => self.rebase_todo_set(Some("squash")),
            (false, "S") => self.rebase_todo_set(Some("fixup -c")),
            (false, "f") => self.rebase_todo_set(Some("fixup")),
            (false, "F" | "A") => self.rebase_todo_set(Some("fixup -C")),
            (false, "d") => self.rebase_todo_set(Some("drop")),
            (false, "k") | (true, "k") => self.rebase_todo_set(None),
            // other lines
            (false, "b") => self.rebase_todo_bare("break", arg),
            (false, "z") => self.rebase_todo_bare("noop", arg),
            (false, "x") => self.rebase_todo_noncommit("exec", "Execute", arg)?,
            (false, "l") => self.rebase_todo_noncommit("label", "Label", arg)?,
            (false, "t") => self.rebase_todo_noncommit("reset", "Label", arg)?,
            (false, "u") => self.rebase_todo_noncommit("update-ref", "Ref", arg)?,
            (false, "y") => self.rebase_todo_insert()?,
            (true, "/" | "_" | "7") => {
                let undone = self.rebase_todo.as_mut().and_then(|e| e.todo.undo().map(|c| (e, c)).map(|(e, c)| {
                    e.cursor = c.min(e.todo.lines.len().saturating_sub(1));
                    e.mark = None;
                }));
                self.set_msg(if undone.is_some() { "Undo" } else { "No further undo information" });
            }
            // git-rebase-show-commit
            (false, "enter" | " ") => {
                let target = self.rebase_todo.as_ref().map(|e| e.todo.parsed(e.cursor));
                match target {
                    Some(p) if p.typ == Some(LineType::Commit) => {
                        let rev = p.target.clone();
                        self.setup_buffer(BufferMode::Revision { rev, files: Vec::new() }, self.diff_args.clone());
                    }
                    _ => ding(self),
                }
            }
            (false, "?") | (true, "h") => {
                self.view_text = Some(("git-rebase-mode".into(), HELP.into()));
                self.view_scroll = 0;
            }
            // special-mode's q is undefined in git-rebase-mode
            (false, "q") => self.set_msg("q is undefined (C-c C-k cancels the rebase)"),
            (false, "g") => {}
            _ => self.set_msg(&format!("{} is undefined", key.label())),
        }
        Ok(true)
    }

    fn rebase_todo_goto(&mut self, i: usize) {
        if let Some(ed) = self.rebase_todo.as_mut() {
            ed.cursor = i.min(ed.todo.lines.len().saturating_sub(1));
        }
    }

    fn rebase_todo_forward(&mut self, n: isize) {
        if let Some(ed) = self.rebase_todo.as_mut() {
            let last = ed.todo.lines.len().saturating_sub(1) as isize;
            ed.cursor = (ed.cursor as isize + n * self.prefix_arg.map_or(1, |p| p.numeric() as isize)).clamp(0, last) as usize;
        }
    }

    fn rebase_todo_set(&mut self, action: Option<&str>) {
        let Some(ed) = self.rebase_todo.as_mut() else { return };
        match ed.todo.set_action(ed.cursor, ed.mark, action) {
            Some(c) => {
                ed.cursor = c;
                ed.mark = None;
            }
            None => self.set_msg("Not on a rebase line"),
        }
    }

    fn rebase_todo_move(&mut self, dir: isize) {
        let n = dir * self.prefix_arg.map_or(1, |p| p.numeric() as isize);
        let Some(ed) = self.rebase_todo.as_mut() else { return };
        match ed.todo.move_lines(ed.cursor, ed.mark, n) {
            Some((c, m)) => {
                ed.cursor = c;
                ed.mark = m;
            }
            None => self.set_msg("Cannot move further"),
        }
    }

    fn rebase_todo_bare(&mut self, action: &str, arg: bool) {
        let Some(ed) = self.rebase_todo.as_mut() else { return };
        if let Some(c) = ed.todo.set_bare_action(ed.cursor, action, arg) {
            ed.cursor = c;
        }
    }

    /// git-rebase-exec / -label / -reset / -update-ref
    fn rebase_todo_noncommit(&mut self, action: &str, prompt: &str, arg: bool) -> Result<()> {
        let Some(initial) = self.rebase_todo.as_ref().map(|e| e.todo.noncommit_initial(e.cursor, action, arg)) else {
            return Ok(());
        };
        let Some(value) = self.read_line_raw(&format!("{prompt}: "), initial.as_deref())? else {
            return Ok(());
        };
        let Some(ed) = self.rebase_todo.as_mut() else { return Ok(()) };
        match ed.todo.set_noncommit_action(ed.cursor, action, value.trim(), initial.as_deref()) {
            Some(c) => ed.cursor = c,
            None => self.set_msg(""),
        }
        Ok(())
    }

    /// git-rebase-insert
    fn rebase_todo_insert(&mut self) -> Result<()> {
        let Some(rev) = self.read_line_prompt("Insert revision", None)? else { return Ok(()) };
        let Ok(oneline) = self.git.git_string(&["log", "-1", "--no-walk", "--format=%h %s", &rev, "--"]) else {
            self.set_msg("Unknown revision");
            return Ok(());
        };
        if let Some(ed) = self.rebase_todo.as_mut() {
            ed.cursor = ed.todo.insert_pick(ed.cursor, oneline.trim());
        }
        Ok(())
    }

    /// The rows of the todo buffer (git-rebase-mode-font-lock-keywords).
    pub(super) fn rebase_todo_rows(&self, body: usize, scroll: &mut usize) -> (Vec<Row>, String) {
        let t = &self.theme;
        let Some(ed) = &self.rebase_todo else { return (Vec::new(), String::new()) };
        let n = ed.todo.lines.len();
        if ed.cursor < *scroll {
            *scroll = ed.cursor;
        } else if ed.cursor >= *scroll + body {
            *scroll = ed.cursor + 1 - body;
        }
        let region = ed.mark.map(|m| (m.min(ed.cursor), m.max(ed.cursor)));
        let mut rows = Vec::new();
        for (i, line) in ed.todo.lines.iter().enumerate().skip(*scroll).take(body) {
            let base = if i == ed.cursor {
                t.style(Face::CursorLine)
            } else if region.is_some_and(|(a, b)| i >= a && i <= b) {
                t.style(Face::Region)
            } else {
                Style::default()
            };
            let on = |f: Face| {
                let s = t.style(f).over(base);
                Style { bg: base.bg.or(s.bg), ..s }
            };
            let mut r = Row::new(base);
            let gutter = if i == ed.cursor { t.glyphs.cursor } else { ' ' };
            r.push(format!("{gutter} "), on(Face::Gutter));
            let p = ed.todo.parsed(i);
            match p.typ {
                Some(LineType::Commit) if !p.comment => {
                    let face = if p.action == "drop" { Face::Removed } else { Face::Keyword };
                    let action = match &p.options {
                        Some(o) => format!("{} {o}", p.action),
                        None => p.action.clone(),
                    };
                    r.push(action, on(face)).push(" ", base);
                    r.push(p.target.clone(), on(Face::Hash));
                    if !p.trailer.is_empty() {
                        r.push(" ", base).push(p.trailer.clone(), on(if p.action == "drop" { Face::Removed } else { Face::Default }));
                    }
                }
                Some(_) if !p.comment => {
                    r.push(p.action.clone(), on(Face::Keyword));
                    if !p.target.is_empty() {
                        r.push(" ", base).push(p.target.clone(), on(Face::Label));
                    }
                }
                // commented (killed) lines and the instructions
                _ => {
                    r.push(line.clone(), on(Face::Dim));
                }
            }
            rows.push(r);
        }
        (rows, format!("{}/{}", (ed.cursor + 1).min(n), n))
    }
}
