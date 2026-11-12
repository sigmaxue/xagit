//! The built-in COMMIT_EDITMSG buffer (an xagit addition): the `c` menu
//! edits the commit message here instead of handing the terminal to an
//! external editor.  C-n/C-p (and the arrows) move between lines, normal
//! keys edit the text, C-c C-c commits, C-c C-k cancels.

use super::*;
use crate::ui::commit_msg::Message;

/// The state of the commit message buffer.
#[derive(Debug, Clone)]
pub struct CommitEdit {
    pub message: Message,
    /// the `git commit` arguments (already validated by commit_assert)
    pub args: Vec<String>,
    /// environment for the commit (e.g. GIT_COMMITTER_DATE)
    pub env: Vec<(String, String)>,
    /// the message shown when git commit succeeded
    pub done_msg: String,
    /// C-c was typed
    pub cc: bool,
}

const HELP: &str = "\
COMMIT_EDITMSG — write the commit message, then C-c C-c

C-c C-c  commit (git commit with this message)
C-c C-k  cancel without committing
C-n/C-p  next / previous line (also up / down)
C-a/C-e  beginning / end of line; C-b/C-f one character
RET      new line; DEL deletes backwards; C-d deletes forward; C-k kills to EOL
?        show this help

Comment lines (starting with the comment char) and the scissors trailer
are removed by git before committing.";

impl App {
    /// Open the built-in message buffer for `git commit ARGS`.  Returns true
    /// when the buffer was opened (the caller must not run git itself).
    pub(super) fn commit_msg_open(&mut self, args: Vec<String>, env: Vec<(String, String)>, done_msg: &str) -> Result<bool> {
        // git does not need an editor (and would not start one) when the
        // message is taken from elsewhere: -C/--reuse-message, -c,
        // --fixup/--squash without --edit, -m/-F and --no-edit.
        let no_editor = args.iter().any(|a| {
            a == "--no-edit"
                || a == "-m" || a == "-F" || a.starts_with("-m") || a.starts_with("--message=") || a.starts_with("--file=")
                || a.starts_with("--reuse-message=") || a.starts_with("--reedit-message=")
                || a.starts_with("--fixup=") || a.starts_with("--squash=")
        });
        if no_editor {
            let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let res = self.with_terminal(|g| g.commit_with_editor_env(&args, Editor::Accept, &env_refs));
            self.done(done_msg, res.map(|()| String::new()));
            return Ok(false);
        }
        let template = self.git.commit_message_template(&args);
        let cc = self
            .git
            .git_string(&["config", "core.commentChar"])
            .ok()
            .map(|s| s.trim().chars().next().unwrap_or('#'))
            .filter(|c| *c != '\0' && *c != 'a')
            .unwrap_or('#');
        // message body, a blank separator, then the comment instructions
        let mut text = template;
        text.push_str("\n\n");
        text.push_str(&format!("{cc} Please enter the commit message for your changes.\n{cc} C-c C-c to commit, C-c C-k to cancel.\n"));
        self.commit_edit = Some(CommitEdit {
            message: Message::new(&text, cc),
            args,
            env,
            done_msg: done_msg.to_string(),
            cc: false,
        });
        self.setup_buffer(BufferMode::Commit, Vec::new());
        self.set_msg("Type C-c C-c to commit, C-c C-k to cancel, ? for help");
        Ok(true)
    }

    /// The message being edited (tests).
    pub fn commit_msg_text(&self) -> Option<String> {
        self.commit_edit.as_ref().map(|e| e.message.text())
    }

    /// The point (row, col) in the message buffer (tests).
    pub fn commit_msg_point(&self) -> Option<(usize, usize)> {
        self.commit_edit.as_ref().map(|e| (e.message.row, e.message.col))
    }

    fn commit_msg_close(&mut self) -> Option<CommitEdit> {
        let ed = self.commit_edit.take();
        if self.mode == BufferMode::Commit {
            self.kill_buffer();
        }
        ed
    }

