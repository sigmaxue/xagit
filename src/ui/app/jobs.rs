//! ui/app/jobs.rs — work done away from the event loop, so the TUI keeps
//! responding: the buffer is refreshed on a worker thread after each
//! command (magit-refresh), and the commands that talk to remotes
//! (fetch, pull, push) run as background processes (magit-run-git-async).
//!
//! A refresh asked for while a command runs happens once, when the
//! command is done. Outside a terminal (tests) it happens right then, on
//! the same thread, so the result is the same as with a synchronous
//! refresh. Code that needs the refreshed buffer before the command ends
//! calls `settle`.

use super::*;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

/// What a worker sends back.
pub(super) enum Done {
    /// a buffer built for the refresh of generation GEN of MODE and ARGS
    Refresh { gen: u64, mode: BufferMode, args: Vec<String>, buf: Result<Box<StatusBuffer>, String>, head: String },
    /// the git process of job ID started as PID
    Started { id: u64, pid: u32 },
    /// job ID is done
    Git { id: u64, args: Vec<String>, ok: String, result: Result<(), (String, bool)> },
}

/// A git command running in the background.
pub(super) struct Running {
    id: u64,
    label: String,
    pid: Option<u32>,
}

pub(super) struct Jobs {
    tx: Sender<Done>,
    rx: Receiver<Done>,
    /// work is handed to worker threads (the TUI runs in a terminal)
    pub enabled: bool,
    /// commands being run: a refresh waits for the outermost
    pub depth: usize,
    /// a refresh was asked for and not started yet
    pub wanted: bool,
    /// the generation of the latest refresh; older results are dropped
    gen: u64,
    /// the refresh of generation `gen` is running on a worker
    pub refreshing: bool,
    running: Vec<Running>,
    next_id: u64,
    /// when the work shown by the busy indicator began
    since: Option<Instant>,
}

impl Default for Jobs {
    fn default() -> Self {
        let (tx, rx) = channel();
        Jobs {
            tx,
            rx,
            enabled: false,
            depth: 0,
            wanted: false,
            gen: 0,
            refreshing: false,
            running: Vec::new(),
            next_id: 0,
            since: None,
        }
    }
}

impl Jobs {
    /// Whether anything runs in the background.
    pub fn busy(&self) -> bool {
        self.refreshing || !self.running.is_empty()
    }

    /// How long the event loop waits for a key before it looks for
    /// results (and turns the busy indicator).
    pub fn tick(&self) -> Duration {
        if self.busy() {
            Duration::from_millis(15)
        } else {
            Duration::from_secs(3600)
        }
    }

    /// The frame of the busy indicator shown now (none when idle).
    pub fn frame(&self) -> Option<usize> {
        let since = self.since.filter(|_| self.busy())?;
        Some((since.elapsed().as_millis() / 100) as usize)
    }

    fn mark_busy(&mut self) {
        if self.since.is_none() {
            self.since = Some(Instant::now());
        }
    }

    fn mark_idle(&mut self) {
        if !self.busy() {
            self.since = None;
        }
    }
}

/// Everything the refresh function of a buffer reads, so it can be run
/// away from the App.
pub(super) struct BufferSpec {
    git: Git,
    mode: BufferMode,
    buf_args: Vec<String>,
    status_diff_args: Vec<String>,
    status_log_args: Vec<String>,
    margin: Margin,
    options: Options,
    refs_counts: CommitCount,
    vis: Visibility,
}

impl BufferSpec {
    /// The refresh function of the buffer's mode.
    pub fn build(&self) -> StatusBuffer {
        // libgit2 answers the read-only queries of this refresh in-process
        let _queries = crate::git::query::enter(&self.git.toplevel);
        let (git, margin) = (&self.git, &self.margin);
        match &self.mode {
            BufferMode::Status => {
                let st = Status::load(git);
                crate::ui::view::build_status_with(
                    git,
                    &st,
                    &self.status_diff_args,
                    &self.status_log_args,
                    margin,
                    self.options.status_worktrees,
                )
            }
            BufferMode::Diff(spec) => build_diff(git, spec, &self.buf_args),
            BufferMode::Revision { rev, files } => build_revision(git, rev, &self.buf_args, files),
            BufferMode::Stash { stash } => build_stash(git, stash, &self.buf_args),
            BufferMode::Log { revs, files } => build_log(git, revs, &self.buf_args, files, margin),
            BufferMode::LogSelect { rev, msg, .. } => {
                // magit-log-select-refresh-buffer; the header line shows
                // the usage (magit-log-select-show-usage)
                let mut buf = build_log(git, std::slice::from_ref(rev), &self.buf_args, &[], margin);
                buf.title = msg.clone();
                buf
            }
            BufferMode::Reflog { refname } => build_reflog(git, refname, &self.buf_args, margin),
            BufferMode::Stashes { refname } => build_stashes(git, refname, margin),
            BufferMode::Cherry { head, upstream } => build_cherry(git, head, upstream, margin),
            BufferMode::File { rev, file, blame } => build_file(git, rev, file, blame.as_ref()),
            BufferMode::Worktrees => crate::ui::view::build_worktrees(git),
            // the list is drawn from App::rebase_todo
            BufferMode::RebaseTodo => {
                let mut buf = StatusBuffer::default();
                buf.title = "git-rebase-todo".into();
                buf
            }
            BufferMode::Refs { upstream } => {
                let show = |r: &str| self.options.show_ref(r);
                build_refs(git, upstream, &self.buf_args, self.refs_counts, &self.vis, margin, &show)
            }
        }
    }
}

