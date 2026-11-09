//! ui/app.rs — application state machine, event loop, prompts and command dispatch.

use super::keys::Key;
use super::transient::{diff_refresh_menu, registry, resolve_menu, Action, Lookup, Menu, MenuEntry, Reader, RepoState};
use super::diffbuf::{build_diff, build_revision, build_stash, DiffSpec};
use super::logbuf::{build_cherry, build_log, build_reflog, build_stashes, set_commit_limit, Limit, REFLOG_LIMIT};
use super::view::{build_status, BufferLine, LineKind, Section, StatusBuffer, Visibility};
use crate::git::apply::diff_context;
use crate::git::diff::{FileDiff, Scope};
use crate::git::exec::Editor;
use crate::git::ops::Untracked;
use crate::git::status::Status;
use crate::git::Git;
use anyhow::Result;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use std::collections::BTreeMap;
use std::io::{stdout, Write};

pub struct App {
    pub git: Git,
    menus: BTreeMap<String, Menu>,
    /// stack of open transient menus + their active switches
    stack: Vec<(String, BTreeMap<String, bool>)>,
    /// keys typed so far of a multi-key transient binding ("-" of "-a")
    pending: Vec<Key>,
    /// every section of the status buffer, expanded, plus the diffs
    buf: StatusBuffer,
    /// section visibility (ident -> hidden), kept across refreshes
    vis: Visibility,
    /// the visible lines of the status buffer
    lines: Vec<BufferLine>,
    cursor: usize,
    /// first visible line of the status buffer window
    scroll: usize,
    /// the mark (index into `buf.all`) while the region is active (C-SPC)
    mark: Option<usize>,
    /// the major mode of the buffer shown (status, diff, revision, stash)
    mode: BufferMode,
    /// magit-buffer-diff-args of the diff / revision / stash buffer shown
    buf_args: Vec<String>,
    /// diff arguments of the status buffer
    status_args: Vec<String>,
    /// buffers below the one shown (the status buffer under a diff buffer)
    buried: Vec<SavedBuffer>,
    /// the arguments of the `d` transient while one of its suffixes runs
    /// (magit-diff-arguments when transient-current-command is magit-diff)
    transient_diff_args: Option<Vec<String>>,
    /// the same for the `l` transient (magit-log-arguments)
    transient_log_args: Option<Vec<String>>,
    /// C-c was typed (the prefix of C-c C-n)
    cc_prefix: bool,
    message: String,
    quit: bool,
    /// pending text-mode buffer shown instead of status (diff/log/show output)
    view_text: Option<(String, String)>, // (title, content)
    view_scroll: usize,
    /// directory ngit was started in (magit-git-command's default-directory)
    pub cwd: std::path::PathBuf,
    /// arguments of new diff / revision / stash buffers (the current value
    /// of magit-diff, initially magit-diff-mode's "--stat --no-ext-diff")
    pub diff_args: Vec<String>,
    /// arguments of new log buffers (the current value of magit-log-mode,
    /// initially "--graph -n256 --decorate")
    pub log_args: Vec<String>,
    /// answers used instead of reading from the terminal (tests)
    scripted: std::cell::RefCell<std::collections::VecDeque<String>>,
}

/// The major mode of a buffer: magit-status-mode, magit-diff-mode,
/// magit-revision-mode, magit-stash-mode, magit-log-mode,
/// magit-reflog-mode, magit-stashes-mode or magit-cherry-mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferMode {
    Status,
    Diff(DiffSpec),
    Revision { rev: String, files: Vec<String> },
    Stash { stash: String },
    Log { revs: Vec<String>, files: Vec<String> },
    Reflog { refname: String },
    Stashes { refname: String },
    Cherry { head: String, upstream: String },
}