    /// C-c C-c: run `git commit` with the edited message.
    fn commit_msg_finish(&mut self) -> Result<()> {
        let Some(ed) = self.commit_msg_close() else { return Ok(()) };
        if ed.message.committed_message().is_empty() {
            self.set_msg("Abort: empty commit message");
            return Ok(());
        }
        let env: Vec<(&str, &str)> = ed.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let res = self.with_terminal(|g| g.commit_with_message(&ed.args, &env, &ed.message.text()));
        self.done(&ed.done_msg, res.map(|()| String::new()));
        Ok(())
    }

    fn commit_msg_cancel(&mut self) -> Result<()> {
        self.commit_msg_close();
        self.set_msg("Commit aborted");
        self.refresh();
        Ok(())
    }

    /// A key in the COMMIT_EDITMSG buffer. True if handled.
    pub(super) fn commit_msg_key(&mut self, key: &Key) -> Result<bool> {
        if self.mode != BufferMode::Commit || self.view_text.is_some() || self.commit_edit.is_none() {
            return Ok(false);
        }
        if self.cx_prefix || (key.ctrl && key.code == "x") {
            return Ok(false);
        }
        let ed = self.commit_edit.as_mut().unwrap();
        if std::mem::take(&mut ed.cc) {
            match (key.ctrl, key.code.as_str()) {
                (true, "c") => self.commit_msg_finish()?,
                (true, "k") => self.commit_msg_cancel()?,
                _ => self.set_msg(&format!("C-c {} is undefined", key.label())),
            }
            return Ok(true);
        }
        let m = &mut self.commit_edit.as_mut().unwrap().message;
        match (key.ctrl, key.code.as_str()) {
            (true, "c") => {
                self.commit_edit.as_mut().unwrap().cc = true;
                self.set_msg("C-c-");
            }
            (true, "n" | "down") | (false, "down") => m.move_line(1),
            (true, "p" | "up") | (false, "up") => m.move_line(-1),
            (true, "a") => m.home(),
            (true, "e") => m.end(),
            (true, "b" | "left") | (false, "left") => m.move_col(-1),
            (true, "f" | "right") | (false, "right") => m.move_col(1),
            (true, "d") => m.delete_forward(),
            (true, "k") => m.kill_line(),
            (false, "enter") => m.newline(),
            (false, "backspace" | "DEL") => {
                m.delete_backward();
            }
            (false, "tab") => m.insert('\t'),
            (false, "?") => {
                self.view_text = Some(("COMMIT_EDITMSG".into(), HELP.into()));
                self.view_scroll = 0;
            }
            (false, "q") => self.set_msg("q is undefined (C-c C-k cancels, C-x C-c quits)"),
            (false, c) if !key.ctrl && c.chars().count() == 1 => {
                let ch = c.chars().next().unwrap();
                m.insert(ch);
            }
            _ => self.set_msg(&format!("{} is undefined", key.label())),
        }
        Ok(true)
    }

    /// The rows of the COMMIT_EDITMSG buffer.  The point's line carries the
    /// cursor-line face; comment lines are dim.
    pub(super) fn commit_msg_rows(&self, body: usize, w: usize) -> (Vec<Row>, String) {
        let t = &self.theme;
        let Some(ed) = &self.commit_edit else { return (Vec::new(), String::new()) };
        let m = &ed.message;
        let mut scroll = self.scroll;
        if m.row < scroll {
            scroll = m.row;
        } else if m.row >= scroll + body {
            scroll = m.row + 1 - body;
        }
        let mut rows = Vec::new();
        for (i, line) in m.lines.iter().enumerate().skip(scroll).take(body) {
            let base = if i == m.row { t.style(Face::CursorLine) } else { Style::default() };
            let face = if line.starts_with(m.comment_char) { Face::Dim } else { Face::Default };
            let mut r = Row::new(base);
            let gutter = if i == m.row { t.glyphs.cursor } else { ' ' };
            r.push(format!("{gutter} "), t.style(Face::Gutter).over(base));
            let shown: String = line.chars().take(w.saturating_sub(2)).collect();
            r.push(shown, t.style(face).over(base));
            rows.push(r);
        }
        (rows, format!("{}/{}", m.row + 1, m.lines.len()))
    }
}
