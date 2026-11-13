//! The built-in text buffer used to edit files git would otherwise hand
//! to an external editor.  Two kinds:
//!   * git notes, branch descriptions and patch cover letters: read the
//!     text into the buffer and write it back on C-c C-c (synchronous).
//!   * rebase/sequencer messages (reword/squash, `--edit`, `--continue`):
//!     git waits on an EditorServer while its command runs on a worker
//!     thread; the buffer edits the file git passed and answers the server
//!     on C-c C-c / C-c C-k (see `edit_session.rs`).

use super::*;
use crate::ui::commit_msg::Message;

/// What to do with the text on C-c C-c.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextEditSave {
    /// `git notes add -f -F FILE --ref=REF REV` (empty text: remove)
    Notes { r: Option<String>, rev: String },
    /// `git config branch.<BRANCH>.description TEXT` (empty: unset)
    BranchDescription { branch: String },
    /// write the file PATH back (the cover letter of a patch series)
    File { path: std::path::PathBuf },
    /// answer the git editor request SEQ: write PATH, then reply "ok".
    /// C-c C-k replies "cancel" without writing.
    Reply { seq: u64, path: std::path::PathBuf },
}

/// The state of the built-in text buffer.
#[derive(Debug, Clone)]
pub struct TextEdit {
    pub message: Message,
    /// what the buffer is for, shown in the header
    pub title: String,
    pub save: TextEditSave,
    /// C-c was typed
    pub cc: bool,
}

const HELP: &str = "\
Edit the text, then C-c C-c

C-c C-c  save (write the file / set the note or description)
C-c C-k  cancel without changing anything
C-n/C-p  next / previous line (also up / down)
M-p/M-n  previous / next block of non-comment lines (also M-up / M-down)
C-a/C-e  beginning / end of line; C-b/C-f one character
RET      new line; DEL deletes backwards; C-d deletes forward
C-k      kill the whole line at point (ignores the column)
M-k      kill the whole non-comment block at point (a plain-text file is one block)
C-/ C-_  undo last change (a run of typing is one step)
?        show this help

The mode line shows line:column/total lines; the character at point is
highlighted and its whole non-comment block is shaded.  On a sequencer's
message (COMMIT_EDITMSG, SQUASH_MSG, ...) the point starts in the message
itself, not in git's comment instructions.";

impl App {
    /// Open the built-in text buffer with TEXT; C-c C-c saves it via SAVE.
    pub(super) fn text_edit_open(&mut self, text: &str, title: &str, save: TextEditSave) {
        // A sequencer's message (COMMIT_EDITMSG, SQUASH_MSG, ...) comes
        // with git's instruction lines: the point starts in the message
        // and M-p/M-n skip those comment blocks.  Notes, descriptions and
        // cover letters have no comment lines at all: '\0' is a marker no
        // line of these files is stripped for.
        let cc = match &save {
            TextEditSave::Reply { .. } => self.message_comment_char(),
            _ => '\0',
        };
        self.text_edit = Some(TextEdit { message: Message::new(text, cc), title: title.to_string(), save, cc: false });
        self.setup_buffer(BufferMode::TextEdit, Vec::new());
        self.set_msg("Type C-c C-c to save, C-c C-k to cancel, ? for help");
    }

    /// The text being edited (tests).
    pub fn text_edit_text(&self) -> Option<String> {
        self.text_edit.as_ref().map(|e| e.message.text())
    }

    /// The title of the text buffer (tests).
    pub fn text_edit_title(&self) -> Option<String> {
        self.text_edit.as_ref().map(|e| e.title.clone())
    }

    /// The point (row, col) in the text buffer (tests).
    pub fn text_edit_point(&self) -> Option<(usize, usize)> {
        self.text_edit.as_ref().map(|e| (e.message.row, e.message.col))
    }

    /// The point's position shown in the mode line (tests).
    pub fn text_edit_position(&self) -> Option<String> {
        self.text_edit.as_ref().map(|e| e.message.position())
    }

    /// Replace the whole text buffer (tests: what an editor would write).
    pub fn text_edit_set(&mut self, text: &str) {
        if let Some(ed) = self.text_edit.as_mut() {
            let cc = ed.message.comment_char;
            ed.message = Message::new(text, cc);
        }
    }

    fn text_edit_close(&mut self) -> Option<TextEdit> {
        let ed = self.text_edit.take();
        if self.mode == BufferMode::TextEdit {
            self.kill_buffer();
        }
        ed
    }

    /// C-c C-c: apply the edited text (write it back / run git / answer
    /// the waiting git editor), then refresh.  Verbatim with a trailing
    /// newline; comment lines are NOT stripped here (git does that itself
    /// for commit messages).
    fn text_edit_finish(&mut self) -> Result<()> {
        let Some(ed) = self.text_edit_close() else { return Ok(()) };
        let text = ed.message.text();
        let body = text.trim_end_matches('\n');
        match &ed.save {
            TextEditSave::Notes { r, rev } => {
                let res = self.save_note(r.as_deref(), rev, body);
                self.done("", res.map(|()| String::new()));
            }
            TextEditSave::BranchDescription { branch } => {
                let res = self.save_branch_description(branch, body);
                self.done("", res.map(|()| String::new()));
            }
            TextEditSave::File { path } => {
                let res = std::fs::write(path, text.as_bytes())
                    .map_err(|e| anyhow::anyhow!("Cannot write {}: {e}", path.display()));
                self.done("", res.map(|()| String::new()));
            }
            TextEditSave::Reply { seq, path } => {
                let res = std::fs::write(path, text.as_bytes())
                    .map_err(|e| anyhow::anyhow!("Cannot write {}: {e}", path.display()));
                let ok = res.is_ok();
                if let Err(e) = res {
                    self.set_msg(&format!("{e:#}"));
                }
                self.answer_edit(*seq, ok);
            }
        }
        Ok(())
    }