impl App {
    /// The inputs of the refresh function of the buffer shown.
    pub(super) fn buffer_spec(&self) -> BufferSpec {
        BufferSpec {
            git: self.git.clone(),
            mode: self.mode.clone(),
            buf_args: self.buf_args.clone(),
            status_diff_args: self.status_diff_args(),
            status_log_args: self.status_log_args.clone(),
            margin: self.margin_config(),
            options: self.options.clone(),
            refs_counts: self.refs_counts,
            vis: self.vis.clone(),
        }
    }

    /// magit-refresh: rebuild the buffer, keeping point in the same
    /// section at the same offset (or at the same line when that section
    /// is gone). While a command runs this only takes note; the buffer is
    /// rebuilt once the command is done (in the background in a terminal).
    pub fn refresh(&mut self) {
        self.jobs.wanted = true;
        if self.jobs.depth == 0 {
            self.flush_refresh();
        }
    }

    /// Start the refresh asked for: on a worker in a terminal, else now.
    pub(super) fn flush_refresh(&mut self) {
        if !std::mem::take(&mut self.jobs.wanted) {
            return;
        }
        if !self.jobs.enabled {
            self.refresh_now();
            return;
        }
        self.jobs.gen += 1;
        self.jobs.refreshing = true;
        self.jobs.mark_busy();
        let spec = self.buffer_spec();
        let (gen, tx) = (self.jobs.gen, self.jobs.tx.clone());
        std::thread::spawn(move || {
            let _queries = crate::git::query::enter(&spec.git.toplevel);
            let buf = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Box::new(spec.build()))).map_err(|p| {
                p.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into())
            });
            let head = spec.git.head_desc();
            let _ = tx.send(Done::Refresh { gen, mode: spec.mode, args: spec.buf_args, buf, head });
        });
    }

    /// Rebuild the buffer right away, on this thread.
    pub(super) fn refresh_now(&mut self) {
        self.jobs.wanted = false;
        // a refresh still running on a worker is out of date now
        self.jobs.gen += 1;
        self.jobs.refreshing = false;
        self.jobs.mark_idle();
        let _queries = crate::git::query::enter(&self.git.toplevel);
        let buf = self.buffer_spec().build();
        if self.in_terminal {
            self.head_label = self.git.head_desc();
        }
        self.show_refreshed(buf);
    }

    /// Show BUF, the buffer rebuilt, with point where it was.
    fn show_refreshed(&mut self, buf: StatusBuffer) {
        let anchor = self.anchor();
        self.buf = buf;
        self.mark = None;
        self.lines = self.buf.visible(&self.vis);
        let (id, fallback) = anchor;
        let target = id.and_then(|(ident, off)| {
            let sec = self.buf.sections.iter().skip(1).find(|s| s.ident == ident)?;
            Some(if sec.start + off < sec.end { sec.start + off } else { sec.start })
        });
        self.cursor = match target {
            Some(p) => self.visible_index_near(p),
            None => fallback.min(self.lines.len().saturating_sub(1)),
        };
    }

    /// Make the buffer shown up to date: do the refresh asked for, or
    /// wait for the one running.
    pub(super) fn settle(&mut self) {
        loop {
            if self.jobs.wanted {
                self.refresh_now();
                continue;
            }
            if !self.jobs.refreshing {
                return;
            }
            match self.jobs.rx.recv() {
                Ok(done) => self.finish(done),
                Err(_) => {
                    self.jobs.refreshing = false;
                    return;
                }
            }
        }
    }

    /// Show the work done in the background so far. True if anything
    /// was done.
    pub(super) fn poll_jobs(&mut self) -> bool {
        let mut any = false;
        while let Ok(done) = self.jobs.rx.try_recv() {
            self.finish(done);
            any = true;
        }
        any
    }

    /// Wait until no work runs in the background (tests, quitting).
    pub fn wait_background(&mut self) {
        while self.jobs.busy() || self.jobs.wanted {
            if self.jobs.wanted {
                self.flush_refresh();
                continue;
            }
            match self.jobs.rx.recv() {
                Ok(done) => self.finish(done),
                Err(_) => return,
            }
        }
    }

    /// Hand refreshes and remote commands to worker threads (as when
    /// running in a terminal), or do them right away.
    pub fn set_background(&mut self, on: bool) {
        self.jobs.enabled = on;
    }

    fn finish(&mut self, done: Done) {
        match done {
            Done::Refresh { gen, mode, args, buf, head } => {
                if gen != self.jobs.gen {
                    return;
                }
                self.jobs.refreshing = false;
                self.jobs.mark_idle();
                self.head_label = head;
                // the buffer shown is another one by now
                if mode != self.mode || args != self.buf_args {
                    return;
                }
                match buf {
                    Ok(buf) => self.show_refreshed(*buf),
                    Err(e) => self.set_msg(&format!("Refresh failed: {e}")),
                }
            }
            Done::Started { id, pid } => {
                if let Some(r) = self.jobs.running.iter_mut().find(|r| r.id == id) {
                    r.pid = Some(pid);
                }
            }
            Done::Git { id, args, ok, result } => {
                self.jobs.running.retain(|r| r.id != id);
                self.jobs.mark_idle();
                match result {
                    Ok(()) => self.set_msg(&ok),
                    // it wanted to ask something: again, in the terminal
                    Err((_, true)) => {
                        let strs: Vec<&str> = args.iter().map(String::as_str).collect();
                        match self.with_terminal(|g| g.git_interactive(&strs)) {
                            Ok(()) => self.set_msg(&ok),
                            Err(e) => self.set_msg(&format!("{e:#}")),
                        }
                    }
                    Err((e, false)) => self.set_msg(&e),
                }
                self.refresh();
            }
        }
    }

    /// Run the git command A in the background (magit-run-git-async),
    /// then refresh; OK is the message shown when it succeeds. Without
    /// a terminal it runs right away.
    pub(super) fn run_git_async(&mut self, a: &[String], ok: &str) {
        if !self.jobs.enabled {
            let strs: Vec<&str> = a.iter().map(String::as_str).collect();
            match self.git.git_interactive(&strs) {
                Ok(()) => self.set_msg(ok),
                Err(e) => self.set_msg(&format!("{e:#}")),
            }
            self.refresh();
            return;
        }
        self.jobs.next_id += 1;
        let id = self.jobs.next_id;
        let label = format!("git {}", a.iter().take(2).filter(|s| !s.starts_with('-')).cloned().collect::<Vec<_>>().join(" "));
        self.set_msg(&format!("Running {label}..."));
        self.jobs.running.push(Running { id, label, pid: None });
        self.jobs.mark_busy();
        let (git, tx, args, ok) = (self.git.clone(), self.jobs.tx.clone(), a.to_vec(), ok.to_string());
        std::thread::spawn(move || {
            let strs: Vec<&str> = args.iter().map(String::as_str).collect();
            let started = tx.clone();
            let result = git.git_detached(&strs, move |pid| {
                let _ = started.send(Done::Started { id, pid });
            });
            let _ = tx.send(Done::Git { id, args, ok, result });
        });
    }

    /// The busy indicator of the mode line: a turning glyph and what
    /// runs (none when idle).
    pub(super) fn busy_label(&self) -> Option<String> {
        let frames = self.theme.glyphs.spinner;
        let frame = frames[self.jobs.frame()? % frames.len()];
        let what = match self.jobs.running.as_slice() {
            [] => "refreshing".to_string(),
            [one] => one.label.clone(),
            many => format!("{} git processes", many.len()),
        };
        Some(format!("{frame} {what}"))
    }

    /// Whether KEY only moves point, so it can be done in the buffer
    /// shown while its refresh runs.
    pub(super) fn only_moves(&self, key: &Key) -> bool {
        if !self.stack.is_empty()
            || !self.pending.is_empty()
            || self.transient_help.is_some()
            || self.cc_prefix
            || self.cx_prefix
            || self.smerge_prefix
            || self.reading_arg
            || self.prefix_arg.is_some()
            || matches!(self.mode, BufferMode::File { .. })
        {
            return false;
        }
        matches!(
            status_binding(key),
            Some(
                Binding::Down
                    | Binding::Up
                    | Binding::SectionForward
                    | Binding::SectionBackward
                    | Binding::SiblingForward
                    | Binding::SiblingBackward
            )
        )
    }

    /// Before quitting: the git processes still running are killed when
    /// the user agrees, else xagit stays. True if xagit may quit.
    pub(super) fn quit_with_jobs(&mut self) -> bool {
        if self.jobs.running.is_empty() {
            return true;
        }
        let n = self.jobs.running.len();
        let prompt = format!("{n} git process{} still running; kill and quit? ", if n == 1 { "" } else { "es" });
        if !self.yes_no(&prompt).unwrap_or(false) {
            self.quit = false;
            self.set_msg("");
            return false;
        }
        #[cfg(unix)]
        for pid in self.jobs.running.iter().filter_map(|r| r.pid) {
            // the process group: git and the ssh it started
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
        }
        true
    }
}