impl BufferMode {
    /// Magit has one buffer per mode and repository.
    fn same_kind(&self, other: &BufferMode) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// A buffer that is not shown.
struct SavedBuffer {
    mode: BufferMode,
    args: Vec<String>,
    buf: StatusBuffer,
    vis: Visibility,
    cursor: usize,
    scroll: usize,
}

/// What a key does in the status buffer when no transient is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// q / ESC: leave the text view, or quit (magit-mode-bury-buffer)
    Bury,
    Refresh,
    /// next-line / previous-line (C-n, C-p, arrows)
    Down,
    Up,
    /// magit-section-forward / magit-section-backward (n, p)
    SectionForward,
    SectionBackward,
    /// magit-section-forward-sibling / -backward-sibling (M-n, M-p)
    SiblingForward,
    SiblingBackward,
    /// magit-section-toggle (TAB)
    Toggle,
    /// magit-section-cycle (C-TAB)
    Cycle,
    /// magit-section-cycle-global (S-TAB)
    CycleGlobal,
    /// magit-section-cycle-diffs (M-TAB)
    CycleDiffs,
    /// magit-section-show-level-N (1..4) and -N-all (M-1..M-4)
    ShowLevel(u8),
    ShowLevelAll(u8),
    /// set-mark-command (C-SPC, C-@)
    SetMark,
    /// keyboard-quit (C-g): deactivate the region
    KeyboardQuit,
    Visit,
    Menu(&'static str),
    Command(&'static str),
}

/// Choices cycled by `b C r` / `b C R` (magit-branch.<branch>.rebase, magit-pull.rebase).
pub const REBASE_CHOICES: &[&str] = &["true", "merges", "interactive", "false"];
/// Choices cycled by `b C a m` (magit-branch.autoSetupMerge).
pub const AUTOSETUP_MERGE_CHOICES: &[&str] = &["always", "true", "false"];
/// Choices cycled by `b C a r` (magit-branch.autoSetupRebase).
pub const AUTOSETUP_REBASE_CHOICES: &[&str] = &["always", "local", "remote", "never"];
/// magit-published-branches (default)
const PUBLISHED_BRANCHES: &[&str] = &["origin/master"];

/// Status buffer key map (magit-status-mode-map + magit-mode-map +
/// magit-section-mode-map). `k` is magit-discard and `j` is
/// magit-status-jump (Magit has no vi-style movement); `n` / `p` move
/// between sections, C-n / C-p and the arrows between lines.
pub fn status_binding(key: &Key) -> Option<Binding> {
    use Binding::*;
    if key.ctrl {
        return match key.code.as_str() {
            "n" => Some(Down),
            "p" => Some(Up),
            " " => Some(SetMark),
            "g" => Some(KeyboardQuit),
            "tab" => Some(Cycle),
            "j" | "enter" => Some(Command("diff-visit-worktree-file")),
            _ => None,
        };
    }
    Some(match key.code.as_str() {
        "q" | "esc" => Bury,
        "h" | "?" => Menu("dispatch"),
        "g" | "G" => Refresh,
        "n" => SectionForward,
        "p" => SectionBackward,
        "down" => Down,
        "up" => Up,
        "M-n" => SiblingForward,
        "M-p" => SiblingBackward,
        "j" => Command("jump"),
        "^" => Command("section-up"),
        "tab" => Toggle,
        "M-tab" => CycleDiffs,
        " " => Command("diff-show-or-scroll-up"),
        "backspace" => Command("diff-show-or-scroll-down"),
        "+" => Command("diff-more-context"),
        "-" => Command("diff-less-context"),
        "0" => Command("diff-default-context"),
        "backtab" => CycleGlobal,
        "1" => ShowLevel(1),
        "2" => ShowLevel(2),
        "3" => ShowLevel(3),
        "4" => ShowLevel(4),
        "M-1" => ShowLevelAll(1),
        "M-2" => ShowLevelAll(2),
        "M-3" => ShowLevelAll(3),
        "M-4" => ShowLevelAll(4),
        "enter" => Visit,
        "!" => Menu("run-menu"),
        "$" => Command("process-buffer"),
        "H" => Command("section-info"),
        "J" => Command("display-buffer"),
        "Y" => Command("cherries"),
        "%" => Menu("worktree-menu"),
        "s" => Command("stage"),
        "S" => Command("stage-all"),
        "u" => Command("unstage"),
        "U" => Command("unstage-all"),
        "k" => Command("discard"),
        "K" => Command("untrack"),
        "x" => Command("reset-quickly"),
        "a" => Command("apply"),
        "v" => Command("reverse"),
        "y" => Command("show-refs"),
        ":" | "Q" => Command("git-command"),
        "I" => Command("init"),
        "C" => Command("clone"),
        "e" => Command("ediff-dwim"),
        "D" => Menu("diff-refresh-menu"),
        "L" => Menu("log-refresh-menu"),
        "V" => Menu("revert-menu"),
        "c" => Menu("commit-menu"),
        "b" => Menu("branch-menu"),
        "B" => Menu("bisect-menu"),
        "E" => Menu("ediff-menu"),
        "f" => Menu("fetch-menu"),
        "F" => Menu("pull-menu"),
        "P" => Menu("push-menu"),
        "m" => Menu("merge-menu"),
        "M" => Menu("remote-menu"),
        "o" => Menu("submodule-menu"),
        "O" => Menu("subtree-menu"),
        "r" => Menu("rebase-menu"),
        "t" => Menu("tag-menu"),
        "T" => Menu("notes-menu"),
        "w" => Menu("patches-menu"),
        "W" => Menu("patch-menu"),
        "z" => Menu("stash-menu"),
        "Z" => Menu("worktree-menu"),
        "X" => Menu("reset-menu"),
        "i" => Menu("gitignore-menu"),
        "l" => Menu("log-menu"),
        "d" => Menu("diff-menu"),
        "A" => Menu("cherry-pick-menu"),
        _ => return None,
    })
}

impl App {
    pub fn new(git: Git) -> App {
        let cwd = git.toplevel.clone();
        let mut app = App {
            git,
            menus: registry(),
            stack: Vec::new(),
            pending: Vec::new(),
            buf: StatusBuffer::default(),
            vis: Visibility::new(),
            lines: Vec::new(),
            cursor: 0,
            scroll: 0,
            mark: None,
            mode: BufferMode::Status,
            buf_args: Vec::new(),
            status_args: vec!["--no-ext-diff".into(), "--ignore-submodules=none".into()],
            buried: Vec::new(),
            transient_diff_args: None,
            transient_log_args: None,
            cc_prefix: false,
            message: String::new(),
            quit: false,
            view_text: None,
            view_scroll: 0,
            cwd,
            diff_args: vec!["--stat".into(), "--no-ext-diff".into()],
            log_args: vec!["--graph".into(), "-n256".into(), "--decorate".into()],
            scripted: std::cell::RefCell::new(std::collections::VecDeque::new()),
        };
        app.refresh();
        // magit-status-initial-section '(1): the first section after the headers
        if let Some(i) = app.lines.iter().position(|l| l.heading && l.depth == 0 && l.kind != LineKind::Head) {
            app.cursor = i;
        }
        app
    }

    /// Diff arguments of the status buffer's diffs.
    fn status_diff_args(&self) -> Vec<String> {
        self.status_args
            .iter()
            .filter(|a| !matches!(a.as_str(), "--stat" | "--no-index" | "--color-moved"))
            .cloned()
            .collect()
    }

    /// magit-buffer-diff-args of the buffer shown.
    fn buffer_args(&self) -> Vec<String> {
        match self.mode {
            BufferMode::Status => self.status_diff_args(),
            _ => self.buf_args.clone(),
        }
    }

    /// magit-diff-type of the diff group GROUP in the buffer shown.
    fn diff_type<'a>(&'a self, group: &'a str) -> &'a str {
        match &self.mode {
            BufferMode::Status => group,
            BufferMode::Diff(spec) => spec.typ.as_str(),
            _ => "committed",
        }
    }

    /// The major mode of the buffer shown.
    pub fn mode(&self) -> &BufferMode {
        &self.mode
    }

    /// The header line of the buffer shown (empty for the status buffer).
    pub fn header_line(&self) -> &str {
        &self.buf.title
    }

    /// magit-refresh: rebuild the buffer, keeping point in the same section
    /// at the same offset (or at the same line when that section is gone).
    pub fn refresh(&mut self) {
        let anchor = self.anchor();
        self.buf = self.build_buffer();
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

    /// The refresh function of the buffer's mode.
    fn build_buffer(&self) -> StatusBuffer {
        match &self.mode {
            BufferMode::Status => {
                let st = Status::load(&self.git);
                build_status(&self.git, &st, &self.status_diff_args())
            }
            BufferMode::Diff(spec) => build_diff(&self.git, spec, &self.buf_args),
            BufferMode::Revision { rev, files } => build_revision(&self.git, rev, &self.buf_args, files),
            BufferMode::Stash { stash } => build_stash(&self.git, stash, &self.buf_args),
            BufferMode::Log { revs, files } => build_log(&self.git, revs, &self.buf_args, files),
            BufferMode::Reflog { refname } => build_reflog(&self.git, refname, &self.buf_args),
            BufferMode::Stashes { refname } => build_stashes(&self.git, refname),
            BufferMode::Cherry { head, upstream } => build_cherry(&self.git, head, upstream),
        }
    }

    /// magit-setup-buffer: show a buffer in MODE with ARGS, reusing the
    /// buffer of that mode (or burying the buffer shown).
    fn setup_buffer(&mut self, mode: BufferMode, args: Vec<String>) {
        self.view_text = None;
        let same = self.mode == mode;
        if !self.mode.same_kind(&mode) {
            let saved = SavedBuffer {
                mode: std::mem::replace(&mut self.mode, BufferMode::Status),
                args: std::mem::take(&mut self.buf_args),
                buf: std::mem::take(&mut self.buf),
                vis: std::mem::take(&mut self.vis),
                cursor: self.cursor,
                scroll: self.scroll,
            };
            self.buried.retain(|b| !b.mode.same_kind(&mode));
            self.buried.push(saved);
        } else if !same {
            self.vis.clear();
        }
        if !same {
            self.buf = StatusBuffer::default();
            self.lines.clear();
            self.cursor = 0;
            self.scroll = 0;
        }
        self.mode = mode;
        self.buf_args = args;
        self.refresh();
    }

    /// magit-mode-bury-buffer (q): leave the text view, else show the
    /// buffer below (refreshed), else quit.
    fn bury_buffer(&mut self) {
        if self.view_text.take().is_some() {
            self.set_msg("");
            return;
        }
        let Some(b) = self.buried.pop() else {
            self.quit = true;
            return;
        };
        self.mode = b.mode;
        self.buf_args = b.args;
        self.buf = b.buf;
        self.vis = b.vis;
        self.lines = self.buf.visible(&self.vis);
        self.cursor = b.cursor.min(self.lines.len().saturating_sub(1));
        self.scroll = b.scroll;
        self.refresh();
    }

    /// (ident of the section at point, offset from its heading), plus the
    /// visible index as fallback.
    fn anchor(&self) -> (Option<(String, usize)>, usize) {
        if self.lines.is_empty() || self.buf.sections.is_empty() {
            return (None, self.cursor);
        }
        let p = self.point();
        let sec = &self.buf.sections[self.buf.section_at(p)];
        (Some((sec.ident.clone(), p.saturating_sub(sec.start))), self.cursor)
    }

    /// Position (index into `buf.all`) of point.
    fn point(&self) -> usize {
        self.lines.get(self.cursor).map_or(0, |l| l.pos)
    }

    /// Visible index of buffer position P; when P is inside a hidden body,
    /// the heading of the outermost hidden section around it.
    fn visible_index_near(&self, p: usize) -> usize {
        if let Ok(i) = self.lines.binary_search_by_key(&p, |l| l.pos) {
            return i;
        }
        let mut s = self.buf.section_at(p);
        if self.buf.sections.get(s).is_some_and(|x| x.start == p) {
            s = self.buf.sections[s].parent.unwrap_or(0);
        }
        let mut target = None;
        while s != 0 {
            if self.hidden(s) {
                target = Some(self.buf.sections[s].start);
            }
            s = self.buf.sections[s].parent.unwrap_or(0);
        }
        if let Some(Ok(i)) = target.map(|t| self.lines.binary_search_by_key(&t, |l| l.pos)) {
            return i;
        }
        self.lines.iter().position(|l| l.pos > p).unwrap_or(self.lines.len().saturating_sub(1))
    }

    /// Re-show the buffer after a visibility change with point at P.
    fn relayout_at(&mut self, p: usize) {
        self.lines = self.buf.visible(&self.vis);
        self.cursor = self.visible_index_near(p);
    }

    /// Visible indices covered by the active region (mark to point).
    fn region_bounds(&self) -> Option<(usize, usize)> {
        let m = self.visible_index_near(self.mark?);
        Some((m.min(self.cursor), m.max(self.cursor)))
    }

    // ------------------------------------------------------------ rendering

    fn draw(&mut self) {
        let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));
        let (w, h) = (w as usize, h as usize);
        let mut rows: Vec<String> = Vec::new();

        // transient popup (shown in a window at the bottom, like transient)
        let mut popup: Vec<String> = Vec::new();
        if let Some((name, active)) = self.stack.last() {
            if let Some(menu) = self.menus.get(name) {
                popup.push(String::new());
                popup.push(menu.title.clone());
                popup.extend(menu.render(active).lines().map(|l| format!("  {l}")));
                popup.push("  q/C-g quit popup".into());
            }
        }
        let body = h.saturating_sub(2 + popup.len()).max(1);

        if let Some((title, content)) = &self.view_text {
            rows.push(format!("--- {title} ---"));
            let total = content.lines().count();
            self.view_scroll = self.view_scroll.min(total.saturating_sub(1));
            rows.extend(content.lines().skip(self.view_scroll).take(body.saturating_sub(1)).map(str::to_string));
        } else {
            let mut body = body;
            if self.mode != BufferMode::Status {
                rows.push(self.buf.title.clone());
                body = body.saturating_sub(1).max(1);
            }
            if self.cursor < self.scroll {
                self.scroll = self.cursor;
            } else if self.cursor >= self.scroll + body {
                self.scroll = self.cursor + 1 - body;
            }
            let region = self.region_bounds();
            for (i, line) in self.lines.iter().enumerate().skip(self.scroll).take(body) {
                let cur = if i == self.cursor {
                    '>'
                } else if region.is_some_and(|(a, b)| i >= a && i <= b) {
                    '|'
                } else {
                    ' '
                };
                let fringe = match line.hidden {
                    Some(true) => '▸',
                    Some(false) => '▾',
                    None => ' ',
                };
                let row = format!("{cur}{fringe}{}", line.text);
                let mw = self.buf.margin_width;
                if mw > 0 && w > mw {
                    // the right margin (magit-log-margin)
                    let text: String = row.chars().take(w - mw).collect();
                    let pad = (w - mw).saturating_sub(text.chars().count());
                    rows.push(format!("{text}{}{}", " ".repeat(pad), line.margin));
                } else {
                    rows.push(row);
                }
            }
        }
        while rows.len() < body {
            rows.push(String::new());
        }
        rows.extend(popup);
        let branch = self.git.head_desc();
        rows.push(format!("[ngit:{branch}] {}", self.message));
        rows.push("(h/? dispatch, g refresh, q quit, TAB toggle, RET visit, n/p section, C-n/C-p line)".into());

        let mut out = stdout();
        let _ = execute!(out, Clear(ClearType::All));
        for (y, r) in rows.iter().take(h).enumerate() {
            let r: String = r.chars().take(w).collect();
            let _ = execute!(out, crossterm::cursor::MoveTo(0, y as u16), crossterm::style::Print(r));
        }
        let _ = out.flush();
    }

    // ------------------------------------------------------------ input helpers

    /// Answers consumed by prompts instead of reading the terminal (tests).
    pub fn script_answers(&mut self, answers: &[&str]) {
        self.scripted.borrow_mut().extend(answers.iter().map(|s| s.to_string()));
    }

    fn read_line_prompt(&self, prompt: &str, default: Option<&str>) -> Result<Option<String>> {
        let scripted = self.scripted.borrow_mut().pop_front();
        let buf = if let Some(ans) = scripted {
            ans
        } else {
            // Leave raw mode temporarily to read a normal line.
            let _ = disable_raw_mode();
            let mut out = stdout();
            let (_, h) = crossterm::terminal::size().unwrap_or((80, 24));
            let _ = execute!(
                out,
                crossterm::cursor::MoveTo(0, h.saturating_sub(1)),
                Clear(ClearType::CurrentLine),
                crossterm::style::Print(prompt),
                crossterm::style::Print(
                    default
                        .map(|d| format!(" (default: {d}): "))
                        .unwrap_or_else(|| ": ".into())
                )
            );
            let _ = out.flush();
            let mut buf = String::new();
            let n = std::io::stdin().read_line(&mut buf)?;
            let _ = enable_raw_mode();
            if n == 0 {
                return Ok(None); // EOF / C-d
            }
            buf
        };
        let mut s = buf.trim().to_string();
        if s.is_empty() {
            if let Some(d) = default {
                s = d.to_string();
            }
        }
        if s.is_empty() {
            return Ok(None);
        }
        Ok(Some(s))
    }

    fn yes_no(&self, prompt: &str) -> Result<bool> {
        loop {
            let Some(ans) = self.read_line_prompt(&format!("{prompt} (y/n)"), Some("n"))? else {
                return Ok(false);
            };
            match ans.to_lowercase().as_str() {
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                _ => continue,
            }
        }
    }

    fn pick<'a>(&mut self, prompt: &str, choices: &'a [String], default: Option<&str>) -> Result<Option<&'a String>> {
        if choices.is_empty() {
            self.message = "no candidates".into();
            return Ok(None);
        }
        // simple completion: substring match or exact
        let Some(inp) = self.read_line_prompt(
            prompt,
            default.or(Some(&choices[0])),
        )? else {
            return Ok(None);
        };
        if let Some(exact) = choices.iter().find(|c| c.as_str() == inp) {
            return Ok(Some(exact));
        }
        let hits: Vec<&String> = choices.iter().filter(|c| c.contains(inp.as_str())).collect();
        if hits.len() == 1 {
            Ok(Some(hits[0]))
        } else if hits.is_empty() {
            self.message = "no match".into();
            Ok(None)
        } else {
            Ok(Some(hits[0]))
        }
    }

    fn set_msg(&mut self, m: &str) {
        self.message = m.to_string();
    }

    // ------------------------------------------------------------ selection helpers

    fn current_line(&self) -> Option<&BufferLine> {
        self.lines.get(self.cursor)
    }

    /// Pick one of the last 30 commits; returns the abbreviated hash.
    fn pick_recent_commit(&mut self, prompt: &str) -> Result<Option<String>> {
        let revs: Vec<String> = self
            .git
            .log(None, 30)?
            .iter()
            .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
            .filter(|r| !r.is_empty())
            .collect();
        Ok(self.pick(prompt, &revs, None)?.cloned())
    }

    /// Read a commit, defaulting to the commit at point
    /// (magit-read-other-branch-or-commit).
    fn read_commit(&mut self, prompt: &str) -> Result<Option<String>> {
        match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Commit { hash, .. }) => {
                let short = self.git.rev_short(&hash);
                self.read_line_prompt(prompt, Some(&short))
            }
            _ => self.pick_recent_commit(prompt),
        }
    }

    /// (section, path) of the file the line at point belongs to.
    fn current_file(&self) -> Option<(String, String)> {
        match &self.current_line()?.kind {
            LineKind::File { section, path }
            | LineKind::Hunk { section, path, .. }
            | LineKind::DiffLine { section, path, .. } => Some((section.clone(), path.clone())),
            _ => None,
        }
    }

    /// Files of the untracked / unstaged / staged section around point.
    fn files_in_current_section(&self) -> Vec<String> {
        let Some(cur) = self.current_line() else { return vec![] };
        let section = match &cur.kind {
            LineKind::File { section, .. }
            | LineKind::Hunk { section, .. }
            | LineKind::DiffLine { section, .. }
            | LineKind::SectionHeader { section, .. } => section.clone(),
            _ => return vec![],
        };
        self.buf
            .all
            .iter()
            .filter_map(|l| match &l.kind {
                LineKind::File { section: s, path } if *s == section => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    // ------------------------------------------------------------ event loop

    pub fn run(&mut self) -> Result<()> {
        let _ = enable_raw_mode();
        let _ = execute!(stdout(), EnterAlternateScreen);
        self.draw();
        loop {
            if self.quit {
                break;
            }
            match event::read()? {
                Event::Key(ev) => {
                    if ev.kind == crossterm::event::KeyEventKind::Release {
                        continue;
                    }
                    let key = Key::from_event(&ev);
                    if let Err(e) = self.handle_key(key) {
                        self.set_msg(&format!("{e}"));
                    }
                }
                _ => {}
            }
            self.draw();
        }
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        Ok(())
    }

    fn handle_key(&mut self, key: Key) -> Result<()> {
        // transient popup takes precedence
        if let Some((name, active)) = self.stack.last().cloned() {
            if let Some(menu) = self.menus.get(&name).cloned() {
                return self.handle_transient_key(key, menu, active);
            }
        }
        if std::mem::take(&mut self.cc_prefix) {
            match (&self.mode, key.ctrl, key.code.as_str()) {
                (BufferMode::Log { .. }, true, "n") => self.log_move_to_parent(),
                _ => self.set_msg(&format!("C-c {} is undefined", key.label())),
            }
            return Ok(());
        }
        if key.ctrl && key.code == "c" {
            self.cc_prefix = true;
            self.set_msg("C-c-");
            return Ok(());
        }
        // magit-log-mode-map (also the parent of magit-reflog-mode-map and
        // so of magit-stashes-mode-map)
        if matches!(self.mode, BufferMode::Log { .. } | BufferMode::Reflog { .. } | BufferMode::Stashes { .. })
            && self.view_text.is_none()
            && !key.ctrl
        {
            let limit = match key.code.as_str() {
                "+" => Some(Limit::Double),
                "-" => Some(Limit::Half),
                "=" => Some(Limit::Toggle),
                "j" => return self.log_move_to_revision(),
                _ => None,
            };
            if let Some(limit) = limit {
                self.log_set_commit_limit(limit);
                return Ok(());
            }
        }
        // status buffer bindings (magit-status-mode-map)
        let Some(binding) = status_binding(&key) else {
            return Ok(());
        };
        match binding {
            Binding::Bury => self.bury_buffer(),
            Binding::Refresh => {
                self.refresh();
                self.set_msg("refreshed");
            }
            // the text view (diff / log / show output) scrolls by line
            Binding::Down | Binding::SectionForward if self.view_text.is_some() => self.view_scroll += 1,
            Binding::Up | Binding::SectionBackward if self.view_text.is_some() => {
                self.view_scroll = self.view_scroll.saturating_sub(1)
            }
            Binding::Down => self.forward_line(1),
            Binding::Up => self.forward_line(-1),
            Binding::SectionForward => self.section_forward(),
            Binding::SectionBackward => self.section_backward(),
            Binding::SiblingForward => self.sibling_forward(),
            Binding::SiblingBackward => self.sibling_backward(),
            Binding::Toggle => self.toggle_section(),
            Binding::Cycle => self.cycle_section(),
            Binding::CycleGlobal => self.cycle_global(),
            Binding::CycleDiffs => self.cycle_diffs(),
            Binding::ShowLevel(n) => self.show_level(i32::from(n)),
            Binding::ShowLevelAll(n) => self.show_level(-i32::from(n)),
            Binding::SetMark => {
                self.mark = Some(self.point());
                self.set_msg("Mark set");
            }
            Binding::KeyboardQuit => {
                self.mark = None;
                self.set_msg("Quit");
            }
            Binding::Visit => self.visit_thing()?,
            Binding::Menu(m) => self.open_menu(m),
            Binding::Command(c) => self.run_command(c, &BTreeMap::new())?,
        }
        Ok(())
    }

    fn open_menu(&mut self, name: &str) {
        let name = resolve_menu(name, self.repo_state());
        if !self.menus.contains_key(&name) {
            return;
        }
        if name == "diff-refresh-menu" {
            let menu = diff_refresh_menu(self.mode != BufferMode::Status);
            self.menus.insert(name.clone(), menu);
        }
        if name == "log-refresh-menu" {
            let kind = match self.mode {
                BufferMode::Reflog { .. } | BufferMode::Stashes { .. } => "reflog",
                BufferMode::Cherry { .. } => "cherry",
                _ => "",
            };
            if !kind.is_empty() {
                self.set_msg(&format!("Cannot change log arguments in {kind} buffers"));
                return;
            }
        }
        // `d` starts with the arguments of the diff buffer shown, else the
        // current value of magit-diff-mode; `D` with the buffer's arguments
        let shortlog_default = ["--numbered".to_string(), "--summary".to_string()];
        let preset: &[String] = match (name.as_str(), &self.mode) {
            ("diff-menu", BufferMode::Diff(_)) => &self.buf_args,
            ("diff-menu", _) => &self.diff_args,
            ("diff-refresh-menu", BufferMode::Status) => &self.status_args,
            ("diff-refresh-menu", _) => &self.buf_args,
            ("log-menu" | "log-refresh-menu", BufferMode::Log { .. }) => &self.buf_args,
            ("log-menu" | "log-refresh-menu", _) => &self.log_args,
            ("shortlog-menu", _) => &shortlog_default,
            _ => &[],
        };
        let active: BTreeMap<String, bool> = preset.iter().map(|a| (a.clone(), true)).collect();
        self.pending.clear();
        self.stack.push((name, active));
    }

    fn repo_state(&self) -> RepoState {
        RepoState {
            merge: self.git.merge_in_progress(),
            rebase: self.git.rebase_in_progress(),
            sequencer: self.git.pick_or_revert_in_progress(),
            am: self.git.am_in_progress(),
            bisect: self.git.bisect_in_progress(),
        }
    }

    /// Name of the transient currently shown, if any.
    pub fn current_menu(&self) -> Option<&str> {
        self.stack.last().map(|(n, _)| n.as_str())
    }

    /// Feed one key (Emacs notation, e.g. "h", "C-g", "<tab>") as if typed.
    /// Only keys that do not prompt should be used outside the terminal.
    pub fn press(&mut self, label: &str) -> Result<()> {
        self.handle_key(super::keys::key_from_label(label))
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// The visible lines of the status buffer.
    pub fn visible_lines(&self) -> &[BufferLine] {
        &self.lines
    }

    /// Text of the line at point.
    pub fn point_text(&self) -> &str {
        self.current_line().map_or("", |l| l.text.as_str())
    }

    /// Title and content of the text view (diff / log / file), if shown.
    pub fn text_view(&self) -> Option<(&str, &str)> {
        self.view_text.as_ref().map(|(t, c)| (t.as_str(), c.as_str()))
    }

    /// First line of the text view shown on screen.
    pub fn text_view_scroll(&self) -> usize {
        self.view_scroll
    }

    /// Switches enabled in the transient currently shown.
    pub fn active_switches(&self) -> Vec<String> {
        self.stack
            .last()
            .map(|(_, a)| a.iter().filter(|(_, on)| **on).map(|(k, _)| k.clone()).collect())
            .unwrap_or_default()
    }

    /// Give the terminal to a program that needs it (the user's editor for a
    /// rebase todo list or commit message), then take it back.
    fn with_terminal<T>(&mut self, f: impl FnOnce(&Git) -> T) -> T {
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        let r = f(&self.git);
        let _ = enable_raw_mode();
        let _ = execute!(stdout(), EnterAlternateScreen);
        r
    }

    fn handle_transient_key(
        &mut self,
        key: Key,
        menu: Menu,
        mut active: BTreeMap<String, bool>,
    ) -> Result<()> {
        if key.code == "q" || key.code == "esc" || (key.ctrl && key.code == "g") {
            self.pending.clear();
            self.stack.pop();
            return Ok(());
        }
        let mut typed = std::mem::take(&mut self.pending);
        typed.push(key);
        let entry: MenuEntry = match menu.lookup(&typed) {
            Lookup::Found(e) => e.clone(),
            Lookup::Prefix => {
                self.pending = typed;
                return Ok(());
            }
            Lookup::None => {
                let keys: Vec<String> = typed.iter().map(Key::label).collect();
                self.set_msg(&format!("{} is undefined", keys.join(" ")));
                return Ok(());
            }
        };
        match entry.action.clone() {
            Action::Switch(sw) => {
                let on = *active.get(&sw).unwrap_or(&false);
                active.insert(sw, !on);
                if let Some(top) = self.stack.last_mut() {
                    top.1 = active;
                }
            }
            Action::Option { arg, reader } => {
                let old = entry.action.value_in(&active);
                let is_choice = matches!(reader, Reader::Choices(_));
                let new = if old.is_some() && !is_choice {
                    None
                } else {
                    match self.read_option(&arg, &reader)? {
                        Some(v) => v,
                        None => return Ok(()),
                    }
                };
                if let Some(o) = old {
                    active.remove(&o);
                }
                if let Some(v) = new {
                    active.insert(format!("{arg}{v}"), true);
                }
                if let Some(top) = self.stack.last_mut() {
                    top.1 = active;
                }
            }
            Action::Switches(args) => {
                // transient-switches: cycle to the next argument, then none
                let old = entry.action.value_in(&active);
                let next = match &old {
                    None => args.first().cloned(),
                    Some(o) => args.iter().skip_while(|a| *a != o).nth(1).cloned(),
                };
                if let Some(o) = old {
                    active.remove(&o);
                }
                if let Some(n) = next {
                    active.insert(n, true);
                }
                if let Some(top) = self.stack.last_mut() {
                    top.1 = active;
                }
            }
            Action::Submenu(name) => {
                self.stack.pop();
                self.open_menu(&name);
            }
            Action::Command(cmd) => {
                self.stack.pop();
                if menu.name == "diff-menu" {
                    self.transient_diff_args = Some(self.args_from(&active));
                }
                if menu.name == "log-menu" {
                    self.transient_log_args = Some(self.args_from(&active));
                }
                let r = self.run_command(&cmd, &active);
                self.transient_diff_args = None;
                self.transient_log_args = None;
                r?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ commands

    /// Read the value of the option ARG (transient-infix-read). None: the
    /// user quit (nothing changes); Some(None): unset; Some(Some(v)): set
    /// to ARG followed by V.
    fn read_option(&mut self, arg: &str, reader: &Reader) -> Result<Option<Option<String>>> {
        let value = match reader {
            Reader::Choices(choices) => {
                let prompt = format!("{arg} [unspecified, {}]", choices.join(", "));
                let Some(v) = self.read_line_prompt(&prompt, Some("unspecified"))? else {
                    return Ok(None);
                };
                if "unspecified".starts_with(v.as_str()) {
                    return Ok(Some(None));
                }
                match choices.iter().find(|c| c.starts_with(v.as_str())) {
                    Some(c) => c.clone(),
                    None => {
                        self.set_msg(&format!("{v} is not one of {}", choices.join(", ")));
                        return Ok(None);
                    }
                }
            }
            Reader::FileTrace => {
                let Some(file) = self.read_line_prompt("File", self.current_file().map(|(_, p)| p).as_deref())? else {
                    return Ok(None);
                };
                let Some(trace) = self.read_line_prompt("Trace", None)? else {
                    return Ok(None);
                };
                format!("{trace}:{file}")
            }
            _ => {
                let Some(v) = self.read_line_prompt(arg, None)? else {
                    return Ok(None);
                };
                let valid = match reader {
                    Reader::Number { zero } => v.parse::<u64>().is_ok_and(|n| *zero || n > 0),
                    Reader::Similarity => {
                        let digits = v.strip_suffix('%').unwrap_or(&v);
                        !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
                    }
                    _ => true,
                };
                if !valid {
                    let what = match reader {
                        Reader::Number { zero: true } => "a non-negative integer".to_string(),
                        Reader::Number { zero: false } => "a positive integer".to_string(),
                        _ => "a percentage ending in %, or a fraction, omitting the implied leading \"0.\"".to_string(),
                    };
                    self.set_msg(&format!("Please enter {what}"));
                    return Ok(None);
                }
                v
            }
        };
        Ok(Some(Some(value)))
    }
    fn args_from(&self, active: &BTreeMap<String, bool>) -> Vec<String> {
        active
            .iter()
            .filter(|(_, on)| **on)
            .map(|(k, _)| k.clone())
            .collect()
    }

    fn untracked_from(&self, active: &BTreeMap<String, bool>) -> Untracked {
        if *active.get("--all").unwrap_or(&false) {
            Untracked::All
        } else if *active.get("--include-untracked").unwrap_or(&false) {
            Untracked::Include
        } else {
            Untracked::No
        }
    }

    #[allow(clippy::too_many_lines)]
    fn run_command(&mut self, cmd: &str, active: &BTreeMap<String, bool>) -> Result<()> {
        // a menu used as a command (e.g. dispatch "b Branch") opens that menu
        if self.menus.contains_key(cmd) {
            self.open_menu(cmd);
            return Ok(());
        }
        let args = self.args_from(active);
        macro_rules! g {
            () => {
                &self.git
            };
        }
        match cmd {
            "refresh" => {
                self.refresh();
                self.set_msg("refreshed");
            }
            "quit" => self.bury_buffer(),
            "toggle-section" => self.toggle_section(),
            "visit" => self.visit_thing()?,
            "jump" => self.jump_section(),
            "help" => {
                self.view_text = Some((
                    "Key bindings".into(),
                    "h/? dispatch popup — all commands live there.\n\
                     n/p move, j jump to section, ^ section up\n\
                     s stage, u unstage, S stage all, U unstage all\n\
                     k discard, K untrack, x reset quickly, a apply, v reverse\n\
                     c commit, b branch, z stash, P push, F pull, f fetch\n\
                     m merge, r rebase, X reset, t tag, M remote\n\
                     l log, d diff, L/D change log/diff arguments, A cherry-pick, V revert\n\
                     w apply patches, W patch, o submodule, $ git process log\n\
                     g refresh, TAB toggle section, RET visit, q quit"
                        .into(),
                ));
            }

            // -------- staging
            "stage" => self.stage_cmd()?,
            "stage-all" => self.stage_modified()?,
            "unstage" => self.unstage_cmd()?,
            "unstage-all" => self.unstage_all()?,
            "discard" => self.discard_cmd()?,
            "untrack" => {
                if let Some((_, path)) = self.pick_current_file_or_section() {
                    g!().untrack(&path)?;
                    self.set_msg("untracked");
                    self.refresh();
                }
            }
            "diff-show-or-scroll-up" => self.show_or_scroll(true)?,
            "diff-show-or-scroll-down" => self.show_or_scroll(false)?,
            "diff-more-context" => self.set_diff_context(|n| Some(n + 1)),
            "diff-less-context" => self.set_diff_context(|n| Some(n.saturating_sub(1))),
            "diff-default-context" => self.set_diff_context(|_| None),

            // -------- commit menu
            "commit-create" => {
                // -C reuses a message as is, -c edits it in the editor
                if args.iter().any(|a| a.starts_with("--reedit-message=")) {
                    self.with_terminal(|g| g.commit(None, &args))?;
                } else {
                    let msg = if args.iter().any(|a| a.starts_with("--reuse-message=")) {
                        None
                    } else {
                        self.read_line_prompt("Commit message", None)?
                    };
                    g!().commit(msg.as_deref(), &args)?;
                }
                self.set_msg("committed");
                self.refresh();
            }
            "commit-extend" => {
                g!().commit_extend(&args)?;
                self.set_msg("extended HEAD commit");
                self.refresh();
            }
            "commit-amend" => {
                let msg = self.read_line_prompt("Amend message (empty keeps)", Some(""))?;
                let m = msg.filter(|s| !s.is_empty());
                g!().commit_amend(m.as_deref(), &args)?;
                self.set_msg("amended HEAD commit");
                self.refresh();
            }
            "commit-reword" => {
                let msg = self.read_line_prompt("New message", None)?;
                if let Some(m) = msg {
                    g!().commit_reword(&m)?;
                    self.set_msg("reworded HEAD commit");
                    self.refresh();
                }
            }
            "commit-fixup" => {
                let branches = g!().branches();
                if let Some(rev) = self.pick("Fixup commit (branch or hash)", &branches, None)? {
                    g!().commit_fixup(rev)?;
                    self.set_msg("created fixup commit");
                    self.refresh();
                }
            }
            "commit-squash" => {
                let branches = g!().branches();
                if let Some(rev) = self.pick("Squash into (branch or hash)", &branches, None)? {
                    g!().commit_squash(rev)?;
                    self.set_msg("created squash commit");
                    self.refresh();
                }
            }
            "commit-instant-fixup" => {
                let branches = g!().branches();
                if let Some(rev) = self.pick("Instant fixup into", &branches, None)? {
                    g!().commit_instant_fixup(rev)?;
                    self.set_msg("instant fixup done");
                    self.refresh();
                }
            }
            "commit-instant-squash" => {
                let branches = g!().branches();
                if let Some(rev) = self.pick("Instant squash into", &branches, None)? {
                    g!().commit_instant_squash(rev)?;
                    self.set_msg("instant squash done");
                    self.refresh();
                }
            }

            // -------- branch menu
            "checkout" | "branch-checkout" => {
                let bs = g!().branches();
                if let Some(b) = self.pick("Checkout branch/revision", &bs, None)? {
                    g!().checkout(b)?;
                    self.set_msg(&format!("checked out {b}"));
                    self.refresh();
                }
            }
            "branch-and-checkout" => {
                let Some(name) = self.read_line_prompt("New branch name", None)? else {
                    return Ok(());
                };
                let bs = g!().branches();
                let start = self.pick("Start from (empty = HEAD)", &bs, Some(""))?;
                g!().branch_and_checkout(&name, start.map(|s| s.as_str()))?;
                self.set_msg(&format!("created and checked out {name}"));
                self.refresh();
            }
            "branch-create" => {
                let Some(name) = self.read_line_prompt("New branch name", None)? else {
                    return Ok(());
                };
                let bs = g!().branches();
                let start = self.pick("Start from (empty = HEAD)", &bs, Some(""))?;
                g!().branch_create(&name, start.map(|s| s.as_str()))?;
                self.set_msg(&format!("created branch {name}"));
                self.refresh();
            }
            "branch-spinoff" => {
                let Some(name) = self.read_line_prompt("Spin off to branch", None)? else {
                    return Ok(());
                };
                g!().branch_spinoff(&name)?;
                self.set_msg(&format!("spin-off {name}"));
                self.refresh();
            }
            "branch-spinout" => {
                let Some(name) = self.read_line_prompt("Spin out branch", None)? else {
                    return Ok(());
                };
                g!().branch_spinout(&name)?;
                self.set_msg(&format!("spin-out {name}"));
                self.refresh();
            }
            "branch-rename" => {
                let bs = g!().branches();
                if let Some(old) = self.pick("Rename branch", &bs, None)? {
                    if let Some(new) = self.read_line_prompt("New name", Some(old))? {
                        g!().branch_rename(old, &new)?;
                        self.set_msg("renamed");
                        self.refresh();
                    }
                }
            }
            "branch-reset" => {
                let bs = g!().branches();
                if let Some(b) = self.pick("Reset branch", &bs, None)? {
                    if let Some(rev) = self.read_line_prompt("Reset to", Some("HEAD"))? {
                        g!().git_interactive(&["branch", "-f", b, &rev])?;
                        if g!().current_branch().as_deref() == Some(b.as_str()) {
                            let _ = g!().git_interactive(&["reset", "--keep", &rev]);
                        }
                        self.set_msg("branch reset");
                        self.refresh();
                    }
                }
            }
            "branch-delete" => {
                let bs = g!().branches();
                if let Some(b) = self.pick("Delete branch", &bs, None)? {
                    g!().branch_delete(b)?;
                    self.set_msg(&format!("deleted {b}"));
                    self.refresh();
                }
            }

            // -------- remote menu
            "remote-add" => {
                let Some(name) = self.read_line_prompt("Remote name", None)? else {
                    return Ok(());
                };
                let Some(url) = self.read_line_prompt("Remote url", None)? else {
                    return Ok(());
                };
                let fetch = *active.get("-f").unwrap_or(&false);
                g!().remote_add(&name, &url, fetch)?;
                self.set_msg(&format!("added remote {name}"));
                self.refresh();
            }
            "remote-rename" => {
                let rs = g!().remotes();
                if let Some(old) = self.pick("Rename remote", &rs, None)? {
                    if let Some(new) = self.read_line_prompt("New name", Some(old))? {
                        g!().remote_rename(old, &new)?;
                        self.set_msg("renamed remote");
                        self.refresh();
                    }
                }
            }
            "remote-remove" => {
                let rs = g!().remotes();
                if let Some(r) = self.pick("Remove remote", &rs, None)? {
                    g!().remote_remove(r)?;
                    self.set_msg("removed remote");
                    self.refresh();
                }
            }
            "remote-prune" => {
                let rs = g!().remotes();
                if let Some(r) = self.pick("Prune remote", &rs, None)? {
                    g!().remote_prune(r)?;
                    self.set_msg("pruned");
                    self.refresh();
                }
            }

            // -------- fetch/pull/push (upstream = branch.<b>.remote / branch.<b>.merge)
            "fetch-upstream" => {
                match g!().fetch_upstream(&args) {
                    Ok(r) => self.set_msg(&format!("fetched {r}")),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "fetch-all" => {
                let prune = args.iter().any(|a| a == "--prune");
                g!().fetch_all(prune)?;
                self.set_msg("fetched all remotes");
                self.refresh();
            }
            "fetch-other" | "fetch-branch" => {
                let remotes = g!().remotes();
                let Some(r) = self.pick("Fetch from remote", &remotes, None)?.cloned() else {
                    return Ok(());
                };
                if cmd == "fetch-other" {
                    g!().fetch(&r, &args)?;
                    self.set_msg(&format!("fetched {r}"));
                } else {
                    let Some(b) = self.read_line_prompt(&format!("Fetch branch from {r}"), None)? else {
                        return Ok(());
                    };
                    g!().fetch_branch(&r, &b, &args)?;
                    self.set_msg(&format!("fetched {r} {b}"));
                }
                self.refresh();
            }
            "pull-upstream" => {
                match g!().pull_upstream(&args) {
                    Ok(()) => self.set_msg("pulled"),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "pull-other" => {
                let mut choices = g!().remote_branches();
                choices.extend(g!().branches());
                let Some(src) = self.pick("Pull from", &choices, None)?.cloned() else {
                    return Ok(());
                };
                let (remote, branch) = g!().tracked_of(&src);
                g!().pull(&remote, &branch, &args)?;
                self.set_msg(&format!("pulled {src}"));
                self.refresh();
            }
            "push-upstream" => {
                match g!().push_upstream(&args) {
                    Ok(()) => self.set_msg("pushed"),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "push-other" | "push-other-branch" => {
                let src = if cmd == "push-other" {
                    match g!().current_branch() {
                        Some(b) => b,
                        None => {
                            self.set_msg("cannot push: detached HEAD");
                            return Ok(());
                        }
                    }
                } else {
                    let bs = g!().branches();
                    let Some(b) = self.pick("Push branch", &bs, None)?.cloned() else {
                        return Ok(());
                    };
                    b
                };
                let default = g!().remotes().first().map(|r| format!("{r}/{src}"));
                let Some(target) =
                    self.read_line_prompt(&format!("Push {src} to (remote/branch)"), default.as_deref())?
                else {
                    return Ok(());
                };
                let (remote, dst) = g!().tracked_of(&target);
                if remote == "." {
                    self.set_msg(&format!("not a remote branch: {target}"));
                    return Ok(());
                }
                g!().push_refspec(&remote, &src, &dst, &args)?;
                self.set_msg(&format!("pushed {src} to {target}"));
                self.refresh();
            }
            "push-tag" => {
                let remotes = g!().remotes();
                let tags = g!().tags();
                if let (Some(r), Some(t)) = (
                    self.pick("Push tag to remote", &remotes, None)?,
                    self.pick("Tag", &tags, None)?,
                ) {
                    g!().push_tag(r, t)?;
                    self.set_msg("tag pushed");
                }
                self.refresh();
            }
            "push-pushremote" => {
                match g!().push_pushremote(&args) {
                    Ok(r) => self.set_msg(&format!("pushed to {r}")),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "push-tags" | "push-matching" | "push-refspecs" => {
                let remotes = g!().remotes();
                let default = g!()
                    .current_branch()
                    .and_then(|b| g!().upstream_of(&b))
                    .map(|(r, _)| r)
                    .filter(|r| r != ".");
                let Some(remote) = self.pick("Push to remote", &remotes, default.as_deref())?.cloned() else {
                    return Ok(());
                };
                let res = match cmd {
                    "push-tags" => g!().push_tags(&remote, &args),
                    "push-matching" => g!().push_matching(&remote, &args),
                    _ => {
                        let Some(specs) = self.read_line_prompt("Refspecs (space separated)", None)? else {
                            return Ok(());
                        };
                        let specs: Vec<String> = specs.split_whitespace().map(str::to_string).collect();
                        g!().push_refspecs(&remote, &specs, &args)
                    }
                };
                match res {
                    Ok(()) => self.set_msg(&format!("pushed to {remote}")),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }

            // -------- merge / rebase
            "merge-plain" | "merge-nocommit" | "merge-squash" | "merge-editmsg" | "merge-preview" => {
                let mut bs = g!().branches();
                bs.extend(g!().remote_branches());
                let Some(rev) = self.pick("Merge revision", &bs, None)?.cloned() else {
                    return Ok(());
                };
                match cmd {
                    "merge-plain" => g!().merge(&rev, &args)?,
                    "merge-nocommit" => g!().merge_nocommit(&rev, &args)?,
                    "merge-squash" => g!().merge_squash(&rev)?,
                    "merge-editmsg" => {
                        let into = g!().current_branch().unwrap_or_else(|| "HEAD".into());
                        let default = format!("Merge branch '{rev}' into {into}");
                        let Some(msg) = self.read_line_prompt("Merge message", Some(&default))? else {
                            return Ok(());
                        };
                        g!().merge_with_message(&rev, &msg, &args)?;
                    }
                    _ => {
                        match g!().merge_preview(&rev) {
                            Ok(p) if p.trim().is_empty() => self.set_msg("merge would not change anything"),
                            Ok(p) => self.view_text = Some((format!("preview merge of {rev}"), p)),
                            Err(e) => self.set_msg(&format!("{e}")),
                        }
                        return Ok(());
                    }
                }
                self.set_msg("merged");
                self.refresh();
            }
            "merge-commit" => {
                match g!().merge_commit() {
                    Ok(()) => self.set_msg("merge committed"),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "merge-abort" => {
                g!().merge_abort()?;
                self.set_msg("merge aborted");
                self.refresh();
            }
            "rebase-pushremote" => {
                match g!().rebase_pushremote(&args) {
                    Ok(t) => self.set_msg(&format!("rebased onto {t}")),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "rebase-autosquash" => {
                let default = g!().autosquash_base();
                let Some(base) =
                    self.read_line_prompt("Autosquash commits after", default.as_deref().or(Some("HEAD~10")))?
                else {
                    return Ok(());
                };
                match g!().rebase_autosquash_with(&base, &args) {
                    Ok(()) => self.set_msg("autosquashed"),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "rebase-upstream" | "rebase-other" => {
                let bs = g!().branches();
                if cmd == "rebase-upstream" {
                    let branch = g!().current_branch().unwrap_or_else(|| "HEAD".into());
                    let target = format!("{branch}@{{u}}");
                    if g!().rev_parse(&target).is_ok() {
                        g!().rebase(&target, &args)?;
                        self.set_msg("rebased onto upstream");
                    } else {
                        self.set_msg("no upstream configured");
                    }
                } else if let Some(onto) = self.pick("Rebase onto", &bs, None)? {
                    g!().rebase(onto, &args)?;
                    self.set_msg("rebased");
                }
                self.refresh();
            }
            "rebase-continue" => {
                g!().rebase_continue()?;
                self.set_msg("rebase continued");
                self.refresh();
            }
            "rebase-skip" => {
                g!().rebase_skip()?;
                self.set_msg("rebase skipped");
                self.refresh();
            }
            "rebase-abort" => {
                g!().rebase_abort()?;
                self.set_msg("rebase aborted");
                self.refresh();
            }

            // -------- reset
            "reset-mixed" | "reset-soft" | "reset-hard" | "reset-keep" | "reset-index"
            | "reset-worktree" | "reset-quickly" => {
                let bs = g!().branches();
                let rev = self
                    .pick("Reset to (branch/hash)", &bs, Some("HEAD"))?
                    .cloned()
                    .unwrap_or_else(|| "HEAD".into());
                let hard_confirm = cmd == "reset-hard" || cmd == "reset-quickly";
                if hard_confirm && !self.yes_no(&format!("Discard uncommitted changes and reset to {rev}?"))? {
                    self.set_msg("aborted");
                    return Ok(());
                }
                match cmd {
                    "reset-mixed" => g!().reset_mixed(&rev)?,
                    "reset-soft" => g!().reset_soft(&rev)?,
                    "reset-hard" => g!().reset_hard(&rev)?,
                    "reset-keep" => g!().reset_keep(&rev)?,
                    "reset-index" => g!().reset_index(&rev)?,
                    "reset-worktree" => g!().reset_worktree(&rev)?,
                    "reset-quickly" => g!().reset_mixed(&rev)?, // Magit default without prefix arg
                    _ => unreachable!(),
                }
                self.set_msg("reset");
                self.refresh();
            }

            // -------- tag
            "tag-create" => {
                // a message is needed for annotated and signed tags
                let annotate = args
                    .iter()
                    .any(|a| a == "--annotate" || a == "--sign" || a.starts_with("--local-user="));
                let Some(name) = self.read_line_prompt("Tag name", None)? else {
                    return Ok(());
                };
                let rev = self
                    .read_line_prompt("Place tag on", Some("HEAD"))?
                    .unwrap_or_else(|| "HEAD".into());
                let edit = args.iter().any(|a| a == "--edit");
                let msg = if annotate && !edit {
                    self.read_line_prompt("Tag message", Some(&name))?
                } else {
                    None
                };
                if edit {
                    self.with_terminal(|g| g.tag_create(&name, &rev, &args, None))?;
                } else {
                    g!().tag_create(&name, &rev, &args, msg.as_deref())?;
                }
                self.set_msg(&format!("tagged {name}"));
                self.refresh();
            }
            "tag-delete" => {
                let tags = g!().tags();
                if let Some(t) = self.pick("Delete tag", &tags, None)? {
                    g!().tag_delete(t)?;
                    self.set_msg("tag deleted");
                    self.refresh();
                }
            }

            // -------- revert / cherry-pick
            "revert" | "revert-no-commit" => {
                let Some(r) = self.pick_recent_commit("Revert commit")? else {
                    return Ok(());
                };
                let no_commit = cmd == "revert-no-commit";
                g!().revert(&[r.clone()], &args, no_commit)?;
                self.set_msg(if no_commit {
                    "reverted changes in the working tree"
                } else {
                    "reverted"
                });
                self.refresh();
            }
            "sequencer-continue" | "sequencer-skip" | "sequencer-abort" => {
                let action = cmd.trim_start_matches("sequencer-");
                match g!().sequencer(action) {
                    Ok(()) => self.set_msg(&format!("sequencer {action}")),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            // magit-apply / magit-stash-apply / magit-cherry-apply
            "apply" => self.apply_cmd(&args)?,
            // magit-reverse / magit-revert-no-commit
            "reverse" => self.reverse_cmd()?,
            "cherry-pick" | "cherry-apply" => {
                let log = g!().log(None, 30)?;
                let revs: Vec<String> = log
                    .iter()
                    .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
                    .collect();
                if let Some(r) = self.pick("Cherry (hash)", &revs, None)? {
                    match cmd {
                        "cherry-pick" => g!().cherry_pick(&[r.clone()], &args)?,
                        _ => g!().cherry_apply(&[r.clone()], &args)?,
                    }
                    self.set_msg("done");
                    self.refresh();
                }
            }

            // -------- stash
            "stash-both" | "stash-index" | "stash-worktree" | "stash-keep-index" => {
                let untracked = self.untracked_from(active);
                let msg = self.read_line_prompt("Stash message", Some("ngit stash"))?;
                match cmd {
                    "stash-both" => g!().stash_both(msg.as_deref(), untracked)?,
                    "stash-index" => g!().stash_index(msg.as_deref())?,
                    "stash-worktree" => g!().stash_worktree(msg.as_deref(), untracked)?,
                    "stash-keep-index" => g!().stash_keep_index(msg.as_deref(), untracked)?,
                    _ => unreachable!(),
                }
                self.set_msg("stashed");
                self.refresh();
            }
            "snapshot-both" | "snapshot-index" | "snapshot-worktree" => {
                let untracked = self.untracked_from(active);
                match cmd {
                    "snapshot-both" => g!().snapshot(crate::git::ops::SnapshotScope::Both, untracked)?,
                    "snapshot-index" => {
                        g!().snapshot(crate::git::ops::SnapshotScope::Index, untracked)?
                    }
                    _ => g!().snapshot(crate::git::ops::SnapshotScope::Worktree, untracked)?,
                }
                self.set_msg("snapshot saved");
                self.refresh();
            }
            "stash-show" => self.diff_view("stash")?,
            "stash-apply" | "stash-pop" | "stash-drop" | "stash-branch" => {
                let stashes = g!().stashes();
                if stashes.is_empty() {
                    self.set_msg("no stashes");
                    return Ok(());
                }
                let names: Vec<String> = stashes.iter().map(|(n, _)| n.clone()).collect();
                if let Some(s) = self.pick("Stash", &names, None)? {
                    match cmd {
                        "stash-apply" => g!().stash_apply(s, true)?,
                        "stash-pop" => g!().stash_pop(s, true)?,
                        "stash-drop" => {
                            if self.yes_no(&format!("Drop {s}?"))? {
                                g!().stash_drop(s)?;
                            } else {
                                return Ok(());
                            }
                        }
                        "stash-branch" => {
                            if let Some(name) = self.read_line_prompt("Branch name", None)? {
                                g!().stash_branch(&name, s)?;
                            } else {
                                return Ok(());
                            }
                        }
                        _ => unreachable!(),
                    }
                    self.set_msg("done");
                    self.refresh();
                }
            }
            "stash-list" => self.setup_buffer(BufferMode::Stashes { refname: "refs/stash".into() }, Vec::new()),

            // -------- ignore / misc
            "ignore" => {
                let Some(pattern) = self.read_line_prompt("Pattern to ignore (top dir)", None)? else {
                    return Ok(());
                };
                g!().ignore(&pattern)?;
                self.set_msg("added to .gitignore");
                self.refresh();
            }
            "ignore-subdir" => {
                let default = self
                    .current_file()
                    .and_then(|(_, p)| std::path::Path::new(&p).parent().map(|d| d.display().to_string()))
                    .filter(|d| !d.is_empty());
                let Some(dir) = self.read_line_prompt("Subdirectory", default.as_deref())? else {
                    return Ok(());
                };
                let Some(pattern) = self.read_line_prompt(&format!("Pattern to ignore in {dir}"), None)? else {
                    return Ok(());
                };
                g!().ignore_in_subdir(&dir, &pattern)?;
                self.set_msg(&format!("added to {}/.gitignore", dir.trim_end_matches('/')));
                self.refresh();
            }
            "git-command" | "git-command-cwd" => {
                let Some(line) = self.read_line_prompt("git command (args after 'git')", Some("status"))? else {
                    return Ok(());
                };
                let parts: Vec<String> = line.split_whitespace().map(|s| s.to_string()).collect();
                let strs: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
                let out = if cmd == "git-command-cwd" {
                    g!().git_string_in(&self.cwd, &strs)
                } else {
                    g!().git_string(&strs)
                };
                match out {
                    Ok(o) => self.view_text = Some((format!("git {}", line), o)),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "show-refs" => {
                let mut out = String::new();
                out.push_str(&format!("Branches:\n"));
                for b in g!().branches() {
                    let marker = if Some(b.clone()) == g!().current_branch() { "*" } else { " " };
                    out.push_str(&format!("  {marker} {b}\n"));
                }
                out.push_str("Tags:\n");
                for t in g!().tags() {
                    out.push_str(&format!("  {t}\n"));
                }
                self.view_text = Some(("refs".into(), out));
            }

            // -------- log / diff / show (arguments come from the l/d menus)
            "log-current" | "log-head" | "log-other" | "log-related" | "log-branches" | "log-all-branches"
            | "log-all" | "log-change" => self.log_view(cmd)?,
            // magit-log-refresh (L g): new arguments for the log buffer shown
            "log-refresh" => match self.mode {
                BufferMode::Log { .. } => {
                    self.buf_args = args.clone();
                    self.refresh();
                }
                _ => {
                    self.log_args = args.clone();
                    self.set_msg(&format!("log arguments: {}", self.log_args.join(" ")));
                }
            },
            "reflog-current" | "reflog-other" | "reflog-head" => {
                let refname = match cmd {
                    "reflog-head" => "HEAD".to_string(),
                    "reflog-current" => g!().current_branch().unwrap_or_else(|| "HEAD".into()),
                    _ => {
                        let default = match self.current_line().map(|l| l.kind.clone()) {
                            Some(LineKind::Commit { hash, .. }) => Some(self.git.rev_short(&hash)),
                            _ => g!().current_branch(),
                        };
                        let Some(r) = self.read_line_prompt("Show reflog for", default.as_deref())? else {
                            return Ok(());
                        };
                        r
                    }
                };
                self.setup_buffer(BufferMode::Reflog { refname }, vec![format!("-n{REFLOG_LIMIT}")]);
            }
            // magit-shortlog-since / -range: `git shortlog ARGS REV` shown in
            // the *magit-shortlog* buffer
            "shortlog-since" | "shortlog-range" => {
                let rev = if cmd == "shortlog-since" {
                    let default = crate::ui::view::current_tag(g!()).map(|(t, _)| t);
                    let Some(c) = self.read_line_prompt("Shortlog since", default.as_deref())? else {
                        return Ok(());
                    };
                    format!("{c}..")
                } else {
                    let default = match self.dwim() {
                        Dwim::Commit(c) => Some(self.git.rev_short(&c)),
                        Dwim::Range(r) => Some(r),
                        _ => g!().current_branch(),
                    };
                    let Some(r) = self.read_line_prompt("Shortlog for revision or range", default.as_deref())? else {
                        return Ok(());
                    };
                    r
                };
                let mut a: Vec<&str> = vec!["shortlog"];
                a.extend(args.iter().map(String::as_str));
                a.push(&rev);
                match g!().git_string(&a) {
                    Ok(o) => self.view_text = Some(("*magit-shortlog*".into(), o)),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
            }
            "diff-unstaged" | "diff-staged" | "diff-worktree" | "diff-dwim" | "diff-range"
            | "show-commit" => {
                let kind = if cmd == "show-commit" { "commit" } else { &cmd[5..] };
                self.diff_view(kind)?;
            }
            // magit-diff-refresh (D g): new arguments for the buffer shown
            "diff-refresh" => {
                match self.mode {
                    BufferMode::Status => self.status_args = args.clone(),
                    _ => self.buf_args = args.clone(),
                }
                self.refresh();
            }
            "process-buffer" => {
                let log = crate::git::exec::process_log();
                let text = if log.is_empty() {
                    "No git commands run yet".to_string()
                } else {
                    log.join("\n")
                };
                self.view_text = Some(("git process log".into(), text));
            }
            "section-up" => self.section_up(),
            "diff-visit-worktree-file" => self.visit(true)?,

            // -------- repo-level
            "init" => {
                let Some(path) = self.read_line_prompt("Initialize repository at", Some("."))? else {
                    return Ok(());
                };
                let abs = std::path::Path::new(&path)
                    .canonicalize()
                    .unwrap_or_else(|_| std::path::PathBuf::from(&path));
                g!().git_interactive_at(&abs, &["init"])?;
                self.set_msg(&format!("initialized {path}"));
            }
            "clone" => {
                let Some(url) = self.read_line_prompt("Repository url", None)? else {
                    return Ok(());
                };
                let target = self.read_line_prompt("Target directory", Some("."))?;
                let t = target.unwrap_or_else(|| ".".into());
                g!().git_interactive_at(&std::env::current_dir()?, &["clone", &url, &t])?;
                self.set_msg("cloned");
            }

            // All remaining commands are Magit extras
            _ => self.run_extra(cmd, &args)?,
        }
        Ok(())
    }

    // ------------------------------------------------------------ diff / log views

    /// The diff commands of `d`: magit-diff-unstaged, -staged,
    /// -working-tree, -range, -dwim, magit-show-commit, magit-stash-show.
    fn diff_view(&mut self, kind: &str) -> Result<()> {
        match kind {
            "unstaged" => self.show_diff(None, None, "unstaged"),
            "staged" => self.show_diff(None, Some("--cached"), "staged"),
            "worktree" => self.show_diff(Some("HEAD".into()), None, "committed"),
            "range" => {
                let default = self.dwim_range_default();
                let Some(r) = self.read_line_prompt("Diff for range", default.as_deref())? else {
                    return Ok(());
                };
                self.show_diff(Some(r), None, "committed");
            }
            "dwim" => self.diff_dwim()?,
            "commit" => {
                let rev = match self.current_line().map(|l| l.kind.clone()) {
                    Some(LineKind::Commit { hash, .. }) => Some(hash),
                    _ => match &self.mode {
                        BufferMode::Revision { rev, .. } => Some(rev.clone()),
                        _ => {
                            let current = self.git.current_branch();
                            self.read_line_prompt("Show commit", current.as_deref())?
                        }
                    },
                };
                if let Some(rev) = rev {
                    self.show_commit(&rev);
                }
            }
            "stash" => {
                let at = match self.current_line().map(|l| l.kind.clone()) {
                    Some(LineKind::Stash { name, .. }) => Some(name),
                    _ => match &self.mode {
                        BufferMode::Stash { stash } => Some(stash.clone()),
                        _ => None,
                    },
                };
                let stash = match at {
                    Some(s) => Some(s),
                    None => {
                        let names: Vec<String> = self.git.stashes().into_iter().map(|(n, _)| n).collect();
                        self.pick("Show stash", &names, None)?.cloned()
                    }
                };
                if let Some(s) = stash {
                    self.show_stash(&s);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The default of magit-diff-read-range-or-commit: the dwim range, or
    /// the current branch.
    fn dwim_range_default(&self) -> Option<String> {
        match self.dwim() {
            Dwim::Commit(v) => Some(format!("{v}^..{v}")),
            Dwim::Range(r) => Some(r),
            _ => self.git.current_branch(),
        }
    }

    /// magit-diff--dwim
    fn dwim(&self) -> Dwim {
        if let Some(secs) = self.region_sections(true) {
            let commits: Vec<String> = secs
                .iter()
                .filter_map(|&s| match &self.buf.all[self.sec(s).start].kind {
                    LineKind::Commit { hash, .. } => Some(hash.clone()),
                    _ => None,
                })
                .collect();
            if commits.len() == secs.len() {
                return Dwim::Range(format!("{}..{}", commits[commits.len() - 1], commits[0]));
            }
        }
        let p = self.point();
        let line = self.buf.all.get(p);
        match &self.mode {
            BufferMode::Stash { stash } => {
                let group = match line.map(|l| &l.kind) {
                    Some(LineKind::Commit { hash, .. }) => return Dwim::Commit(hash.clone()),
                    Some(
                        LineKind::File { section, .. }
                        | LineKind::Hunk { section, .. }
                        | LineKind::DiffLine { section, .. }
                        | LineKind::Meta { section, .. },
                    ) => section.clone(),
                    _ => String::new(),
                };
                Dwim::Commit(match group.as_str() {
                    "stash-index" => format!("{stash}^2"),
                    "stash-untracked" => format!("{stash}^3"),
                    _ => stash.clone(),
                })
            }
            BufferMode::Revision { rev, .. } => Dwim::Commit(rev.clone()),
            // [commit], [file commit], [hunk file commit]: the diffs of a
            // commit in a log use its hash as their group
            BufferMode::Log { .. } | BufferMode::Reflog { .. } => match line.map(|l| &l.kind) {
                Some(LineKind::Commit { hash, .. }) => Dwim::Commit(hash.clone()),
                Some(
                    LineKind::File { section, .. }
                    | LineKind::Hunk { section, .. }
                    | LineKind::DiffLine { section, .. }
                    | LineKind::Meta { section, .. },
                ) => Dwim::Commit(section.clone()),
                _ => Dwim::None,
            },
            BufferMode::Stashes { .. } | BufferMode::Cherry { .. } => match line.map(|l| &l.kind) {
                Some(LineKind::Commit { hash, .. }) => Dwim::Commit(hash.clone()),
                Some(LineKind::Stash { name, .. }) => Dwim::Stash(name.clone()),
                _ => Dwim::None,
            },
            BufferMode::Diff(spec) => match (spec.typ.as_str(), &spec.range) {
                ("committed", Some(r)) => Dwim::Range(r.clone()),
                (t, _) => Dwim::Type(t.to_string()),
            },
            BufferMode::Status => {
                let inside = |id: &str| line.is_some_and(|l| l.ident == id || l.ancestors.iter().any(|a| a == id));
                if inside("unstaged") {
                    return Dwim::Type("unstaged".into());
                }
                if inside("staged") {
                    return Dwim::Type("staged".into());
                }
                let s = self.cur_sec();
                let ident = &self.sec(s).ident;
                if let Some(r) = ident.strip_prefix("unpushed:").or_else(|| ident.strip_prefix("unpulled:")) {
                    if self.sec(s).start == p || !matches!(line.map(|l| &l.kind), Some(LineKind::Commit { .. })) {
                        return Dwim::Range(range_to_endpoints(r));
                    }
                }
                match line.map(|l| &l.kind) {
                    Some(LineKind::Commit { hash, .. }) => Dwim::Commit(hash.clone()),
                    Some(LineKind::Stash { name, .. }) => Dwim::Stash(name.clone()),
                    _ => Dwim::None,
                }
            }
        }
    }

    /// magit-diff-dwim
    fn diff_dwim(&mut self) -> Result<()> {
        match self.dwim() {
            Dwim::Type(t) if t == "unstaged" => self.show_diff(None, None, "unstaged"),
            Dwim::Type(t) if t == "staged" => self.show_diff(None, Some("--cached"), "staged"),
            Dwim::Type(_) => self.refresh(),
            Dwim::Stash(s) => self.show_stash(&s),
            Dwim::Commit(v) => {
                self.mark = None;
                self.show_diff(Some(format!("{v}^..{v}")), None, "committed")
            }
            Dwim::Range(r) => {
                self.mark = None;
                self.show_diff(Some(r), None, "committed")
            }
            Dwim::None => self.diff_view("range")?,
        }
        Ok(())
    }

    /// magit-log-arguments: the arguments of the `l` transient, else those
    /// of the log buffer shown, else the current value of magit-log-mode.
    fn log_arguments(&self) -> Vec<String> {
        match (&self.transient_log_args, &self.mode) {
            (Some(a), _) => a.clone(),
            (None, BufferMode::Log { .. }) => self.buf_args.clone(),
            (None, _) => self.log_args.clone(),
        }
    }

    /// The log commands of `l` (magit-log-current, -head, -other, -related,
    /// -branches, -all-branches, -all) and magit-log-buffer-file.
    fn log_view(&mut self, cmd: &str) -> Result<()> {
        let current = self.git.current_branch();
        let at_point = match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Commit { hash, .. }) => Some(self.git.rev_short(&hash)),
            _ => None,
        };
        let mut files = Vec::new();
        let revs: Vec<String> = match cmd {
            "log-current" => vec![current.unwrap_or_else(|| "HEAD".into())],
            "log-head" => vec!["HEAD".into()],
            "log-other" => {
                // magit-log-read-revs
                let previous = self
                    .git
                    .git_string(&["rev-parse", "--abbrev-ref", "@{-1}"])
                    .ok()
                    .filter(|s| !s.is_empty());
                let default = at_point.clone().or(previous);
                let Some(r) = self.read_line_prompt("Log rev,s", default.as_deref())? else {
                    return Ok(());
                };
                let revs: Vec<String> = r.split([',', ' ']).filter(|s| !s.is_empty()).map(str::to_string).collect();
                if revs.is_empty() {
                    return Ok(());
                }
                revs
            }
            "log-related" => self.related_revs(),
            "log-branches" | "log-all-branches" => {
                let mut v = Vec::new();
                if current.is_none() {
                    v.push("HEAD".to_string());
                }
                v.push("--branches".into());
                if cmd == "log-all-branches" {
                    v.push("--remotes".into());
                }
                v
            }
            "log-all" => vec!["--all".into()],
            "log-change" => {
                let path = match self.current_file() {
                    Some((_, p)) => p,
                    None => {
                        let Some(p) = self.read_line_prompt("Log file", None)? else {
                            return Ok(());
                        };
                        p
                    }
                };
                files.push(path);
                vec![current.unwrap_or_else(|| "HEAD".into())]
            }
            _ => return Ok(()),
        };
        let args = self.log_arguments();
        self.setup_buffer(BufferMode::Log { revs, files }, args);
        // magit-log-goto-same-commit
        if let Some(h) = at_point {
            self.log_goto_commit(&h);
        }
        Ok(())
    }

    /// The revisions of magit-log-related: the current branch, its push
    /// target and upstream (and that upstream's upstream when it is local);
    /// for a detached HEAD, HEAD and the previous branch's.
    fn related_revs(&self) -> Vec<String> {
        let g = &self.git;
        let mut head = None;
        let current = g.current_branch().or_else(|| {
            head = Some("HEAD".to_string());
            g.git_string(&["rev-parse", "--abbrev-ref", "@{-1}"]).ok().filter(|s| !s.is_empty() && s != "HEAD")
        });
        let Some(current) = current else { return vec!["HEAD".into()] };
        let upstream_of = |b: &str| {
            g.git_string(&["rev-parse", "--abbrev-ref", &format!("{b}@{{upstream}}")]).ok().filter(|s| !s.is_empty())
        };
        let target = super::view::push_branch(g, &current)
            .filter(|t| g.git_ok(&["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{t}")]));
        let upstream = upstream_of(&current);
        let upup = upstream
            .as_ref()
            .filter(|u| g.git_ok(&["show-ref", "--verify", "--quiet", &format!("refs/heads/{u}")]))
            .and_then(|u| upstream_of(u));
        [Some(current), head, target, upstream, upup].into_iter().flatten().collect()
    }

    /// magit-log-goto-commit-section: move to the commit REV in the log.
    fn log_goto_commit(&mut self, rev: &str) -> bool {
        let abbrev = self.git.rev_short(rev);
        let id = format!("commit:{abbrev}");
        match (1..self.buf.sections.len()).find(|&i| self.sec(i).ident == id || self.sec(i).ident.starts_with(&format!("{id}:"))) {
            Some(s) => {
                self.goto_sec(s);
                true
            }
            None => false,
        }
    }

    /// `+`, `-` and `=` in log and reflog buffers.
    fn log_set_commit_limit(&mut self, f: Limit) {
        self.buf_args = set_commit_limit(&self.buf_args, f);
        self.refresh();
    }

    /// `j` in log buffers (magit-log-move-to-revision).
    fn log_move_to_revision(&mut self) -> Result<()> {
        let default = self.git.current_branch();
        let Some(rev) = self.read_line_prompt("In log, jump to", default.as_deref())? else {
            return Ok(());
        };
        if !self.log_goto_commit(&rev) {
            self.set_msg(&format!("{rev} isn't visible in the current log buffer"));
        }
        Ok(())
    }

    /// C-c C-n in log buffers (magit-log-move-to-parent).
    fn log_move_to_parent(&mut self) {
        let Some(LineKind::Commit { hash, .. }) = self.current_line().map(|l| l.kind.clone()) else { return };
        let parent_rev = format!("{hash}^1");
        let Ok(parent) = self.git.git_string(&["rev-parse", "--short", &parent_rev]) else {
            self.set_msg(&format!("Parent {parent_rev} does not exist"));
            return;
        };
        let s = self.cur_sec();
        let id = format!("commit:{parent}");
        match (s + 1..self.buf.sections.len()).find(|&i| self.sec(i).ident == id) {
            Some(t) => self.goto_sec(t),
            None => self.set_msg(&format!("Parent {parent} not found.  Try typing + first")),
        }
    }

    /// magit-cherry ("Y"): read the head (magit-read-branch) and the upstream
    /// (magit-read-other-branch, defaulting to the current branch unless that
    /// is the head, else the head's upstream, else the previous branch).
    fn cherry(&mut self) -> Result<()> {
        let mut branches = self.git.branches();
        branches.extend(self.git.remote_branches());
        let current = self.git.current_branch();
        let Some(head) = self.pick("Cherry head", &branches, current.as_deref())?.cloned() else {
            return Ok(());
        };
        let upstream_of = self
            .git
            .git_string(&["rev-parse", "--abbrev-ref", &format!("{head}@{{upstream}}")])
            .ok()
            .filter(|s| !s.is_empty());
        let previous = self
            .git
            .git_string(&["rev-parse", "--abbrev-ref", "@{-1}"])
            .ok()
            .filter(|s| !s.is_empty() && s != "HEAD");
        let default = current.filter(|c| *c != head).or(upstream_of).or(previous);
        let others: Vec<String> = branches.into_iter().filter(|b| *b != head).collect();
        let Some(upstream) = self.pick("Cherry upstream", &others, default.as_deref())?.cloned() else {
            return Ok(());
        };
        self.setup_buffer(BufferMode::Cherry { head, upstream }, Vec::new());
        Ok(())
    }

    // ------------------------------------------------------------ sections (magit-section)

    fn sec(&self, s: usize) -> &Section {
        &self.buf.sections[s]
    }

    /// The section at point (magit-current-section); 0 is the root.
    fn cur_sec(&self) -> usize {
        self.buf.section_at(self.point())
    }

    fn hidden(&self, s: usize) -> bool {
        s != 0 && self.buf.is_hidden(&self.vis, &self.buf.sections[s].ident)
    }

    fn set_hidden(&mut self, s: usize, hidden: bool) {
        if s != 0 {
            let id = self.buf.sections[s].ident.clone();
            self.vis.insert(id, hidden);
        }
    }

    fn children(&self, s: usize) -> Vec<usize> {
        self.buf.sections[s].children.clone()
    }

    fn goto_sec(&mut self, s: usize) {
        let p = self.buf.sections[s].start;
        self.cursor = self.visible_index_near(p);
    }

    /// forward-line N (used by movement commands on the root section)
    fn forward_line(&mut self, n: isize) {
        let max = self.lines.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + n).clamp(0, max.max(0)) as usize;
    }

    /// magit-section-siblings: the next / previous sibling of S.
    fn sibling(&self, s: usize, next: bool) -> Option<usize> {
        let ch = &self.sec(self.sec(s).parent?).children;
        let i = ch.iter().position(|&c| c == s)?;
        if next {
            ch.get(i + 1).copied()
        } else {
            i.checked_sub(1).map(|j| ch[j])
        }
    }

    /// Point is on the empty last line of S (`(= (oref s end) (1+ (point)))`).
    fn at_section_end(&self, s: usize, p: usize) -> bool {
        let sec = self.sec(s);
        sec.end == p + 1 && p != sec.start && self.buf.all.get(p).is_some_and(|l| l.text.is_empty())
    }

    /// magit-section-forward (n)
    fn section_forward(&mut self) {
        let p = self.point();
        let s = self.cur_sec();
        if s == 0 {
            return self.forward_line(1);
        }
        let mut next = if !self.hidden(s) && !self.at_section_end(s, p) {
            self.sec(s).children.first().copied()
        } else {
            None
        };
        let mut cur = Some(s);
        while next.is_none() {
            let Some(c) = cur else { break };
            next = self.sibling(c, true);
            if next.is_none() {
                cur = self.sec(c).parent;
            }
        }
        match next {
            Some(n) => self.goto_sec(n),
            None => self.set_msg("No next section"),
        }
    }

    /// magit-section-backward (p)
    fn section_backward(&mut self) {
        let p = self.point();
        if p == 0 {
            return self.set_msg("No previous section");
        }
        let s = self.cur_sec();
        let sec = self.sec(s).clone();
        if self.at_section_end(s, p) && !sec.children.is_empty() {
            let last = *sec.children.last().unwrap_or(&s);
            return self.goto_sec(last);
        }
        if sec.parent.is_some() && p != sec.start {
            return self.goto_sec(s);
        }
        let prev = match self.sibling(s, false) {
            Some(mut prev) => {
                while !self.hidden(prev) && !self.sec(prev).children.is_empty() {
                    prev = *self.sec(prev).children.last().unwrap_or(&prev);
                }
                Some(prev)
            }
            None => sec.parent,
        };
        match prev {
            Some(x) => self.goto_sec(x),
            None => self.forward_line(-1),
        }
    }

    /// magit-section-forward-sibling (M-n)
    fn sibling_forward(&mut self) {
        let s = self.cur_sec();
        if s == 0 {
            return self.forward_line(1);
        }
        match self.sibling(s, true) {
            Some(n) => self.goto_sec(n),
            None => self.section_forward(),
        }
    }

    /// magit-section-backward-sibling (M-p)
    fn sibling_backward(&mut self) {
        let s = self.cur_sec();
        if s == 0 {
            return self.forward_line(-1);
        }
        match self.sibling(s, false) {
            Some(n) => self.goto_sec(n),
            None => self.section_backward(),
        }
    }

    /// magit-section-up (^)
    fn section_up(&mut self) {
        match self.sec(self.cur_sec()).parent {
            Some(par) => self.goto_sec(par),
            None => self.set_msg("No parent section"),
        }
    }

    fn show_sec(&mut self, s: usize) {
        self.set_hidden(s, false);
    }

    fn hide_sec(&mut self, s: usize) {
        self.set_hidden(s, true);
    }

    fn hide_children(&mut self, s: usize) {
        for c in self.children(s) {
            self.hide_sec(c);
        }
    }

    /// magit-section-show-headings
    fn show_headings(&mut self, s: usize) {
        self.show_headings_1(s);
        self.show_sec(s);
    }

    fn show_headings_1(&mut self, s: usize) {
        for c in self.children(s) {
            self.set_hidden(c, false);
            if !self.sec(c).children.is_empty() || !self.sec(c).content {
                self.show_headings_1(c);
            }
        }
    }

    /// magit-section-show-children
    fn show_children(&mut self, s: usize, depth: Option<usize>) {
        self.show_children_1(s, depth);
        self.show_sec(s);
    }

    fn show_children_1(&mut self, s: usize, depth: Option<usize>) {
        for c in self.children(s) {
            self.set_hidden(c, false);
            match depth {
                Some(0) => self.hide_sec(c),
                Some(d) => self.show_children_1(c, Some(d - 1)),
                None => self.show_children_1(c, None),
            }
        }
    }

    /// magit-section-hidden-body
    fn hidden_body(&self, s: usize) -> bool {
        let ch = &self.sec(s).children;
        if ch.is_empty() {
            self.sec(s).content && self.hidden(s)
        } else {
            ch.iter().any(|&c| self.hidden_body(c))
        }
    }

    /// magit-section-toggle (TAB)
    fn toggle_section(&mut self) {
        let p = self.point();
        let s = self.cur_sec();
        if s == 0 {
            return self.set_msg("Cannot hide root section");
        }
        if self.hidden(s) {
            self.show_sec(s);
        } else {
            self.hide_sec(s);
        }
        self.relayout_at(p);
    }

    /// The cycle step shared by magit-section-cycle and -cycle-global for
    /// the children CH of a shown section; returns false when nothing applied.
    fn cycle_children(&mut self, s: usize, ch: &[usize]) -> bool {
        if ch.iter().any(|&c| self.hidden(c)) && ch.iter().any(|&c| !self.sec(c).children.is_empty()) {
            self.show_headings(s);
        } else if ch.iter().any(|&c| self.hidden_body(c)) {
            self.show_children(s, None);
        } else {
            return false;
        }
        true
    }

    /// magit-section-cycle (C-TAB)
    fn cycle_section(&mut self) {
        let p = self.point();
        let s = self.cur_sec();
        if s == 0 {
            return self.cycle_global();
        }
        if self.hidden(s) {
            self.show_sec(s);
            self.hide_children(s);
        } else {
            let ch = self.children(s);
            if !self.cycle_children(s, &ch) {
                self.hide_sec(s);
            }
        }
        self.relayout_at(p);
    }

    /// magit-section-cycle-global (S-TAB)
    fn cycle_global(&mut self) {
        let p = self.point();
        let ch = self.children(0);
        if !self.cycle_children(0, &ch) {
            for c in ch {
                self.hide_sec(c);
            }
        }
        self.relayout_at(p);
    }

    /// magit-section-cycle-diffs (M-TAB): the file sections of the staged
    /// and unstaged changes.
    fn cycle_diffs(&mut self) {
        let p = self.point();
        let mut files = Vec::new();
        if self.mode == BufferMode::Status {
            for id in ["staged", "unstaged"] {
                if let Some(s) = (1..self.buf.sections.len()).find(|&s| self.sec(s).ident == id) {
                    if self.hidden(s) {
                        self.show_sec(s);
                    }
                    files.extend(self.children(s));
                }
            }
        } else {
            files = self
                .children(0)
                .into_iter()
                .filter(|&s| matches!(self.buf.all[self.sec(s).start].kind, LineKind::File { .. }))
                .collect();
        }
        if files.is_empty() {
            return self.relayout_at(p);
        }
        if files.iter().any(|&f| self.hidden(f)) {
            for &f in &files {
                self.show_sec(f);
                self.hide_children(f);
            }
        } else {
            let hunks: Vec<usize> = files.iter().flat_map(|&f| self.children(f)).collect();
            if hunks.iter().any(|&h| self.hidden(h)) && hunks.iter().any(|&h| !self.sec(h).children.is_empty()) {
                for &f in &files {
                    self.show_headings(f);
                }
            } else if hunks.iter().any(|&h| self.hidden_body(h)) {
                for &f in &files {
                    self.show_children(f, None);
                }
            } else {
                for &f in &files {
                    self.hide_sec(f);
                }
            }
        }
        self.relayout_at(p);
    }

    /// magit-section-show-level: LEVEL > 0 for the sections around point
    /// (1..4), LEVEL < 0 for all sections (M-1..M-4).
    fn show_level(&mut self, level: i32) {
        if level < 0 {
            let lvl = level.unsigned_abs() as usize;
            let mut s = self.cur_sec();
            let mut p = self.point();
            while self.sec(s).level > lvl {
                s = self.sec(s).parent.unwrap_or(0);
                p = self.sec(s).start;
            }
            self.show_children(0, Some(lvl.saturating_sub(1)));
            return self.relayout_at(p);
        }
        let lvl = level as usize;
        let sections = self.region_sections(false).unwrap_or_else(|| vec![self.cur_sec()]);
        let mut p = self.point();
        for section in sections {
            let mut s = section;
            let mut i = self.sec(s).level;
            loop {
                if i < lvl {
                    self.show_children(s, Some(lvl - i - 1));
                    break;
                }
                if i == lvl {
                    self.hide_sec(s);
                    break;
                }
                s = self.sec(s).parent.unwrap_or(0);
                i -= 1;
            }
            p = self.sec(s).start;
        }
        self.relayout_at(p);
    }

    /// magit-region-sections: sibling sections selected by a region that
    /// begins and ends in their headings. With MULTIPLE, a single section
    /// does not count.
    fn region_sections(&self, multiple: bool) -> Option<Vec<usize>> {
        let m = self.mark?;
        let p = self.point();
        let (rb, re) = (m.min(p), m.max(p));
        let sb = self.buf.section_at(rb);
        let se = self.buf.section_at(re);
        if se == 0 || (sb == se && (multiple || re > rb)) {
            return None;
        }
        if self.sec(sb).start != rb || self.sec(se).start != re {
            return None;
        }
        let ch = &self.sec(self.sec(sb).parent?).children;
        let i = ch.iter().position(|&c| c == sb)?;
        let j = ch.iter().position(|&c| c == se)?;
        (j >= i).then(|| ch[i..=j].to_vec())
    }

    // ------------------------------------------------------------ diff selection (magit-diff-scope)

    /// magit-apply--get-selection + magit-diff-scope + magit-diff-type.
    fn diff_selection(&self) -> Option<DiffSel> {
        if let Some(secs) = self.region_sections(true) {
            let kinds: Vec<&LineKind> = secs.iter().map(|&s| &self.buf.all[self.sec(s).start].kind).collect();
            let files: Vec<(String, String)> = kinds
                .iter()
                .filter_map(|k| match k {
                    LineKind::File { section, path } => Some((section.clone(), path.clone())),
                    _ => None,
                })
                .collect();
            if files.len() == kinds.len() {
                let typ = files[0].0.clone();
                return Some(DiffSel::Files { typ, paths: files.into_iter().map(|f| f.1).collect() });
            }
            let hunks: Vec<(String, String, usize)> = kinds
                .iter()
                .filter_map(|k| match k {
                    LineKind::Hunk { section, path, hunk } => Some((section.clone(), path.clone(), *hunk)),
                    _ => None,
                })
                .collect();
            if hunks.len() == kinds.len() {
                let (typ, path) = (hunks[0].0.clone(), hunks[0].1.clone());
                return Some(DiffSel::Hunks { typ, path, hunks: hunks.into_iter().map(|h| h.2).collect() });
            }
        }
        let s = self.cur_sec();
        if s == 0 {
            return None;
        }
        let start = self.sec(s).start;
        let is_list = |t: &str| matches!(t, "untracked" | "unstaged" | "staged");
        match &self.buf.all[start].kind {
            LineKind::Hunk { section, path, hunk } => {
                if let Some(m) = self.mark {
                    let p = self.point();
                    let line = |q: usize| match self.buf.all[q].kind {
                        LineKind::DiffLine { line, .. } => Some(line),
                        _ => None,
                    };
                    if self.buf.section_at(m) == s && m != start && p != start {
                        if let (Some(a), Some(b)) = (line(m), line(p)) {
                            return Some(DiffSel::Region {
                                typ: section.clone(),
                                path: path.clone(),
                                hunk: *hunk,
                                from: a.min(b),
                                to: a.max(b),
                            });
                        }
                    }
                }
                Some(DiffSel::Hunk { typ: section.clone(), path: path.clone(), hunk: *hunk })
            }
            LineKind::File { section, path } => Some(DiffSel::File { typ: section.clone(), path: path.clone() }),
            LineKind::Meta { section, path, kind } => Some(DiffSel::Meta {
                typ: section.clone(),
                path: path.clone(),
                scope: if kind == "chmod" { Scope::Chmod } else { Scope::Rename },
            }),
            LineKind::SectionHeader { section, .. } if is_list(section) => Some(DiffSel::List { typ: section.clone() }),
            _ => None,
        }
    }

    fn context(&self) -> usize {
        diff_context(&self.buffer_args())
    }

    fn file_diff(&self, typ: &str, path: &str) -> Result<FileDiff> {
        self.buf
            .diff(typ, path)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Cannot get required diff headers"))
    }

    /// Paths of the files a file / files / list selection covers.
    fn sel_paths(&self, sel: &DiffSel) -> Vec<String> {
        match sel {
            DiffSel::File { path, .. } => vec![path.clone()],
            DiffSel::Files { paths, .. } => paths.clone(),
            DiffSel::List { typ } if typ == "untracked" => self.git.untracked_files(),
            DiffSel::List { typ } => self.buf.files(typ).iter().map(|f| f.path.clone()).collect(),
            DiffSel::Hunk { path, .. }
            | DiffSel::Hunks { path, .. }
            | DiffSel::Region { path, .. }
            | DiffSel::Meta { path, .. } => vec![path.clone()],
        }
    }

    /// magit-apply--ignore-whitespace-p: whole files have to be applied as
    /// patches when the diff ignores whitespace (and no file is binary).
    fn ignore_whitespace(&self, sel: &DiffSel) -> bool {
        let group = sel.typ();
        if !matches!(self.diff_type(group), "unstaged" | "staged") || !sel.is_files() {
            return false;
        }
        let flags = ["--ignore-space-at-eol", "--ignore-space-change", "--ignore-all-space", "--ignore-blank-lines"];
        self.buffer_args().iter().any(|a| flags.contains(&a.as_str()))
            && !self.sel_paths(sel).iter().any(|p| self.buf.diff(group, p).is_some_and(|d| d.binary))
    }

    /// Hunk / hunks / region (or pseudo-hunk) selection as (path, scope).
    fn hunk_scope(sel: &DiffSel) -> Option<(String, Scope)> {
        match sel {
            DiffSel::Hunk { path, hunk, .. } => Some((path.clone(), Scope::Hunk(*hunk))),
            DiffSel::Hunks { path, hunks, .. } => Some((path.clone(), Scope::Hunks(hunks.clone()))),
            DiffSel::Region { path, hunk, from, to, .. } => {
                Some((path.clone(), Scope::Region { hunk: *hunk, from: *from, to: *to }))
            }
            DiffSel::Meta { path, scope, .. } => Some((path.clone(), scope.clone())),
            _ => None,
        }
    }

    /// The prompt of magit-discard-* / magit-reverse-* for a hunk selection.
    fn hunk_prompt(verb: &str, sel: &DiffSel) -> String {
        match sel {
            DiffSel::Region { .. } => format!("{verb} region"),
            DiffSel::Hunks { hunks, path, .. } => format!("{verb} {} hunks from {path}", hunks.len()),
            _ => format!("{verb} hunk"),
        }
    }

    /// magit-confirm: "PROMPT? (y/n)"; a "no" leaves "Abort" in the echo area.
    fn confirm(&mut self, prompt: &str) -> Result<bool> {
        let ok = self.yes_no(&format!("{prompt}?"))?;
        if !ok {
            self.set_msg("Abort");
        }
        Ok(ok)
    }

    /// magit-confirm-files: `PROMPT "file"` or the file list and `PROMPT N files`.
    fn confirm_files(&mut self, prompt: &str, files: &[String]) -> Result<bool> {
        match files {
            [] => Ok(true),
            [f] => self.confirm(&format!("{prompt} \"{f}\"")),
            fs => self.confirm(&format!("{}\n\n{prompt} {} files", fs.join("\n"), fs.len())),
        }
    }

    // ------------------------------------------------------------ apply commands (magit-apply.el)

    /// `s`: magit-stage where a section keymap remaps magit-stage-files
    /// (files, hunks, the untracked and unstaged sections), else
    /// magit-stage-files.
    fn stage_cmd(&mut self) -> Result<()> {
        let sel = self.diff_selection().filter(|d| !matches!(d, DiffSel::List { typ } if typ == "staged"));
        let Some(sel) = sel else { return self.stage_files() };
        let ctx = self.context();
        let ws = self.ignore_whitespace(&sel);
        match self.diff_type(sel.typ()) {
            "untracked" => {
                let files = self.sel_paths(&sel);
                if files.is_empty() {
                    return Ok(());
                }
                let mut a = vec!["add", "--"];
                a.extend(files.iter().map(String::as_str));
                self.git.git_interactive(&a)?;
            }
            "unstaged" => {
                if let Some((path, scope)) = Self::hunk_scope(&sel) {
                    let fd = self.file_diff("unstaged", &path)?;
                    self.git.stage_change(&fd, &scope, ctx)?;
                } else if ws {
                    for p in self.sel_paths(&sel) {
                        let fd = self.file_diff("unstaged", &p)?;
                        self.git.stage_change(&fd, &Scope::File, ctx)?;
                    }
                } else if matches!(sel, DiffSel::List { .. }) {
                    return self.stage_modified();
                } else {
                    let files = self.sel_paths(&sel);
                    let mut a = vec!["add", "-u", "--"];
                    a.extend(files.iter().map(String::as_str));
                    self.git.git_interactive(&a)?;
                }
            }
            "staged" => {
                self.set_msg("Already staged");
                return Ok(());
            }
            "committed" => {
                self.set_msg("Cannot stage committed changes");
                return Ok(());
            }
            _ => {
                self.set_msg("Cannot stage this change");
                return Ok(());
            }
        }
        self.refresh();
        Ok(())
    }

    /// magit-stage-files: read files (unstaged and untracked), default the
    /// file at point.
    fn stage_files(&mut self) -> Result<()> {
        let mut choices = self.git.unstaged_files();
        choices.extend(self.git.untracked_files());
        let default = self.current_file().map(|f| f.1).filter(|f| choices.contains(f));
        let Some(file) = self.pick("Stage file,s", &choices, default.as_deref())?.cloned() else {
            return Ok(());
        };
        self.git.stage(&file)?;
        self.set_msg("staged");
        self.refresh();
        Ok(())
    }

    /// magit-stage-modified (S): `git add -u .`, confirming when something
    /// is staged already.
    fn stage_modified(&mut self) -> Result<()> {
        if self.git.anything_staged() && !self.confirm("Stage all changes")? {
            return Ok(());
        }
        self.git.stage_all()?;
        self.set_msg("staged all modified");
        self.refresh();
        Ok(())
    }

    /// `u`: magit-unstage on files, hunks and the staged section, else
    /// magit-unstage-files.
    fn unstage_cmd(&mut self) -> Result<()> {
        let sel = self.diff_selection().filter(|d| !matches!(d, DiffSel::List { typ } if typ != "staged"));
        let Some(sel) = sel else { return self.unstage_files() };
        let ctx = self.context();
        let ws = self.ignore_whitespace(&sel);
        match (self.diff_type(sel.typ()), &sel) {
            ("untracked", _) => {
                self.set_msg("Cannot unstage untracked changes");
                return Ok(());
            }
            ("unstaged", DiffSel::File { .. } | DiffSel::Files { .. }) => {
                // magit-unstage-intent
                let staged = self.git.staged_files();
                let intent: Vec<String> = self.sel_paths(&sel).into_iter().filter(|f| staged.contains(f)).collect();
                if intent.is_empty() {
                    self.set_msg("Already unstaged");
                    return Ok(());
                }
                for f in &intent {
                    self.git.unstage(f)?;
                }
            }
            ("unstaged", _) => {
                self.set_msg("Already unstaged");
                return Ok(());
            }
            // magit-unstage-committed: magit-reverse-in-index
            ("committed", _) => return self.reverse_sel(&sel, &["--cached"]),
            ("undefined", _) => {
                self.set_msg("Cannot unstage this change");
                return Ok(());
            }
            _ => {
                if let Some((path, scope)) = Self::hunk_scope(&sel) {
                    let fd = self.file_diff("staged", &path)?;
                    self.git.unstage_change(&fd, &scope, ctx)?;
                } else if ws {
                    for p in self.sel_paths(&sel) {
                        let fd = self.file_diff("staged", &p)?;
                        self.git.unstage_change(&fd, &Scope::File, ctx)?;
                    }
                } else if matches!(sel, DiffSel::List { .. }) {
                    return self.unstage_all();
                } else {
                    for p in self.sel_paths(&sel) {
                        self.git.unstage(&p)?;
                    }
                }
            }
        }
        self.refresh();
        Ok(())
    }

    /// magit-unstage-files: read staged files, default the file at point.
    fn unstage_files(&mut self) -> Result<()> {
        let choices = self.git.staged_files();
        let default = self.current_file().map(|f| f.1).filter(|f| choices.contains(f));
        let Some(file) = self.pick("Unstage file,s", &choices, default.as_deref())?.cloned() else {
            return Ok(());
        };
        self.git.unstage(&file)?;
        self.set_msg("unstaged");
        self.refresh();
        Ok(())
    }

    /// magit-unstage-all (U)
    fn unstage_all(&mut self) -> Result<()> {
        if !self.git.anything_staged() {
            self.set_msg("Nothing to unstage");
            return Ok(());
        }
        let others = !self.git.unstaged_files().is_empty() || !self.git.untracked_files().is_empty();
        if others && !self.confirm("Unstage all changes")? {
            return Ok(());
        }
        self.git.unstage_all()?;
        self.set_msg("unstaged all");
        self.refresh();
        Ok(())
    }

    /// `k`: magit-discard on changes, magit-stash-drop on a stash,
    /// magit-stash-clear on the stashes section.
    fn discard_cmd(&mut self) -> Result<()> {
        match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Stash { name, .. }) => {
                if self.yes_no(&format!("Drop {name}?"))? {
                    self.git.stash_drop(&name)?;
                    self.set_msg("stash dropped");
                    self.refresh();
                }
                return Ok(());
            }
            Some(LineKind::SectionHeader { section, .. }) if section == "stashes" => {
                if self.confirm("Drop all stashes in refs/stash")? {
                    self.git.stash_clear()?;
                    self.refresh();
                }
                return Ok(());
            }
            _ => {}
        }
        let Some(sel) = self.diff_selection() else {
            self.set_msg("There is no thing at point that could be deleted");
            return Ok(());
        };
        let ctx = self.context();
        let group = sel.typ().to_string();
        let typ = self.diff_type(&group).to_string();
        match typ.as_str() {
            "committed" => {
                self.set_msg("Cannot discard committed changes");
                return Ok(());
            }
            "undefined" => {
                self.set_msg("Cannot discard this change");
                return Ok(());
            }
            _ => {}
        }
        if let Some((path, scope)) = Self::hunk_scope(&sel) {
            if !self.confirm(&Self::hunk_prompt("Discard", &sel))? {
                return Ok(());
            }
            let fd = self.file_diff(&group, &path)?;
            self.git.discard_change(&fd, &scope, typ == "staged", ctx)?;
        } else {
            let files = self.sel_paths(&sel);
            let prompt = if typ == "untracked" { "Delete".to_string() } else { format!("Discard {typ} changes in") };
            if !self.confirm_files(&prompt, &files)? {
                return Ok(());
            }
            for f in &files {
                match typ.as_str() {
                    "untracked" => self.git.discard_untracked(f)?,
                    "unstaged" => self.git.discard_unstaged(f)?,
                    _ => self.git.discard_staged(f)?,
                }
            }
        }
        self.set_msg("discarded");
        self.refresh();
        Ok(())
    }

    /// `v`: magit-reverse on changes; magit-revert-no-commit elsewhere
    /// (default: the commit at point).
    fn reverse_cmd(&mut self) -> Result<()> {
        let sel = self.diff_selection().filter(|d| !matches!(d, DiffSel::List { typ } if typ != "staged"));
        let Some(sel) = sel else {
            let Some(r) = self.read_commit("Revert changes")? else { return Ok(()) };
            self.git.revert(std::slice::from_ref(&r), &[], true)?;
            self.set_msg(&format!("reversed {r} in the working tree"));
            self.refresh();
            return Ok(());
        };
        match self.diff_type(sel.typ()) {
            "untracked" => {
                self.set_msg("Cannot reverse untracked changes");
                Ok(())
            }
            "unstaged" => {
                self.set_msg("Cannot reverse unstaged changes");
                Ok(())
            }
            _ => self.reverse_sel(&sel, &[]),
        }
    }

    /// magit-reverse-region / -hunk / -hunks / -files: apply the selection
    /// with `--reverse ARGS --reject`.
    fn reverse_sel(&mut self, sel: &DiffSel, args: &[&str]) -> Result<()> {
        let group = sel.typ().to_string();
        let ctx = self.context();
        let mut a = vec!["--reverse"];
        a.extend(args);
        a.push("--reject");
        if let Some((path, scope)) = Self::hunk_scope(sel) {
            if !self.confirm(&Self::hunk_prompt("Reverse", sel))? {
                return Ok(());
            }
            let fd = self.file_diff(&group, &path)?;
            self.git.apply_scope(&fd, &scope, &a, ctx)?;
        } else {
            let (binaries, files): (Vec<String>, Vec<String>) = self
                .sel_paths(sel)
                .into_iter()
                .partition(|f| self.buf.diff(&group, f).is_some_and(|d| d.binary));
            if !self.confirm_files("Reverse", &files)? {
                return Ok(());
            }
            let fds = files.iter().map(|f| self.file_diff(&group, f)).collect::<Result<Vec<_>>>()?;
            self.git.apply_files(&fds.iter().collect::<Vec<_>>(), &a, ctx)?;
            if !binaries.is_empty() {
                self.refresh();
                self.set_msg("Cannot reverse binary files");
                return Ok(());
            }
        }
        self.refresh();
        Ok(())
    }

    /// `a`: magit-apply on changes, magit-stash-apply on a stash,
    /// magit-cherry-apply elsewhere (default: the commit at point).
    fn apply_cmd(&mut self, args: &[String]) -> Result<()> {
        if let Some(LineKind::Stash { name, .. }) = self.current_line().map(|l| l.kind.clone()) {
            self.git.stash_apply(&name, true)?;
            self.set_msg(&format!("applied {name}"));
            self.refresh();
            return Ok(());
        }
        let Some(sel) = self.diff_selection() else {
            let Some(r) = self.read_commit("Apply changes from commit")? else { return Ok(()) };
            self.git.cherry_apply(std::slice::from_ref(&r), args)?;
            self.set_msg(&format!("applied {r} to the working tree"));
            self.refresh();
            return Ok(());
        };
        let group = sel.typ().to_string();
        let three: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a == "--3way").collect();
        match (self.diff_type(&group), &sel) {
            ("unstaged" | "staged", _) => self.set_msg("Change is already in the working tree"),
            ("untracked", DiffSel::File { .. } | DiffSel::Files { .. }) => self.open_menu("patches-menu"),
            ("untracked", _) => {}
            _ => {
                let ctx = self.context();
                if let Some((path, scope)) = Self::hunk_scope(&sel) {
                    let fd = self.file_diff(&group, &path)?;
                    self.git.apply_scope(&fd, &scope, &three, ctx)?;
                } else {
                    let fds = self.sel_paths(&sel).iter().map(|f| self.file_diff(&group, f)).collect::<Result<Vec<_>>>()?;
                    self.git.apply_files(&fds.iter().collect::<Vec<_>>(), &three, ctx)?;
                }
                self.refresh();
            }
        }
        Ok(())
    }

    /// magit-diff-set-context: `+` / `-` / `0` change `-U<n>` of the diffs
    /// of the buffer shown.
    fn set_diff_context(&mut self, f: impl Fn(usize) -> Option<usize>) {
        let def = self
            .git
            .git_string(&["config", "diff.context"])
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(3);
        let args = match self.mode {
            BufferMode::Status => &mut self.status_args,
            _ => &mut self.buf_args,
        };
        let is_u = |a: &String| a.starts_with("-U") && a[2..].chars().all(|c| c.is_ascii_digit());
        let pos = args.iter().position(is_u);
        let num = pos.and_then(|i| args[i][2..].parse().ok()).unwrap_or(def);
        if let Some(i) = pos {
            args.remove(i);
        }
        if let Some(n) = f(num).filter(|&n| n != def) {
            args.insert(0, format!("-U{n}"));
        }
        self.refresh();
    }

    // ------------------------------------------------------------ visiting

    /// magit-diff-visit--sides: the (revision, file) of the old and the new
    /// side of the diff of PATH in GROUP; revisions are "{worktree}",
    /// "{index}" or a revision.
    fn visit_sides(&self, group: &str, path: &str) -> ((String, String), (String, String)) {
        let orig = self.buf.diff(group, path).and_then(|d| d.orig.clone()).unwrap_or_else(|| path.to_string());
        let wt = || "{worktree}".to_string();
        let abbrev = |r: &str| self.git.rev_short(r);
        let (old, new) = match (&self.mode, self.diff_type(group)) {
            (BufferMode::Revision { rev, .. }, _) => (abbrev(&format!("{rev}^")), abbrev_if_oid(&self.git, rev)),
            // a diff inside a log: its group is the commit
            (BufferMode::Log { .. } | BufferMode::Reflog { .. }, _) => {
                (abbrev(&format!("{group}^")), abbrev_if_oid(&self.git, group))
            }
            (BufferMode::Stash { stash }, _) => {
                let rev = match group {
                    "stash-index" => format!("{stash}^2"),
                    "stash-untracked" => format!("{stash}^3"),
                    _ => stash.clone(),
                };
                (abbrev(&format!("{rev}^")), rev)
            }
            (BufferMode::Diff(spec), "committed") => match &spec.range {
                Some(r) => self.split_range(r, spec.typearg.as_deref() == Some("--cached")),
                None => (wt(), wt()),
            },
            (_, "staged") => (abbrev("HEAD"), "{index}".to_string()),
            (_, "unstaged") => {
                let staged = self.git.git_ok(&["diff", "--cached", "--quiet", "--", &orig]);
                (if staged { abbrev("HEAD") } else { "{index}".to_string() }, wt())
            }
            _ => (wt(), wt()),
        };
        ((old, orig), (new, path.to_string()))
    }

    /// magit-split-range: "A..B" -> (A, B), "A...B" -> (merge base, B); a
    /// single revision is compared with the worktree (or the index).
    fn split_range(&self, range: &str, cached: bool) -> (String, String) {
        let side = |s: &str| if s.is_empty() { "HEAD".to_string() } else { s.to_string() };
        if let Some((a, b)) = range.split_once("...") {
            let base = self.git.git_string(&["merge-base", &side(a), &side(b)]).unwrap_or_default();
            (self.git.rev_short(base.trim()), side(b))
        } else if let Some((a, b)) = range.split_once("..") {
            (side(a), side(b))
        } else {
            (range.to_string(), if cached { "{index}" } else { "{worktree}" }.to_string())
        }
    }

    /// magit-diff-hunk-line: the line in the visited file of body line LINE
    /// (or the heading) of hunk HUNK, on the old side when GOTO_FROM.
    fn hunk_line(fd: &FileDiff, hunk: usize, line: Option<usize>, goto_from: bool) -> usize {
        let Some(h) = fd.hunks.get(hunk) else { return 1 };
        let start = h.range().map_or(1, |r| if goto_from { r.old_start } else { r.new_start }).max(1);
        let skip = if goto_from { '+' } else { '-' };
        start + line.map_or(0, |li| h.lines.iter().take(li).filter(|l| !l.starts_with(skip)).count())
    }

    /// magit-diff-visit--offset: LINE of FILE at REV moved by the changes
    /// between REV (or the index) and the worktree.
    fn visit_offset(&self, line: usize, file: &str, rev: &str) -> usize {
        let out = if rev == "{index}" {
            self.git.git_string(&["diff", "--no-ext-diff", "--", file])
        } else {
            self.git.git_string(&["diff", "--no-ext-diff", rev, "--", file])
        };
        let mut offset: isize = 0;
        for h in crate::git::diff::parse(&out.unwrap_or_default()).iter().flat_map(|f| f.hunks.iter()) {
            let Some(r) = h.range() else { continue };
            if r.old_start > line {
                break;
            }
            if r.old_start + r.old_len < line {
                offset += r.new_len as isize - r.old_len as isize;
                continue;
            }
            let mut rest = line - r.old_start;
            for l in &h.lines {
                if rest == 0 {
                    break;
                }
                match l.chars().next() {
                    Some(' ') => rest -= 1,
                    Some('-') => {
                        offset -= 1;
                        rest -= 1;
                    }
                    Some('+') => offset += 1,
                    _ => {}
                }
            }
            break;
        }
        (line as isize + offset).max(1) as usize
    }

    /// magit-diff-visit-file (WORKTREE: magit-diff-visit-worktree-file):
    /// show the version of the file at point, at the line at point. Added
    /// and context lines visit the new side, removed lines the old side.
    fn visit_file(&mut self, group: &str, path: &str, hunk: Option<(usize, Option<usize>)>, worktree: bool) {
        let (old, new) = self.visit_sides(group, path);
        let fd = self.buf.diff(group, path).cloned();
        let on_removed = match (hunk, &fd) {
            (Some((h, Some(l))), Some(fd)) => fd.hunks.get(h).and_then(|x| x.lines.get(l)).is_some_and(|t| t.starts_with('-')),
            _ => false,
        };
        let goto_from = !worktree && on_removed;
        let (rev, file) = if goto_from { old } else { new };
        let mut line = match (hunk, &fd) {
            (Some((h, l)), Some(fd)) => Some(Self::hunk_line(fd, h, l, goto_from)),
            _ => None,
        };
        let (title, content) = if worktree || rev == "{worktree}" {
            if worktree && rev != "{worktree}" {
                line = line.map(|l| self.visit_offset(l, &file, &rev));
            }
            let full = self.git.toplevel.join(&file);
            match std::fs::read(&full) {
                Ok(bytes) => (file.clone(), String::from_utf8_lossy(&bytes).into_owned()),
                Err(_) => {
                    self.set_msg(&format!("Cannot visit {file}: file does not exist"));
                    return;
                }
            }
        } else {
            let spec = if rev == "{index}" { format!(":{file}") } else { format!("{rev}:{file}") };
            match self.git.git_string(&["cat-file", "-p", &spec]) {
                Ok(c) => (format!("{file}.~{rev}~"), c),
                Err(e) => {
                    self.set_msg(&format!("{e}"));
                    return;
                }
            }
        };
        self.view_text = Some((title, content));
        self.view_scroll = line.map_or(0, |l| l.saturating_sub(1));
    }

    /// RET (magit-visit-thing as remapped by the section at point)
    fn visit_thing(&mut self) -> Result<()> {
        self.visit(false)
    }

    /// RET, or C-RET / C-j (magit-diff-visit-worktree-file) when WORKTREE.
    fn visit(&mut self, worktree: bool) -> Result<()> {
        let Some(kind) = self.current_line().map(|l| l.kind.clone()) else { return Ok(()) };
        // the "Type + to show more history" button of log buffers
        if !worktree && self.current_line().is_some_and(|l| l.ident == "longer") {
            self.log_set_commit_limit(Limit::Double);
            return Ok(());
        }
        self.view_scroll = 0;
        match kind {
            LineKind::File { section, path } | LineKind::Meta { section, path, .. } => {
                self.visit_file(&section, &path, None, worktree)
            }
            LineKind::Hunk { section, path, hunk } => self.visit_file(&section, &path, Some((hunk, None)), worktree),
            LineKind::DiffLine { section, path, hunk, line } => {
                self.visit_file(&section, &path, Some((hunk, Some(line))), worktree)
            }
            _ if worktree => self.set_msg("C-<return> is undefined"),
            LineKind::SectionHeader { section, .. } if section == "unstaged" || section == "staged" => {
                self.diff_view(&section)?
            }
            LineKind::SectionHeader { section, .. } if section == "stashes" => {
                self.setup_buffer(BufferMode::Stashes { refname: "refs/stash".into() }, Vec::new())
            }
            LineKind::Stash { name, .. } => self.show_stash(&name),
            LineKind::Commit { hash, .. } => self.show_commit(&hash),
            _ => self.set_msg("There is no thing at point that could be visited"),
        }
        Ok(())
    }

    /// magit-show-commit: the revision buffer of REV, with the arguments of
    /// the revision buffer shown, else the current diff arguments.
    fn show_commit(&mut self, rev: &str) {
        let args = match (&self.transient_diff_args, &self.mode) {
            (Some(a), _) => a.clone(),
            (None, BufferMode::Revision { .. }) => self.buf_args.clone(),
            (None, _) => self.diff_args.clone(),
        };
        self.setup_buffer(BufferMode::Revision { rev: rev.to_string(), files: Vec::new() }, args);
    }

    /// magit-stash-show: the stash buffer of STASH (without "--stat").
    fn show_stash(&mut self, stash: &str) {
        let args = match (&self.transient_diff_args, &self.mode) {
            (Some(a), _) => a,
            (None, BufferMode::Stash { .. }) => &self.buf_args,
            (None, _) => &self.diff_args,
        };
        let args: Vec<String> = args.iter().filter(|a| *a != "--stat").cloned().collect();
        self.setup_buffer(BufferMode::Stash { stash: stash.to_string() }, args);
    }

    /// Show a diff buffer (magit-diff-setup-buffer).
    fn show_diff(&mut self, range: Option<String>, typearg: Option<&str>, typ: &str) {
        let spec = DiffSpec { range, typearg: typearg.map(str::to_string), typ: typ.into(), files: Vec::new() };
        let args = match (&self.transient_diff_args, &self.mode) {
            (Some(a), _) => a.clone(),
            (None, BufferMode::Diff(_)) => self.buf_args.clone(),
            (None, _) => self.diff_args.clone(),
        };
        self.setup_buffer(BufferMode::Diff(spec), args);
    }

    /// SPC / DEL: scroll the text view; in diff / revision / stash buffers
    /// scroll-up / scroll-down; in the status buffer
    /// magit-diff-show-or-scroll-up / -down (show the thing at point).
    fn show_or_scroll(&mut self, up: bool) -> Result<()> {
        let (_, h) = crossterm::terminal::size().unwrap_or((80, 24));
        let page = (h as usize).saturating_sub(5).max(1);
        if self.view_text.is_some() {
            self.view_scroll = if up { self.view_scroll + page } else { self.view_scroll.saturating_sub(page) };
            return Ok(());
        }
        if matches!(self.mode, BufferMode::Diff(_) | BufferMode::Revision { .. } | BufferMode::Stash { .. }) {
            if up {
                if self.scroll + page >= self.lines.len() {
                    self.set_msg("End of buffer");
                    return Ok(());
                }
                self.scroll += page;
                self.cursor = self.cursor.max(self.scroll);
            } else {
                if self.scroll == 0 {
                    self.set_msg("Beginning of buffer");
                    return Ok(());
                }
                self.scroll = self.scroll.saturating_sub(page);
                self.cursor = self.cursor.min(self.scroll + page - 1);
            }
            return Ok(());
        }
        match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Commit { hash, .. }) => self.show_commit(&hash),
            Some(LineKind::Stash { name, .. }) => self.show_stash(&name),
            Some(_) if self.diff_selection().is_some() => self.diff_dwim()?,
            _ => {}
        }
        Ok(())
    }

    /// `j`: magit-status-jump in the status buffer (the next top-level
    /// section), magit-jump-to-diffstat-or-diff elsewhere.
    fn jump_section(&mut self) {
        if self.mode != BufferMode::Status {
            return self.jump_to_diffstat_or_diff();
        }
        let headers: Vec<usize> = self
            .children(0)
            .into_iter()
            .filter(|&s| !matches!(self.buf.all[self.sec(s).start].kind, LineKind::Head))
            .map(|s| self.sec(s).start)
            .collect();
        let p = self.point();
        if let Some(&next) = headers.iter().find(|&&h| h > p).or(headers.first()) {
            self.cursor = self.visible_index_near(next);
        }
    }

    /// magit-jump-to-diffstat-or-diff: from a file in the diffstat to its
    /// diff, from a file or hunk to its diffstat line, else to the diffstat.
    fn jump_to_diffstat_or_diff(&mut self) {
        let s = self.cur_sec();
        let ident = self.sec(s).ident.clone();
        let target = match &self.buf.all[self.sec(s).start].kind {
            LineKind::File { section, path } if ident.contains("/diffstat/") => format!("{section}/{path}"),
            LineKind::File { section, path } | LineKind::Hunk { section, path, .. } | LineKind::Meta { section, path, .. } => {
                format!("{section}/diffstat/{path}")
            }
            _ => self
                .buf
                .sections
                .iter()
                .find(|x| x.ident.ends_with("/diffstat"))
                .map(|x| x.ident.clone())
                .unwrap_or_default(),
        };
        match (1..self.buf.sections.len()).find(|&i| !target.is_empty() && self.sec(i).ident == target) {
            Some(t) => self.goto_sec(t),
            None => self.set_msg("No diffstat in this buffer"),
        }
    }

    // ------------------------------------------------------------ point helpers

    fn pick_current_file_or_section(&self) -> Option<(String, String)> {
        if let Some((section, path)) = self.current_file() {
            return Some((section, path));
        }
        let files = self.files_in_current_section();
        files.first().map(|f| (String::new(), f.clone()))
    }
}

/// What magit-diff--dwim finds at point.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Dwim {
    None,
    /// "unstaged", "staged" or "undefined"
    Type(String),
    Commit(String),
    Stash(String),
    Range(String),
}

/// magit-diff--range-to-endpoints: "A...B" -> "A..B", "A..B" -> "A...B".
fn range_to_endpoints(range: &str) -> String {
    if range.contains("...") {
        range.replacen("...", "..", 1)
    } else if range.contains("..") {
        range.replacen("..", "...", 1)
    } else {
        range.to_string()
    }
}

/// magit--abbrev-if-oid: abbreviate REV when it is a full object id.
fn abbrev_if_oid(g: &Git, rev: &str) -> String {
    if rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()) {
        g.rev_short(rev)
    } else {
        rev.to_string()
    }
}

/// What a diff command acts on (magit-diff-type and magit-diff-scope).
#[derive(Debug, Clone, PartialEq)]
enum DiffSel {
    /// the untracked / unstaged / staged section
    List { typ: String },
    File { typ: String, path: String },
    Files { typ: String, paths: Vec<String> },
    Hunk { typ: String, path: String, hunk: usize },
    Hunks { typ: String, path: String, hunks: Vec<usize> },
    Region { typ: String, path: String, hunk: usize, from: usize, to: usize },
    /// the `chmod` / `rename` pseudo-hunk of a file
    Meta { typ: String, path: String, scope: Scope },
}

impl DiffSel {
    /// The diff group (`section` of the file lines); its diff type is
    /// `App::diff_type`.
    fn typ(&self) -> &str {
        match self {
            DiffSel::List { typ }
            | DiffSel::File { typ, .. }
            | DiffSel::Files { typ, .. }
            | DiffSel::Hunk { typ, .. }
            | DiffSel::Hunks { typ, .. }
            | DiffSel::Region { typ, .. }
            | DiffSel::Meta { typ, .. } => typ,
        }
    }

    /// file, files or list scope
    fn is_files(&self) -> bool {
        matches!(self, DiffSel::List { .. } | DiffSel::File { .. } | DiffSel::Files { .. })
    }
}

/// extra helper on Git for running outside repo (init/clone)
impl Git {
    fn git_interactive_at(&self, dir: &std::path::Path, args: &[&str]) -> Result<()> {
        let st = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()?;
        if !st.success() {
            anyhow::bail!("git {} failed: {}", args.first().unwrap_or(&""), st);
        }
        Ok(())
    }
}

/// Magit extras: bisect / worktree / submodule / notes / ediff.
/// Kept out of `run_command`'s main dispatch so each feature stays together.
impl App {
    fn show(&mut self, title: &str, res: Result<String>) {
        match res {
            Ok(out) if out.trim().is_empty() => self.set_msg("(no output)"),
            Ok(out) => self.view_text = Some((title.into(), out)),
            Err(e) => self.set_msg(&format!("{e}")),
        }
    }

    fn done(&mut self, ok_msg: &str, res: Result<String>) {
        match res {
            Ok(out) => {
                let first = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                self.set_msg(if first.is_empty() { ok_msg } else { first });
                self.refresh();
            }
            Err(e) => self.set_msg(&format!("{e}")),
        }
    }

    fn run_extra(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        match cmd {
            // ---- bisect
            "bisect-start" => {
                let Some(bad) = self.read_line_prompt("Bisect bad revision", Some("HEAD"))? else {
                    return Ok(());
                };
                let good = self.read_line_prompt("Bisect good revisions (space separated)", None)?;
                let goods: Vec<String> = good
                    .as_deref()
                    .unwrap_or("")
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                let res = self.git.bisect_start(Some(&bad), &goods);
                self.done("bisect started", res);
            }
            "bisect-bad" | "bisect-good" | "bisect-skip" => {
                let state = match cmd {
                    "bisect-bad" => "bad",
                    "bisect-good" => "good",
                    _ => "skip",
                };
                let rev = self.read_line_prompt(&format!("Bisect {state} revision"), Some("HEAD"))?;
                let res = self.git.bisect_mark(state, rev.as_deref());
                self.done(&format!("bisect {state}"), res);
            }
            "bisect-reset" => {
                let rev = self.read_line_prompt("Reset to revision (empty = pre-bisect HEAD)", None)?;
                let res = self.git.bisect_reset(rev.as_deref());
                self.done("bisect reset", res);
            }
            "bisect-log" => {
                let res = self.git.bisect_log();
                self.show("bisect log", res);
            }
            "bisect-run" | "bisect-start-run" => {
                // magit-bisect-run: when not bisecting yet, start first
                if cmd == "bisect-start-run" || !self.git.bisect_in_progress() {
                    let Some(bad) = self.read_line_prompt("Bisect bad revision", Some("HEAD"))? else {
                        return Ok(());
                    };
                    let Some(good) = self.read_line_prompt("Bisect good revision", None)? else {
                        return Ok(());
                    };
                    if let Err(e) = self.git.bisect_start(Some(&bad), &[good]) {
                        self.set_msg(&format!("{e}"));
                        return Ok(());
                    }
                }
                let Some(command) = self.read_line_prompt("Bisect run shell command", None)? else {
                    return Ok(());
                };
                let res = self.git.bisect_run(&command);
                self.show("bisect run", res);
                self.refresh();
            }

            // ---- worktree
            "worktree-list" => {
                let res = self.git.worktree_list();
                self.show("worktrees", res);
            }
            "worktree-checkout" | "worktree-branch" => {
                let Some(path) = self.read_line_prompt("Worktree path", None)? else {
                    return Ok(());
                };
                let is_new = cmd == "worktree-branch";
                let branch = self.read_line_prompt(
                    if is_new { "New branch name" } else { "Checkout branch/revision (empty = HEAD)" },
                    None,
                )?;
                let res = self.git.worktree_add(&path, branch.as_deref(), is_new, None);
                self.done("worktree added", res);
            }
            "worktree-move" => {
                let paths = self.git.worktree_paths();
                let Some(from) = self.pick("Move worktree", &paths, None)?.cloned() else {
                    return Ok(());
                };
                let Some(to) = self.read_line_prompt("New path", None)? else {
                    return Ok(());
                };
                let res = self.git.worktree_move(&from, &to);
                self.done("worktree moved", res);
            }
            "worktree-delete" => {
                let paths = self.git.worktree_paths();
                let Some(path) = self.pick("Delete worktree", &paths, None)?.cloned() else {
                    return Ok(());
                };
                if !self.yes_no(&format!("Delete worktree {path}?"))? {
                    return Ok(());
                }
                let force = self.yes_no("Force delete (dirty worktree)?")?;
                let res = self.git.worktree_remove(&path, force);
                self.done("worktree removed", res);
            }
            "worktree-prune" => {
                let res = self.git.worktree_prune();
                self.done("worktrees pruned", res);
            }

            // ---- submodule
            "submodule-list" => {
                let res = self.git.submodule_list();
                self.show("submodules", res);
            }
            "submodule-add" => {
                let Some(url) = self.read_line_prompt("Submodule url", None)? else {
                    return Ok(());
                };
                let Some(path) = self.read_line_prompt("Submodule path", None)? else {
                    return Ok(());
                };
                let res = self.git.submodule_add(&url, &path);
                self.done("submodule added", res);
            }
            // magit-submodule-register / populate / update / unpopulate:
            // act on one module or on all of them ("(all)")
            "submodule-register" | "submodule-populate" | "submodule-update" | "submodule-unpopulate" => {
                let Some(modules) = self.pick_submodules(cmd != "submodule-unpopulate")? else {
                    return Ok(());
                };
                let res = match cmd {
                    "submodule-register" => self.git.submodule_register(&modules),
                    "submodule-populate" => self.git.submodule_populate(args, &modules),
                    "submodule-update" => self.git.submodule_update_with(args, &modules),
                    _ => self.git.submodule_unpopulate(args, &modules),
                };
                self.done(&cmd.replace('-', " "), res);
            }
            "submodule-sync" => {
                let res = self.git.submodule_sync();
                self.done("submodules synced", res);
            }
            "submodule-fetch" => {
                let res = self.git.fetch_modules(args);
                self.done("fetched modules", res);
            }
            "submodule-remove" => {
                let paths = self.git.submodule_paths();
                let Some(path) = self.pick("Remove submodule", &paths, None)?.cloned() else {
                    return Ok(());
                };
                if !self.yes_no(&format!("Remove submodule {path}?"))? {
                    return Ok(());
                }
                let res = self.git.submodule_remove(&path);
                self.done("submodule removed", res);
            }

            // ---- notes
            "notes-list" => {
                let res = self.git.notes_list();
                self.show("notes", res);
            }
            "notes-edit" => {
                let Some(rev) = self.read_line_prompt("Note for revision", Some("HEAD"))? else {
                    return Ok(());
                };
                let Some(msg) = self.read_line_prompt("Note text (one line)", None)? else {
                    return Ok(());
                };
                let res = self.git.notes_add(&rev, &msg);
                self.done("note saved", res);
            }
            "notes-show" => {
                let Some(rev) = self.read_line_prompt("Show note for revision", Some("HEAD"))? else {
                    return Ok(());
                };
                let res = self.git.notes_show(&rev);
                self.show(&format!("notes {rev}"), res);
            }
            "notes-remove" => {
                let Some(rev) = self.read_line_prompt("Remove note from revision", Some("HEAD"))? else {
                    return Ok(());
                };
                if !self.yes_no(&format!("Remove note from {rev}?"))? {
                    return Ok(());
                }
                let res = self.git.notes_remove(&rev);
                self.done("note removed", res);
            }
            "notes-prune" => {
                let res = self.git.notes_prune();
                self.done("notes pruned", res);
            }

            // ---- ediff (text diff buffers instead of Emacs ediff sessions)
            "ediff-dwim" | "ediff-unstaged" | "ediff-staged" | "ediff-worktree" | "ediff-range"
            | "ediff-commit" | "ediff-stash" => {
                self.diff_view(&cmd["ediff-".len()..])?;
            }

            // ---- section info / buffer display
            // magit-describe-section
            "section-info" => {
                let line = self.current_line().cloned();
                let text = match line {
                    Some(l) => {
                        let s = self.cur_sec();
                        let sec = self.sec(s).clone();
                        let mut chain = Vec::new();
                        let mut cur = Some(s);
                        while let Some(c) = cur {
                            let id = &self.sec(c).ident;
                            chain.push(if id.is_empty() { "status".to_string() } else { id.clone() });
                            cur = self.sec(c).parent;
                        }
                        format!(
                            "Section: {}\nLines: {}-{}\nHidden: {}\nChildren: {}\nLine: {}\nKind: {:?}\nPosition: {}/{}",
                            chain.join(" < "),
                            sec.start + 1,
                            sec.end,
                            self.hidden(s),
                            sec.children.len(),
                            l.text,
                            l.kind,
                            self.cursor + 1,
                            self.lines.len(),
                        )
                    }
                    None => "No line at point".into(),
                };
                self.view_text = Some(("section info".into(), text));
            }
            "display-buffer" => {
                let mut out = format!("Status buffer ({} lines)\n\n", self.lines.len());
                for (i, l) in self.lines.iter().enumerate() {
                    out.push_str(&format!("{:>4}  {}\n", i + 1, l.text));
                }
                self.view_text = Some(("status buffer".into(), out));
            }
            "cherries" => self.cherry()?,

            // ---- subtree
            "subtree-prefixes" => {
                let prefixes = self.git.subtree_prefixes();
                let text = if prefixes.is_empty() {
                    "No subtree prefixes found".into()
                } else {
                    prefixes.join("\n")
                };
                self.view_text = Some(("subtree prefixes".into(), text));
            }
            "subtree-add" | "subtree-pull" | "subtree-push" => {
                let prefix = self.subtree_prefix_prompt()?;
                let Some(prefix) = prefix else { return Ok(()) };
                let Some(repo) = self.read_line_prompt("Subtree repository", None)? else {
                    return Ok(());
                };
                let Some(rev) = self.read_line_prompt("Subtree revision", Some("HEAD"))? else {
                    return Ok(());
                };
                let res = match cmd {
                    "subtree-add" => self.git.subtree_add(&prefix, &repo, &rev),
                    "subtree-pull" => self.git.subtree_pull(&prefix, &repo, &rev),
                    _ => self.git.subtree_push(&prefix, &repo, &rev),
                };
                self.done("subtree updated", res);
            }
            "subtree-split" => {
                let Some(prefix) = self.subtree_prefix_prompt()? else { return Ok(()) };
                let Some(branch) = self.read_line_prompt("Split into branch", None)? else {
                    return Ok(());
                };
                let res = self.git.subtree_split(&prefix, &branch);
                self.done("subtree split", res);
            }

            // ---- patches
            "patches-format" => {
                let Some(range) =
                    self.read_line_prompt("Format patches for range", Some("HEAD~3..HEAD"))?
                else {
                    return Ok(());
                };
                let Some(dir) = self.read_line_prompt("Output directory", Some("patches"))? else {
                    return Ok(());
                };
                let res = self.git.patch_format(&range, &dir);
                self.show("format-patch", res);
            }
            "patches-apply" | "patches-apply-worktree" => {
                let Some(path) = self.read_line_prompt("Patch file(s), space separated", None)? else {
                    return Ok(());
                };
                let files: Vec<String> = path.split_whitespace().map(str::to_string).collect();
                let res = if cmd == "patches-apply" {
                    self.git.am_apply(&files, args)
                } else {
                    match files.as_slice() {
                        [one] => self.git.patch_apply_worktree(one),
                        _ => Err(anyhow::anyhow!("git apply takes exactly one patch file")),
                    }
                };
                self.done("patch applied", res);
            }
            "patches-abort" | "patches-continue" | "patches-skip" | "patches-show" => {
                let res = match cmd {
                    "patches-abort" => self.git.patch_am_abort(),
                    "patches-continue" => self.git.patch_am_continue(),
                    "patches-skip" => self.git.am_skip(),
                    _ => self.git.patch_am_show(),
                };
                if cmd == "patches-show" {
                    self.show("current patch", res);
                } else {
                    self.done("git am", res);
                }
            }
            // magit-patch-save: write the diff of the diff / revision / stash
            // buffer shown to a file (magit-patch-save-arguments '(exclude "--stat"))
            "patch-save" => {
                let (range, typearg, files) = match &self.mode {
                    BufferMode::Diff(s) => (s.range.clone(), s.typearg.clone(), s.files.clone()),
                    BufferMode::Revision { rev, files } => (Some(format!("{rev}^..{rev}")), None, files.clone()),
                    BufferMode::Stash { stash } => (Some(format!("{stash}^..{stash}")), None, Vec::new()),
                    BufferMode::Status
                    | BufferMode::Log { .. }
                    | BufferMode::Reflog { .. }
                    | BufferMode::Stashes { .. }
                    | BufferMode::Cherry { .. } => {
                        self.set_msg("Only diff buffers can be saved as patches");
                        return Ok(());
                    }
                };
                let Some(file) = self.read_line_prompt("Write patch file", None)? else {
                    return Ok(());
                };
                let mut a: Vec<String> = vec!["diff".into()];
                a.extend(range);
                a.push("-p".into());
                a.extend(typearg);
                a.extend(self.buf_args.iter().filter(|x| *x != "--stat").cloned());
                a.push("--".into());
                a.extend(files);
                let a: Vec<&str> = a.iter().map(String::as_str).collect();
                let out = self.git.git_output(&a)?;
                let path = self.git.toplevel.join(&file);
                std::fs::write(&path, out.stdout)?;
                self.refresh();
            }
            "request-pull" => {
                let default_start = self.git.upstream_shortname();
                let Some(start) = self.read_line_prompt("Start (first commit not to include)", default_start.as_deref())? else {
                    return Ok(());
                };
                let default_url = self
                    .git
                    .remotes()
                    .first()
                    .and_then(|r| self.git.git_string(&["remote", "get-url", r]).ok());
                let Some(url) = self.read_line_prompt("Public repository url", default_url.as_deref())? else {
                    return Ok(());
                };
                let end = self.read_line_prompt("End (default HEAD)", None)?;
                let res = self.git.request_pull(&start, &url, end.as_deref());
                self.show("request-pull", res);
            }

            // ---- branch configuration
            "branch-config-show" => {
                let res = self.git.branch_config();
                self.show("branch configuration", res);
            }
            "branch-config-upstream" => {
                let default = self.git.upstream_shortname();
                let Some(up) = self.read_line_prompt("Upstream branch", default.as_deref())? else {
                    return Ok(());
                };
                let res = self.git.branch_set_upstream(&up);
                self.done("upstream set", res);
            }
            "branch-config-unset-upstream" => {
                let res = self.git.branch_unset_upstream();
                self.done("upstream unset", res);
            }
            // magit-branch.<branch>.rebase / magit-pull.rebase:
            // cycle true -> merges -> interactive -> false -> unset
            "branch-config-rebase" | "pull-rebase" => {
                let key = if cmd == "pull-rebase" {
                    "pull.rebase".to_string()
                } else {
                    match self.git.current_branch() {
                        Some(b) => format!("branch.{b}.rebase"),
                        None => {
                            self.set_msg("cannot configure a branch on detached HEAD");
                            return Ok(());
                        }
                    }
                };
                match self.git.cycle_config(&key, REBASE_CHOICES) {
                    Ok(v) => self.set_msg(&format!("{key} = {}", v.as_deref().unwrap_or("(unset)"))),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
            }
            // magit-branch.autoSetupMerge / magit-branch.autoSetupRebase ("b C a m" / "a r")
            "branch-autosetup-merge" | "branch-autosetup-rebase" => {
                let (key, choices): (&str, &[&str]) = if cmd == "branch-autosetup-merge" {
                    ("branch.autoSetupMerge", AUTOSETUP_MERGE_CHOICES)
                } else {
                    ("branch.autoSetupRebase", AUTOSETUP_REBASE_CHOICES)
                };
                match self.git.cycle_config(key, choices) {
                    Ok(v) => self.set_msg(&format!("{key} = {}", v.as_deref().unwrap_or("(unset)"))),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
            }
            "update-default-branch" => self.update_default_branch()?,
            "patches-apply-maildir" => {
                let Some(p) = self.read_line_prompt("Apply mbox or Maildir", None)? else {
                    return Ok(());
                };
                let path = self.cwd.join(&p);
                let res = self.git.am_apply(&[path.display().to_string()], args);
                self.done("patches applied", res);
            }
            "merge-absorb" | "merge-dissolve" => self.merge_absorb_cmd(cmd, args)?,
            "cherry-harvest" | "cherry-donate" | "cherry-spinout" | "cherry-spinoff" => {
                self.cherry_move_cmd(cmd, args)?
            }
            "rebase-interactive" | "rebase-edit-commit" | "rebase-reword-commit" | "rebase-remove-commit" => {
                let prompt = match cmd {
                    "rebase-interactive" => "Rebase this commit and all commits above it",
                    "rebase-edit-commit" => "Edit commit",
                    "rebase-reword-commit" => "Reword commit",
                    _ => "Remove commit",
                };
                let Some(commit) = self.pick_recent_commit(prompt)? else {
                    return Ok(());
                };
                // magit-rebase-remove-commit passes NOASSERT
                if cmd != "rebase-remove-commit"
                    && !self.rebase_assert(&commit, cmd != "rebase-reword-commit", args)?
                {
                    return Ok(());
                }
                let res = match cmd {
                    "rebase-interactive" => {
                        self.with_terminal(|g| g.rebase_interactive(&commit, args, Editor::User, Editor::User))
                    }
                    "rebase-edit-commit" => self.git.rebase_edit_commit(&commit, args),
                    "rebase-reword-commit" => {
                        self.with_terminal(|g| g.rebase_reword_commit(&commit, args, Editor::User))
                    }
                    _ => self.git.rebase_remove_commit(&commit, args),
                };
                self.after_rebase(res);
            }
            "rebase-subset" => {
                let default = self.git.upstream_shortname();
                let Some(newbase) = self.read_line_prompt("Rebase subset onto", default.as_deref())? else {
                    return Ok(());
                };
                let Some(start) =
                    self.pick_recent_commit(&format!("Rebase this commit and the commits above it onto {newbase}"))?
                else {
                    return Ok(());
                };
                let res = self.git.rebase_subset(&newbase, &format!("{start}^"), args);
                self.after_rebase(res);
            }
            "rebase-edit-todo" => {
                let res = self.with_terminal(|g| g.rebase_edit_todo(Editor::User));
                self.after_rebase(res);
            }
            "remote-pushdefault" => {
                let mut choices = self.git.remotes();
                choices.push("(unset)".into());
                let Some(r) = self.pick("remote.pushDefault", &choices, None)?.cloned() else {
                    return Ok(());
                };
                let value = (r != "(unset)").then_some(r.as_str());
                match self.git.set_config("remote.pushDefault", value) {
                    Ok(()) => self.set_msg(&format!("remote.pushDefault = {}", value.unwrap_or("(unset)"))),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
            }
            "branch-config-description" | "branch-config-pushremote" => {
                let (key, prompt) = if cmd == "branch-config-description" {
                    ("description", "Branch description")
                } else {
                    ("pushRemote", "pushRemote")
                };
                let Some(value) = self.read_line_prompt(prompt, None)? else {
                    return Ok(());
                };
                let res = self.git.branch_set_config(key, &value);
                self.done("branch configured", res);
            }

            _ => self.set_msg(&format!("unknown command: {cmd}")),
        }
        Ok(())
    }

    /// Pick a submodule, or "(all)" (empty list = every module) when allowed.
    fn pick_submodules(&mut self, allow_all: bool) -> Result<Option<Vec<String>>> {
        let mut choices = Vec::new();
        if allow_all {
            choices.push("(all)".to_string());
        }
        choices.extend(self.git.submodule_paths());
        if choices.is_empty() || (allow_all && choices.len() == 1) {
            self.set_msg("no submodules");
            return Ok(None);
        }
        Ok(self
            .pick("Submodule", &choices, None)?
            .cloned()
            .map(|c| if c == "(all)" { vec![] } else { vec![c] }))
    }

    /// Report the outcome of a rebase that may stop half-way.
    fn after_rebase(&mut self, res: Result<()>) {
        match res {
            Ok(()) if self.git.rebase_in_progress() => {
                self.set_msg("rebase stopped (r r continue, r s skip, r a abort)")
            }
            Ok(()) => self.set_msg("rebased"),
            Err(e) => self.set_msg(&format!("{e}")),
        }
        self.refresh();
    }

    /// magit-rebase-interactive-assert: confirm rewriting published commits
    /// and rebasing a range that contains merges. With `delay_edit_confirm`,
    /// do not ask when the published branches point exactly at COMMIT.
    fn rebase_assert(&mut self, commit: &str, delay_edit_confirm: bool, args: &[String]) -> Result<bool> {
        let since = match self.git.rebase_since(commit) {
            Ok(s) => s,
            Err(e) => {
                self.set_msg(&format!("{e}"));
                return Ok(false);
            }
        };
        let target = self.git.rev_parse(commit)?;
        let published: Vec<&str> = PUBLISHED_BRANCHES
            .iter()
            .copied()
            .filter(|b| self.git.rev_verify(b) && self.git.is_ancestor(commit, b))
            .collect();
        let all_at_commit = published
            .iter()
            .all(|b| self.git.rev_parse(b).ok().as_deref() == Some(target.as_str()));
        if !published.is_empty()
            && !(delay_edit_confirm && all_at_commit)
            && !self.yes_no(&format!(
                "Some of these commits have already been published to {}. Do you really want to modify them?",
                published.join(", ")
            ))?
        {
            self.set_msg("Abort");
            return Ok(false);
        }
        if !args.iter().any(|a| a.starts_with("--rebase-merges")) {
            let range = format!("{}..HEAD", since.as_deref().unwrap_or(commit));
            let merges = self.git.git_string(&["rev-list", "--merges", &range]).unwrap_or_default();
            if !merges.trim().is_empty() && !self.yes_no("Proceed despite merge in rebase range?")? {
                self.set_msg("Abort");
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// magit-merge-absorb ("m a") / magit-merge-dissolve ("m d")
    fn merge_absorb_cmd(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let current = self.git.current_branch();
        let others: Vec<String> = self
            .git
            .branches()
            .into_iter()
            .filter(|b| Some(b) != current.as_ref())
            .collect();
        let (absorbed, res) = if cmd == "merge-absorb" {
            let Some(branch) = self.pick("Absorb branch", &others, None)?.cloned() else {
                return Ok(());
            };
            if !self.confirm_absorb(&branch)? {
                return Ok(());
            }
            let res = self.git.merge_absorb(&branch, args);
            (branch, res)
        } else {
            let label = match &current {
                Some(c) => c.clone(),
                None => self.git.rev_parse("HEAD")?,
            };
            // default: the current branch's upstream when that is a local branch
            let default = current
                .as_ref()
                .and_then(|b| self.git.upstream_of(b))
                .filter(|(r, _)| r == ".")
                .map(|(_, b)| b);
            let Some(into) = self.pick(&format!("Merge `{label}' into"), &others, default.as_deref())?.cloned()
            else {
                return Ok(());
            };
            if current.is_some() && !self.confirm_absorb(&label)? {
                return Ok(());
            }
            let res = self.git.merge_dissolve(&into, args);
            (label, res)
        };
        match res {
            Ok(()) => self.set_msg(&format!("merged and removed {absorbed}")),
            Err(e) => self.set_msg(&format!("{e}")),
        }
        self.refresh();
        Ok(())
    }

    /// magit--merge-absorb asks before merging the main branch into another one.
    fn confirm_absorb(&mut self, branch: &str) -> Result<bool> {
        if self.git.main_branch().as_deref() == Some(branch)
            && !self.yes_no(&format!("Do you really want to merge `{branch}' into another branch?"))?
        {
            self.set_msg("Abort");
            return Ok(false);
        }
        Ok(true)
    }

    /// magit-cherry-harvest / -donate / -spinout / -spinoff ("A h/d/n/s")
    fn cherry_move_cmd(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let verb = &cmd["cherry-".len()..];
        let away = cmd != "cherry-harvest";
        let current = match self.git.current_branch() {
            Some(c) => c,
            None if cmd == "cherry-donate" => self.git.rev_parse("HEAD")?,
            None => {
                self.set_msg(&format!("Cannot {verb} cherries while HEAD is detached"));
                return Ok(());
            }
        };
        let Some(spec) = self.read_line_prompt(&format!("{verb} cherry (commit or A..B)"), None)? else {
            return Ok(());
        };
        let commits = match self.git.resolve_cherries(&spec) {
            Ok(c) => c,
            Err(e) => {
                self.set_msg(&format!("{e}"));
                return Ok(());
            }
        };
        let reachable = self.git.is_ancestor(&commits[0], &current);
        if away != reachable {
            let are = if reachable { "are" } else { "are not" };
            self.set_msg(&format!("Cannot {verb} cherries that {are} reachable from HEAD"));
            return Ok(());
        }
        let n = commits.len();
        let cherries = if n == 1 { "1 cherry".to_string() } else { format!("{n} cherries") };
        let res = match cmd {
            "cherry-harvest" => {
                let branches = self.git.branches_containing(&commits[0]);
                let src = match branches.len() {
                    0 => None,
                    1 => Some(branches[0].clone()),
                    _ => match self.pick(&format!("Remove {cherries} from branch"), &branches, None)? {
                        Some(b) => Some(b.clone()),
                        None => return Ok(()),
                    },
                };
                self.git.cherry_move(&commits, src.as_deref(), &current, args, None, true)
            }
            "cherry-donate" => {
                let others: Vec<String> =
                    self.git.branches().into_iter().filter(|b| *b != current).collect();
                let Some(dst) = self.pick(&format!("Move {cherries} to branch"), &others, None)?.cloned() else {
                    return Ok(());
                };
                self.git.cherry_move(&commits, Some(&current), &dst, args, None, false)
            }
            _ => {
                let Some(name) =
                    self.read_line_prompt(&format!("Create branch from {cherries} named"), None)?
                else {
                    return Ok(());
                };
                if self.git.local_branch_exists(&name) {
                    self.set_msg(&format!("branch {name} already exists"));
                    return Ok(());
                }
                let default_start = self.git.upstream_shortname().unwrap_or_else(|| current.clone());
                let Some(start) = self.read_line_prompt(
                    &format!("Create branch from {cherries} starting at"),
                    Some(&default_start),
                )?
                else {
                    return Ok(());
                };
                let checkout = cmd == "cherry-spinoff";
                self.git.cherry_move(&commits, Some(&current), &name, args, Some(&start), checkout)
            }
        };
        match res {
            Ok(()) => self.set_msg(&format!("{verb}: moved {cherries}")),
            Err(e) => self.set_msg(&format!("{e} (resolve, then A A to continue)")),
        }
        self.refresh();
        Ok(())
    }

    /// magit-update-default-branch ("b C B")
    fn update_default_branch(&mut self) -> Result<()> {
        let old = match self.git.default_branch(false) {
            Ok((_, o)) => o,
            Err(e) => {
                self.set_msg(&format!("{e}"));
                return Ok(());
            }
        };
        let (remote, new) = match self.git.default_branch(true) {
            Ok((r, Some(n))) => (r, n),
            Ok((_, None)) => {
                self.set_msg("Cannot determine new default branch");
                return Ok(());
            }
            Err(e) => {
                self.set_msg(&format!("{e}"));
                return Ok(());
            }
        };
        let old = if old.as_deref() == Some(new.as_str()) {
            let Some(o) = self.read_line_prompt(
                &format!(
                    "Name of default branch is still `{new}', but the upstreams of some local branches \
                     might need updating. Name of upstream branches to replace with `{new}'"
                ),
                None,
            )?
            else {
                return Ok(());
            };
            o
        } else {
            let old = match old {
                Some(o) => o,
                None => {
                    let others: Vec<String> = self.git.branches().into_iter().filter(|b| *b != new).collect();
                    let Some(o) = self
                        .pick(&format!("Name of old default branch to be renamed to `{new}'"), &others, Some("master"))?
                        .cloned()
                    else {
                        return Ok(());
                    };
                    o
                }
            };
            if !self.yes_no(&format!(
                "Default branch changed from `{old}' to `{new}' on {remote}.  Do the same locally?"
            ))? {
                self.set_msg("Abort");
                return Ok(());
            }
            old
        };
        match self.git.set_default_branch(&new, &old) {
            Ok(()) => self.set_msg(&format!("default branch is now {new}")),
            Err(e) => self.set_msg(&format!("{e}")),
        }
        self.refresh();
        Ok(())
    }

    /// Pick a known subtree prefix, or ask for one when none is recorded.
    fn subtree_prefix_prompt(&mut self) -> Result<Option<String>> {
        let prefixes = self.git.subtree_prefixes();
        if prefixes.is_empty() {
            self.read_line_prompt("Subtree prefix", None)
        } else {
            Ok(self.pick("Subtree prefix", &prefixes, None)?.cloned())
        }
    }
}

/// Program entry helper: launch the TUI for cwd (or first arg path).
pub fn main_run(path: Option<&str>) -> Result<()> {
    let dir = std::path::PathBuf::from(path.unwrap_or("."));
    let abs = dir.canonicalize().unwrap_or(dir);
    let git = Git::discover(&abs)?;
    let mut app = App::new(git);
    if abs.is_dir() {
        app.cwd = abs;
    }
    app.run()
}