    /// `git notes add -f -F FILE REV` (or remove for empty text), with the
    /// optional --ref.  git keeps the text verbatim, like an external editor.
    fn save_note(&self, r: Option<&str>, rev: &str, body: &str) -> Result<()> {
        if body.is_empty() {
            let mut a = vec!["notes".to_string()];
            if let Some(r) = r {
                a.push(format!("--ref={r}"));
            }
            a.extend(["remove".to_string(), rev.to_string()]);
            let strs: Vec<&str> = a.iter().map(String::as_str).collect();
            return self.git.git_ok(&strs).then_some(()).ok_or_else(|| anyhow::anyhow!("git notes remove failed"));
        }
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().map_err(|e| anyhow::anyhow!("temp file: {e}"))?;
        file.write_all(format!("{body}\n").as_bytes()).map_err(|e| anyhow::anyhow!("write note: {e}"))?;
        file.flush().ok();
        let mut a = vec!["notes".to_string()];
        if let Some(r) = r {
            a.push(format!("--ref={r}"));
        }
        a.extend(["add".to_string(), "-f".to_string(), "-F".to_string(), file.path().to_string_lossy().into_owned(), rev.to_string()]);
        let strs: Vec<&str> = a.iter().map(String::as_str).collect();
        self.git.git_interactive(&strs)
    }

    fn save_branch_description(&self, branch: &str, body: &str) -> Result<()> {
        let var = format!("branch.{branch}.description");
        let a: Vec<&str> = if body.is_empty() {
            vec!["config", "--unset", &var]
        } else {
            vec!["config", "--replace-all", &var, body]
        };
        // an unset of a missing key is not an error
        if body.is_empty() && !self.git.git_ok(&["config", "--get", &var]) {
            return Ok(());
        }
        self.git.git_interactive(&a)
    }

    fn text_edit_cancel(&mut self) -> Result<()> {
        let ed = self.text_edit_close();
        if let Some(TextEdit { save: TextEditSave::Reply { seq, .. }, .. }) = ed {
            // git aborts that sequencer step (a stopped rebase stays)
            self.answer_edit(seq, false);
            return Ok(());
        }
        self.set_msg("Edit aborted");
        self.refresh();
        Ok(())
    }

    /// A key in the text buffer. True if handled.
    pub(super) fn text_edit_key(&mut self, key: &Key) -> Result<bool> {
        if self.mode != BufferMode::TextEdit || self.view_text.is_some() || self.text_edit.is_none() {
            return Ok(false);
        }
        if self.cx_prefix || (key.ctrl && key.code == "x") {
            return Ok(false);
        }
        if std::mem::take(&mut self.text_edit.as_mut().unwrap().cc) {
            match (key.ctrl, key.code.as_str()) {
                (true, "c") => self.text_edit_finish()?,
                (true, "k") => self.text_edit_cancel()?,
                _ => self.set_msg(&format!("C-c {} is undefined", key.label())),
            }
            return Ok(true);
        }
        let n = self.prefix_numeric() as isize;
        let m = &mut self.text_edit.as_mut().unwrap().message;
        match (key.ctrl, key.code.as_str()) {
            (true, "c") => {
                self.text_edit.as_mut().unwrap().cc = true;
                self.set_msg("C-c-");
            }
            (true, "n" | "down") | (false, "down") => m.move_line(1),
            (true, "p" | "up") | (false, "up") => m.move_line(-1),
            (false, "M-n" | "M-down") => m.move_block(n),
            (false, "M-p" | "M-up") => m.move_block(-n),
            (true, "a") => m.home(),
            (true, "e") => m.end(),
            (true, "b" | "left") | (false, "left") => m.move_col(-1),
            (true, "f" | "right") | (false, "right") => m.move_col(1),
            (true, "d") => m.delete_forward(),
            (true, "k") => m.kill_line(),
            (true, "/" | "_") => {
                let undone = m.undo();
                self.set_msg(if undone { "Undo" } else { "No further undo information" });
            }
            (false, "M-k") => {
                if !m.kill_block() {
                    self.set_msg("No message block to kill");
                }
            }
            (false, "enter") => m.newline(),
            (false, "backspace" | "DEL") => {
                m.delete_backward();
            }
            (false, "tab") => m.insert('\t'),
            (false, "?") => {
                self.view_text = Some((self.text_edit_title().unwrap_or_default(), HELP.into()));
                self.view_scroll = 0;
            }
            (false, "q") => self.set_msg("q is undefined (C-c C-k cancels, C-x C-c quits)"),
            (false, c) if c.chars().count() == 1 => {
                let ch = c.chars().next().unwrap();
                m.insert(ch);
            }
            _ => self.set_msg(&format!("{} is undefined", key.label())),
        }
        Ok(true)
    }

    /// The rows of the text buffer: the whole block at point is shaded,
    /// comment lines (of a sequencer message) are dim; the mode line
    /// shows line:column/total lines.
    pub(super) fn text_edit_rows(&self, body: usize, w: usize) -> (Vec<Row>, String) {
        let Some(ed) = &self.text_edit else { return (Vec::new(), String::new()) };
        let m = &ed.message;
        let scroll = if m.row >= body { m.row + 1 - body } else { 0 };
        let dim_comments = m.comment_char != '\0';
        let block = m.current_block();
        let rows = (scroll..m.lines.len().min(scroll + body)).map(|i| self.message_row(m, i, w, dim_comments, block)).collect();
        (rows, m.position())
    }
}
