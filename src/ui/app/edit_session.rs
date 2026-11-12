//! A git command whose editor is xagit's built-in text buffer.  The git
//! process runs on a worker thread with its output captured (the TUI
//! keeps the terminal); GIT_EDITOR is an EditorServer script that hands
//! each file to xagit and waits.  `poll_edit` is driven by the event loop
//! (and synchronously in tests), so nothing blocks the UI.

use super::*;
use crate::git::editor_server::{EditRequest, EditorServer};
use crate::git::exec::Editor;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// How to report the end of the git command.
#[derive(Debug, Clone)]
pub enum EditDone {
    /// a rebase command (start): report like after_rebase
    Rebase,
    /// a cherry-pick / revert run: NAME on success
    Sequence(String),
    /// a plain command: OK on success (empty: no message)
    Plain(String),
}

/// A git command running with xagit as its editor.
pub struct EditSession {
    server: EditorServer,
    result: Receiver<Result<(), String>>,
    done: EditDone,
    /// a rebase todo temp file kept alive until git has read it
    _todo_file: Option<tempfile::NamedTempFile>,
    /// a request already handed to the text buffer (SEQ), awaiting C-c C-c
    pending: Option<u64>,
}

impl EditSession {
    /// On quit: answer a request the buffer has not finished so the
    /// waiting editor script (and git) can exit.
    pub(super) fn cancel_pending(&self) {
        if let Some(seq) = self.pending {
            self.server.answer(seq, false);
        }
    }

    /// Release any editor script still waiting for its first request.
    pub(super) fn server_stop(&self) {
        self.server.stop();
    }
}

/// A short poll interval for the file handshake.
const PUMP: Duration = Duration::from_millis(5);
/// How long to wait (synchronously) for git to ask for an edit or finish.
const SYNC_WAIT: Duration = Duration::from_secs(10);

fn title_for(req: &EditRequest) -> String {
    req.path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "COMMIT_EDITMSG".into())
}

impl App {
    /// Run `git ARGS` with xagit's text buffer as the message editor.
    /// TODO is the sequence editor command (a perl todo rewrite / a `cp`
    /// of the built-in todo buffer, or None = accept).  KEEP is a temp
    /// file that must live until git has consumed it.
    pub(super) fn start_edit_git(
        &mut self,
        args: Vec<String>,
        todo: Option<String>,
        done: EditDone,
        keep: Option<tempfile::NamedTempFile>,
    ) -> Result<()> {
        let server = EditorServer::new().map_err(|e| anyhow::anyhow!("Cannot start the editor: {e}"))?;
        let editor = server.command();
        let (tx, rx) = std::sync::mpsc::channel();
        let git = self.git.clone();
        std::thread::spawn(move || {
            let strs: Vec<&str> = args.iter().map(String::as_str).collect();
            let todo_ed = match &todo {
                Some(t) => Editor::Command(t.as_str()),
                None => Editor::Accept,
            };
            let res = git.git_with_editors_capture(todo_ed, Editor::Command(&editor), &[], &strs);
            let _ = tx.send(res);
        });
        self.edit_session = Some(EditSession { server, result: rx, done, _todo_file: keep, pending: None });
        self.jobs.editing = true;
        self.jobs.mark_busy();
        // In tests there is no event loop: pump until git asks for its
        // first edit (the buffer opens) or finishes.  In the terminal the
        // event loop polls instead (one non-blocking check here).
        self.settle_edit(self.sync_edit_wait())
    }

    /// Answer the pending editor request and drive git to its next step.
    pub(super) fn answer_edit(&mut self, seq: u64, ok: bool) {
        if let Some(s) = &self.edit_session {
            s.server.answer(seq, ok);
        }
        if let Some(s) = &mut self.edit_session {
            s.pending = None;
        }
        let _ = self.settle_edit(self.sync_edit_wait());
    }

    /// Tests have no event loop, so pump synchronously; the terminal gets
    /// a single non-blocking check and `poll_edit` from the loop does the rest.
    fn sync_edit_wait(&self) -> Duration {
        if self.in_terminal { Duration::ZERO } else { SYNC_WAIT }
    }

    /// Drive the handshake: open the buffer on a new request, or finish
    /// when git ends.  Non-blocking check in the terminal event loop;
    /// WAIT bounds the synchronous wait used in tests / right after C-c.
    pub(super) fn settle_edit(&mut self, wait: Duration) -> Result<()> {
        let deadline = Instant::now() + wait;
        loop {
            if self.text_edit_open_request()? {
                return Ok(());
            }
            if !self.has_edit_session() || Instant::now() >= deadline {
                return Ok(());
            }
            std::thread::sleep(PUMP);
        }
    }

    fn has_edit_session(&self) -> bool {
        self.edit_session.is_some()
    }

    /// Called from `poll_jobs` on every event-loop turn.
    pub(super) fn poll_edit(&mut self) {
        let _ = self.text_edit_open_request();
    }

    /// True when a buffer was opened or the session finished (so a
    /// synchronous pump can stop waiting for the first edit).
    fn text_edit_open_request(&mut self) -> Result<bool> {
        // git finished?
        let finished = self
            .edit_session
            .as_ref()
            .and_then(|s| s.result.try_recv().ok());
        if let Some(res) = finished {
            let s = self.edit_session.take().unwrap();
            s.server.stop();
            self.jobs.editing = false;
            self.jobs.mark_idle();
            self.finish_edit_git(s.done, res);
            return Ok(true);
        }
        // a new editor request, and no buffer is open yet
        let req = self.edit_session.as_ref().and_then(|s| s.server.take_request());
        let Some(req) = req else { return Ok(false) };
        if self.text_edit.is_some() {
            // shouldn't happen: one request at a time; answer cancel
            if let Some(s) = &self.edit_session {
                s.server.answer(req.seq, false);
            }
            return Ok(false);
        }
        let text = std::fs::read_to_string(&req.path).unwrap_or_default();
        let title = title_for(&req);
        let seq = req.seq;
        if let Some(s) = &mut self.edit_session {
            s.pending = Some(seq);
        }
        self.text_edit_open(&text, &title, text_edit::TextEditSave::Reply { seq, path: req.path });
        Ok(true)
    }

    fn finish_edit_git(&mut self, done: EditDone, res: Result<(), String>) {
        let result = res.map_err(|e| anyhow::anyhow!("{e}"));
        match done {
            EditDone::Rebase => self.after_rebase(result),
            EditDone::Sequence(name) => self.done(&name, result.map(|()| String::new())),
            EditDone::Plain(ok) => self.done(&ok, result.map(|()| String::new())),
        }
    }
}
