//! ui/app.rs — application state machine, event loop, prompts and command dispatch.

use super::keys::Key;
use super::blame::{blob_ancestor, blob_successor, build_file, Blame, BlameType, STYLES};
use super::transient::{
    blame_menu, diff_refresh_menu, file_dispatch_menu, log_refresh_menu, prefix_value, registry, resolve_menu,
    show_refs_menu, Action,
    Lookup, Menu, MenuEntry, Reader, RepoState, VALUE_COMMANDS,
};
use super::diffbuf::{build_diff, build_revision, build_stash, DiffSpec};
use super::logbuf::{
    build_cherry, build_log, build_reflog, build_stashes, set_commit_limit, Limit, Margin, REFLOG_LIMIT,
};
use super::refs::{build_refs, deferred_ref, CommitCount, RefKind};
use super::view::{build_status, status_log_args, BufferLine, LineKind, Section, StatusBuffer, Visibility};
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
use std::collections::{BTreeMap, HashMap};
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
    /// log arguments of the status buffer (its magit-buffer-log-args)
    status_log_args: Vec<String>,
    /// the current value of magit-log-select-mode's log arguments
    log_select_args: Vec<String>,
    /// buffers below the one shown (the status buffer under a diff buffer)
    buried: Vec<SavedBuffer>,
    /// buffers buried with `q`: not below any other, but still around
    /// (at the end of the buffer list, the last buried last)
    background: Vec<SavedBuffer>,
    /// the arguments of the `d` transient while one of its suffixes runs
    /// (magit-diff-arguments when transient-current-command is magit-diff)
    transient_diff_args: Option<Vec<String>>,
    /// the same for the `l` transient (magit-log-arguments)
    transient_log_args: Option<Vec<String>>,
    /// C-c was typed (the prefix of C-c C-n)
    cc_prefix: bool,
    /// the prefix argument of the next command (current-prefix-arg)
    prefix_arg: Option<PrefixArg>,
    /// C-u / M-- was typed and digits are read (universal-argument-map)
    reading_arg: bool,
    /// magit--right-margin-config of the buffer of each mode (permanent)
    margins: HashMap<&'static str, Margin>,
    /// magit-refs-show-commit-count of the refs buffer
    refs_counts: CommitCount,
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
    /// the prompts answered so far (tests)
    prompts: std::cell::RefCell<Vec<String>>,
    /// the TUI owns the terminal (`run`): it is handed over to editors
    in_terminal: bool,
    /// values of transient prefixes set for this session (transient-set)
    set_values: HashMap<String, Vec<String>>,
    /// transient-values: the saved values (transient-save), kept in
    /// `values_file`
    saved_values: BTreeMap<String, Vec<String>>,
    /// transient-values-file (none: values are not saved to a file)
    values_file: Option<std::path::PathBuf>,
    /// transient-history: the values used by each prefix, latest first
    history: HashMap<String, Vec<Vec<String>>>,
    /// the history of the transient shown and the position in it
    menu_history: (Vec<Vec<String>>, usize),
}

/// The major mode of a buffer: magit-status-mode, magit-diff-mode,
/// magit-revision-mode, magit-stash-mode, magit-log-mode,
/// magit-reflog-mode, magit-stashes-mode, magit-cherry-mode or
/// magit-log-select-mode.
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
    /// magit-log-select: the log of REV, from which a commit is picked
    /// for ACTION (magit-log-select-pick-function); MSG is the usage
    LogSelect { rev: String, action: SelectAction, msg: String },
    /// a buffer visiting FILE at REV ("{worktree}", "{index}" or a
    /// commit: magit-blob-mode), maybe blamed (magit-blame-mode)
    File { rev: String, file: String, blame: Option<Blame> },
    /// magit-refs-mode: the refs compared with UPSTREAM
    /// (magit-buffer-refs-upstream)
    Refs { upstream: String },
}

/// What is done with the commit picked in a log-select buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectAction {
    /// magit-rebase-interactive / -edit-commit / -reword-commit /
    /// -remove-commit (CMD is the ngit command)
    Rebase { cmd: String, args: Vec<String> },
    /// magit-rebase-subset onto NEWBASE
    RebaseSubset { newbase: String, args: Vec<String> },
    /// magit-rebase-autosquash
    RebaseAutosquash { args: Vec<String> },
    /// magit-commit-squash-internal with OPTION ("--fixup=", "--squash=",
    /// "--fixup=amend:" or "--fixup=reword:")
    CommitSquash { option: String, args: Vec<String>, edit: bool, rebase: bool },
}

impl BufferMode {
    /// Magit has one buffer per mode and repository (and one per visited
    /// file or blob).
    fn same_kind(&self, other: &BufferMode) -> bool {
        match (self, other) {
            (BufferMode::File { rev: a, file: f, .. }, BufferMode::File { rev: b, file: g, .. }) => a == b && f == g,
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }

    /// magit-log-mode and the modes derived from it with its log
    /// arguments (magit-log-select-mode)
    fn is_log(&self) -> bool {
        matches!(self, BufferMode::Log { .. } | BufferMode::LogSelect { .. })
    }

    /// magit--right-margin-option: the buffers with a right margin.
    fn margin_option(&self) -> Option<&'static str> {
        Some(match self {
            BufferMode::Status => "status",
            BufferMode::Log { .. } => "log",
            BufferMode::LogSelect { .. } => "log-select",
            BufferMode::Reflog { .. } => "reflog",
            BufferMode::Stashes { .. } => "stashes",
            BufferMode::Cherry { .. } => "cherry",
            BufferMode::Refs { .. } => "refs",
            _ => return None,
        })
    }
}

/// regexp-quote
fn regex_quote(s: &str) -> String {
    s.chars().fold(String::new(), |mut out, c| {
        if "[*.\\?+^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

/// The default of the margin option NAME (magit-log-margin and friends).
fn default_margin(name: &str) -> Margin {
    match name {
        "status" => Margin::status(),
        "reflog" | "stashes" => Margin::reflog(),
        // magit-refs-margin: hidden, without the author
        "refs" => Margin::status(),
        _ => Margin::log(),
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
/// The kind of the section at point in the refs buffer, for the
/// commands remapped in its section keymaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefThing {
    Branch,
    Tag,
    Remote,
}

/// A prefix argument (Emacs' raw prefix argument): `C-u` typed N times
/// is Raw(4^N), `C-u 12` is Num(12), `C-u -` / `M--` is Minus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixArg {
    Raw(i64),
    Num(i64),
    Minus,
}

impl PrefixArg {
    /// prefix-numeric-value
    pub fn numeric(self) -> i64 {
        match self {
            PrefixArg::Raw(n) | PrefixArg::Num(n) => n,
            PrefixArg::Minus => -1,
        }
    }

    /// universal-argument--description, as echoed while it is read.
    fn description(self) -> String {
        match self {
            PrefixArg::Raw(n) if n > 0 => {
                let mut c = vec!["C-u"];
                let mut m = n;
                while m > 4 {
                    c.push("C-u");
                    m /= 4;
                }
                c.join(" ")
            }
            PrefixArg::Raw(n) => format!("C-u {n}"),
            PrefixArg::Num(n) => format!("C-u {n}"),
            PrefixArg::Minus => "C-u -".into(),
        }
    }
}

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
        "R" => Command("rename-thing"),
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
            status_log_args: status_log_args(),
            log_select_args: vec!["--graph".into(), "-n256".into(), "--decorate".into()],
            buried: Vec::new(),
            background: Vec::new(),
            transient_diff_args: None,
            transient_log_args: None,
            cc_prefix: false,
            prefix_arg: None,
            reading_arg: false,
            margins: HashMap::new(),
            refs_counts: CommitCount::Nothing,
            message: String::new(),
            quit: false,
            view_text: None,
            view_scroll: 0,
            cwd,
            diff_args: vec!["--stat".into(), "--no-ext-diff".into()],
            log_args: vec!["--graph".into(), "-n256".into(), "--decorate".into()],
            scripted: std::cell::RefCell::new(std::collections::VecDeque::new()),
            prompts: std::cell::RefCell::new(Vec::new()),
            in_terminal: false,
            set_values: HashMap::new(),
            saved_values: BTreeMap::new(),
            values_file: None,
            history: HashMap::new(),
            menu_history: (Vec::new(), 0),
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
    pub fn buffer_args(&self) -> Vec<String> {
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

    /// Whether ngit is about to exit (the last buffer was buried).
    pub fn quitting(&self) -> bool {
        self.quit
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
        let margin = self.margin_config();
        match &self.mode {
            BufferMode::Status => {
                let st = Status::load(&self.git);
                build_status(&self.git, &st, &self.status_diff_args(), &self.status_log_args, &margin)
            }
            BufferMode::Diff(spec) => build_diff(&self.git, spec, &self.buf_args),
            BufferMode::Revision { rev, files } => build_revision(&self.git, rev, &self.buf_args, files),
            BufferMode::Stash { stash } => build_stash(&self.git, stash, &self.buf_args),
            BufferMode::Log { revs, files } => build_log(&self.git, revs, &self.buf_args, files, &margin),
            BufferMode::LogSelect { rev, msg, .. } => {
                // magit-log-select-refresh-buffer; the header line shows
                // the usage (magit-log-select-show-usage)
                let mut buf = build_log(&self.git, std::slice::from_ref(rev), &self.buf_args, &[], &margin);
                buf.title = msg.clone();
                buf
            }
            BufferMode::Reflog { refname } => build_reflog(&self.git, refname, &self.buf_args, &margin),
            BufferMode::Stashes { refname } => build_stashes(&self.git, refname, &margin),
            BufferMode::Cherry { head, upstream } => build_cherry(&self.git, head, upstream, &margin),
            BufferMode::File { rev, file, blame } => build_file(&self.git, rev, file, blame.as_ref()),
            BufferMode::Refs { upstream } => {
                build_refs(&self.git, upstream, &self.buf_args, self.refs_counts, &self.vis, &margin)
            }
        }
    }

    /// magit--right-margin-config of the buffer shown (the default of its
    /// margin option until it is changed).
    pub fn margin_config(&self) -> Margin {
        let name = self.mode.margin_option().unwrap_or("log");
        self.margins.get(name).cloned().unwrap_or_else(|| default_margin(name))
    }

    /// The margin commands of magit-margin-settings and magit-log-refresh.
    fn margin_command(&mut self, cmd: &str) {
        let Some(name) = self.mode.margin_option() else {
            self.set_msg("Magit margin isn't supported in this buffer");
            return;
        };
        let mut m = self.margin_config();
        match cmd {
            "margin-toggle" => m.enable = !m.enable,
            "margin-cycle-style" => m.cycle_style(),
            "margin-toggle-details" => m.details = !m.details,
            "margin-toggle-shortstat" => m.shortstat = !m.shortstat,
            _ => return,
        }
        self.margins.insert(name, m);
        self.refresh();
    }

    /// magit-setup-buffer: show a buffer in MODE with ARGS, reusing the
    /// buffer of that mode (or burying the buffer shown).
    fn setup_buffer(&mut self, mode: BufferMode, args: Vec<String>) {
        self.view_text = None;
        let mut same = self.mode == mode;
        if !self.mode.same_kind(&mode) {
            let saved = self.take_current();
            self.buried.retain(|b| !b.mode.same_kind(&mode));
            self.buried.push(saved);
            // magit-mode-get-buffer: a buried buffer of MODE is reused,
            // with its sections' visibility and point
            if let Some(i) = self.background.iter().position(|b| b.mode == mode) {
                let b = self.background.remove(i);
                self.buf = b.buf;
                self.vis = b.vis;
                self.lines = self.buf.visible(&self.vis);
                self.cursor = b.cursor.min(self.lines.len().saturating_sub(1));
                self.scroll = b.scroll;
                same = true;
            }
            self.background.retain(|b| !b.mode.same_kind(&mode));
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

    /// The buffer shown, taken out (its mode left as Status).
    fn take_current(&mut self) -> SavedBuffer {
        SavedBuffer {
            mode: std::mem::replace(&mut self.mode, BufferMode::Status),
            args: std::mem::take(&mut self.buf_args),
            buf: std::mem::take(&mut self.buf),
            vis: std::mem::take(&mut self.vis),
            cursor: self.cursor,
            scroll: self.scroll,
        }
    }

    /// magit-mode-bury-buffer (q): with two prefix arguments kill all
    /// Magit buffers of the repository (and so quit), with one kill the
    /// buffer shown, else bury it (magit-mode-quit-window): the buffer
    /// below is shown and the buried one stays around (J, or setting up
    /// a buffer of its mode, shows it again).
    fn mode_bury_buffer(&mut self) {
        if self.prefix_numeric() >= 16 {
            self.view_text = None;
            self.buried.clear();
            self.background.clear();
            self.quit = true;
            return;
        }
        if self.prefix_arg.is_some() || self.view_text.is_some() || self.buried.is_empty() {
            self.kill_buffer();
            return;
        }
        let current = self.take_current();
        self.kill_buffer();
        self.background.retain(|b| !b.mode.same_kind(&current.mode));
        self.background.push(current);
    }

    /// Leave the text view, else kill the buffer shown and show the
    /// buffer below (refreshed), else quit.
    fn kill_buffer(&mut self) {
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
        // magit-insert-section-body: the bodies of the sections just shown
        let shown: Vec<String> =
            self.buf.deferred.iter().filter(|id| !self.buf.is_hidden(&self.vis, id)).cloned().collect();
        if !shown.is_empty() {
            let ident = self.buf.all.get(p).map(|l| l.ident.clone());
            self.buf = self.build_buffer();
            let p = ident.and_then(|id| self.buf.all.iter().position(|l| l.ident == id)).unwrap_or(p);
            self.lines = self.buf.visible(&self.vis);
            self.cursor = self.visible_index_near(p);
            for id in shown {
                if self.buf.sections.iter().any(|s| s.ident == id && s.end == s.start + 1) {
                    self.set_msg(&format!("No cherries for {}", deferred_ref(&id)));
                }
            }
            return;
        }
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
                // transient-common-commands are shown after C-x
                if self.pending.first().is_some_and(|k| k.ctrl && k.code == "x") {
                    popup.push("  Value commands".into());
                    popup.extend(VALUE_COMMANDS.iter().map(|(k, d)| format!("  {k:<8} {d}")));
                }
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

    /// The prompts read so far, oldest first.
    pub fn prompts(&self) -> Vec<String> {
        self.prompts.borrow().clone()
    }

    /// Answers consumed by prompts instead of reading the terminal (tests).
    pub fn script_answers(&mut self, answers: &[&str]) {
        self.scripted.borrow_mut().extend(answers.iter().map(|s| s.to_string()));
    }

    fn read_line_prompt(&self, prompt: &str, default: Option<&str>) -> Result<Option<String>> {
        Ok(self.read_line_raw(prompt, default)?.filter(|s| !s.is_empty()))
    }

    /// Read a line (the DEFAULT when empty); only EOF gives none.
    fn read_line_raw(&self, prompt: &str, default: Option<&str>) -> Result<Option<String>> {
        self.prompts.borrow_mut().push(prompt.to_string());
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
        self.in_terminal = true;
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
        if self.universal_argument(&key) {
            return Ok(());
        }
        let r = self.dispatch_key(key);
        // the prefix argument is for one command; prefix keys (C-c, the
        // first keys of a transient's key sequence) pass it on
        if !self.cc_prefix && self.pending.is_empty() {
            self.prefix_arg = None;
        }
        r
    }

    /// universal-argument (C-u), negative-argument (M--) and the digits,
    /// `-` and C-u of universal-argument-map. True if KEY was consumed.
    fn universal_argument(&mut self, key: &Key) -> bool {
        let arg = self.prefix_arg;
        let new = match (key.ctrl, key.code.as_str()) {
            (true, "u") if self.reading_arg => match arg {
                Some(PrefixArg::Raw(n)) => Some(PrefixArg::Raw(n * 4)),
                Some(PrefixArg::Minus) => Some(PrefixArg::Raw(-4)),
                // universal-argument-more after digits ends the argument
                _ => {
                    self.reading_arg = false;
                    return true;
                }
            },
            (true, "u") => Some(PrefixArg::Raw(4)),
            (false, "M--") => match arg {
                Some(PrefixArg::Num(n)) => Some(PrefixArg::Num(-n)),
                Some(PrefixArg::Minus) => None,
                _ => Some(PrefixArg::Minus),
            },
            _ if !self.reading_arg => return false,
            (true, "g") => {
                self.reading_arg = false;
                self.prefix_arg = None;
                self.set_msg("Quit");
                return true;
            }
            (false, d) if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
                let digit = i64::from(d.as_bytes()[0] - b'0');
                match arg {
                    Some(PrefixArg::Num(n)) => Some(PrefixArg::Num(n * 10 + if n < 0 { -digit } else { digit })),
                    Some(PrefixArg::Minus) if digit == 0 => Some(PrefixArg::Minus),
                    Some(PrefixArg::Minus) => Some(PrefixArg::Num(-digit)),
                    Some(PrefixArg::Raw(n)) if n < 0 => Some(PrefixArg::Num(-digit)),
                    _ => Some(PrefixArg::Num(digit)),
                }
            }
            // universal-argument-minus: after digits `-` is a command
            (false, "-") if !matches!(arg, Some(PrefixArg::Num(_))) => Some(PrefixArg::Minus),
            _ => {
                self.reading_arg = false;
                return false;
            }
        };
        self.prefix_arg = new;
        self.reading_arg = true;
        let desc = new.map_or_else(|| "C-u".to_string(), PrefixArg::description);
        self.set_msg(&format!("{desc}-"));
        true
    }

    /// current-prefix-arg
    pub fn current_prefix_arg(&self) -> Option<PrefixArg> {
        self.prefix_arg
    }

    /// (prefix-numeric-value current-prefix-arg)
    fn prefix_numeric(&self) -> i64 {
        self.prefix_arg.map_or(1, PrefixArg::numeric)
    }

    fn dispatch_key(&mut self, key: Key) -> Result<()> {
        // transient popup takes precedence
        if let Some((name, active)) = self.stack.last().cloned() {
            if let Some(menu) = self.menus.get(&name).cloned() {
                return self.handle_transient_key(key, menu, active);
            }
        }
        if std::mem::take(&mut self.cc_prefix) {
            let select = matches!(self.mode, BufferMode::LogSelect { .. });
            let blaming = matches!(self.mode, BufferMode::File { blame: Some(_), .. });
            match (key.ctrl, key.code.as_str()) {
                (true, "n") if self.mode.is_log() => self.log_move_to_parent(),
                (true, "c") if select => return self.log_select_pick(),
                (true, "k") if select => self.kill_buffer(),
                // magit-blame-mode-map
                (true, "q") if blaming => self.blame_quit(),
                // the global binding of magit-file-dispatch
                (false, "M-g") => self.open_menu("file-dispatch"),
                _ => self.set_msg(&format!("C-c {} is undefined", key.label())),
            }
            return Ok(());
        }
        if key.ctrl && key.code == "c" {
            self.cc_prefix = true;
            self.set_msg("C-c-");
            return Ok(());
        }
        // file and blob buffers
        if self.view_text.is_none() && self.file_key(&key)? {
            return Ok(());
        }
        // magit-log-select-mode-map
        if matches!(self.mode, BufferMode::LogSelect { .. }) && self.view_text.is_none() && !key.ctrl {
            match key.code.as_str() {
                "." | "e" => return self.log_select_pick(),
                "q" => {
                    self.kill_buffer();
                    return Ok(());
                }
                _ => {}
            }
        }
        // magit-log-mode-map (also the parent of magit-reflog-mode-map and
        // so of magit-stashes-mode-map, and of magit-log-select-mode-map)
        if (self.mode.is_log() || matches!(self.mode, BufferMode::Reflog { .. } | BufferMode::Stashes { .. }))
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
        // magit-refs-mode-map
        if matches!(self.mode, BufferMode::Refs { .. }) && self.view_text.is_none() && key.ctrl && key.code == "y" {
            return self.run_command("refs-set-show-commit-count", &BTreeMap::new());
        }
        // L is magit-margin-settings in magit-reflog-mode-map (and so in
        // magit-stashes-mode), magit-cherry-mode-map and magit-refs-mode-map
        if matches!(
            self.mode,
            BufferMode::Reflog { .. } | BufferMode::Stashes { .. } | BufferMode::Cherry { .. } | BufferMode::Refs { .. }
        )
            && self.view_text.is_none()
            && !key.ctrl
            && key.code == "L"
        {
            self.open_menu("margin-settings-menu");
            return Ok(());
        }
        // status buffer bindings (magit-status-mode-map)
        let Some(binding) = status_binding(&key) else {
            return Ok(());
        };
        // a file buffer has none of the Magit buffer bindings
        if matches!(self.mode, BufferMode::File { .. })
            && self.view_text.is_none()
            && !matches!(binding, Binding::Down | Binding::Up | Binding::SetMark | Binding::KeyboardQuit)
        {
            self.set_msg(&format!("{} is undefined", key.label()));
            return Ok(());
        }
        match binding {
            Binding::Bury => self.mode_bury_buffer(),
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
            // magit-log-infix-arguments only in magit-log-mode, the short
            // "Arguments" group elsewhere
            let full = matches!(self.mode, BufferMode::Log { .. });
            self.menus.insert(name.clone(), log_refresh_menu(full));
        }
        // suffixes with :if conditions on the visited file or blob
        let (in_file, blob, blaming) = match &self.mode {
            BufferMode::File { rev, blame, .. } => (true, rev != "{worktree}", blame.is_some()),
            _ => (false, false, false),
        };
        if name == "show-refs-menu" {
            let refs = matches!(self.mode, BufferMode::Refs { .. });
            self.menus.insert(name.clone(), show_refs_menu(refs));
        }
        if name == "blame-menu" {
            self.menus.insert(name.clone(), blame_menu(blob, blaming));
        }
        if name == "file-dispatch" {
            let status = self.mode == BufferMode::Status;
            self.menus.insert(name.clone(), file_dispatch_menu(in_file, blaming, status));
        }
        // `d` starts with the arguments of the diff buffer shown, else the
        // current value of magit-diff-mode; `D` with the buffer's arguments.
        // Other prefixes: the set, else the saved, else the default value.
        let key = self.value_key(&name);
        let default: Vec<String> = match self.set_values.get(&key).or(self.saved_values.get(&key)) {
            Some(v) => v.clone(),
            None => prefix_value(&key).iter().map(|s| s.to_string()).collect(),
        };
        let preset: &[String] = match (name.as_str(), &self.mode) {
            ("diff-menu", BufferMode::Diff(_)) => &self.buf_args,
            ("diff-menu", _) => &self.diff_args,
            ("diff-refresh-menu", BufferMode::Status) => &self.status_args,
            ("diff-refresh-menu", _) => &self.buf_args,
            ("log-menu" | "log-refresh-menu", m) if m.is_log() => &self.buf_args,
            ("log-refresh-menu", BufferMode::Status) => &self.status_log_args,
            ("log-menu" | "log-refresh-menu", _) => &self.log_args,
            // magit-show-refs-arguments: the refs buffer's
            ("show-refs-menu", BufferMode::Refs { .. }) => &self.buf_args,
            _ => &default,
        };
        let active: BTreeMap<String, bool> = preset.iter().map(|a| (a.clone(), true)).collect();
        // transient--history-init: the current value, then those used before
        let value = preset.to_vec();
        let mut hist = vec![value.clone()];
        hist.extend(self.history.get(&key).into_iter().flatten().filter(|v| **v != value).cloned());
        self.menu_history = (hist, 0);
        self.pending.clear();
        self.stack.push((name, active));
    }

    /// Where the value of the prefix NAME is kept: the prefix itself
    /// (its sequencer-in-progress variant shares it), or for the diff and
    /// log prefixes the arguments of a major mode (magit-diff--set-value,
    /// magit-log--set-value).
    fn value_key(&self, name: &str) -> String {
        let base = name.split('@').next().unwrap_or(name);
        match base {
            "diff-menu" => "magit-diff:magit-diff-mode",
            "diff-refresh-menu" if self.mode == BufferMode::Status => "magit-diff:magit-status-mode",
            "diff-refresh-menu" => "magit-diff:magit-diff-mode",
            "log-refresh-menu" if self.mode == BufferMode::Status => "magit-log:magit-status-mode",
            "log-refresh-menu" if matches!(self.mode, BufferMode::LogSelect { .. }) => "magit-log:magit-log-select-mode",
            "log-menu" | "log-refresh-menu" => "magit-log:magit-log-mode",
            b => b,
        }
        .to_string()
    }

    /// The default arguments of a major mode, if KEY is one.
    fn mode_args(&mut self, key: &str) -> Option<&mut Vec<String>> {
        match key {
            "magit-diff:magit-status-mode" => Some(&mut self.status_args),
            "magit-diff:magit-diff-mode" => Some(&mut self.diff_args),
            "magit-log:magit-log-mode" => Some(&mut self.log_args),
            "magit-log:magit-status-mode" => Some(&mut self.status_log_args),
            "magit-log:magit-log-select-mode" => Some(&mut self.log_select_args),
            _ => None,
        }
    }

    /// The value of the prefix or major mode KEY before anything is set.
    fn initial_value(key: &str) -> Vec<String> {
        let v: &[&str] = match key {
            "magit-diff:magit-status-mode" => &["--no-ext-diff", "--ignore-submodules=none"],
            "magit-diff:magit-diff-mode" => &["--stat", "--no-ext-diff"],
            "magit-log:magit-log-mode" | "magit-log:magit-log-select-mode" => &["--graph", "-n256", "--decorate"],
            "magit-log:magit-status-mode" => &["-n256", "--decorate"],
            k => prefix_value(k),
        };
        v.iter().map(|s| s.to_string()).collect()
    }

    /// transient-values-file: read the saved values, including the default
    /// arguments of the diff and log modes, and save to FILE from now on.
    pub fn load_values(&mut self, file: std::path::PathBuf) {
        self.saved_values = super::transient::read_values(&file);
        self.values_file = Some(file);
        let saved: Vec<(String, Vec<String>)> = self.saved_values.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (k, v) in saved {
            if let Some(slot) = self.mode_args(&k) {
                *slot = v;
            }
        }
        self.refresh();
    }

    fn save_values(&mut self) {
        if let Some(f) = &self.values_file {
            if let Err(e) = super::transient::write_values(f, &self.saved_values) {
                self.set_msg(&format!("Cannot save {}: {e}", f.display()));
            }
        }
    }

    /// transient--history-push
    fn history_push(&mut self, key: &str, value: Vec<String>) {
        let h = self.history.entry(key.to_string()).or_default();
        h.retain(|v| *v != value);
        h.insert(0, value);
    }

    /// transient-set (C-x s) / transient-save (C-x C-s): the value of the
    /// transient shown becomes the value of the prefix for this session,
    /// and with SAVE for future ones. For the diff and log prefixes it
    /// becomes the arguments of the mode and of the buffer, which is
    /// refreshed.
    fn transient_set_value(&mut self, save: bool) {
        let Some((name, active)) = self.stack.last().cloned() else {
            return;
        };
        let args = self.args_from(&active);
        let key = self.value_key(&name);
        if let Some(slot) = self.mode_args(&key) {
            *slot = args.clone();
            let diff_buffer = matches!(self.mode, BufferMode::Diff(_) | BufferMode::Revision { .. } | BufferMode::Stash { .. });
            if key.starts_with("magit-diff:") {
                if self.mode == BufferMode::Status {
                    self.status_args = args.clone();
                } else if diff_buffer {
                    self.buf_args = args.clone();
                }
            } else if self.mode.is_log() {
                self.buf_args = args.clone();
            }
            self.refresh();
        } else {
            self.set_values.insert(key.clone(), args.clone());
        }
        if save {
            self.saved_values.insert(key.clone(), args.clone());
            self.save_values();
        }
        self.history_push(&key, args);
    }

    /// transient-reset (C-x C-k): forget the set and saved values; the
    /// transient shown gets the default value.
    fn transient_reset(&mut self) {
        let Some((name, _)) = self.stack.last().cloned() else {
            return;
        };
        let key = self.value_key(&name);
        let value = Self::initial_value(&key);
        self.set_values.remove(&key);
        if let Some(slot) = self.mode_args(&key) {
            *slot = value.clone();
        }
        if self.saved_values.remove(&key).is_some() {
            self.save_values();
        }
        self.history_push(&key, value.clone());
        self.set_active(value);
    }

    /// transient-history-prev (C-x p, OLDER) / -next (C-x n).
    fn transient_history(&mut self, older: bool) {
        let (hist, pos) = &self.menu_history;
        let pos = if older { pos + 1 } else { pos.wrapping_sub(1) };
        let Some(value) = hist.get(pos).cloned() else {
            self.set_msg("End of history");
            return;
        };
        self.menu_history.1 = pos;
        self.set_active(value);
    }

    fn set_active(&mut self, value: Vec<String>) {
        if let Some(top) = self.stack.last_mut() {
            top.1 = value.into_iter().map(|a| (a, true)).collect();
        }
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

    /// The keys of the suffixes and infixes of the transient shown.
    pub fn menu_keys(&self) -> Vec<String> {
        self.current_menu()
            .and_then(|n| self.menus.get(n))
            .map(|m| m.groups.iter().flatten().map(|e| e.key.clone()).collect())
            .unwrap_or_default()
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
        if !self.in_terminal {
            return f(&self.git);
        }
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
        // transient-common-commands: the value commands under C-x
        let cx = Key { code: "x".into(), ctrl: true };
        if self.pending == [cx.clone()] {
            self.pending.clear();
            match (key.ctrl, key.code.as_str()) {
                (false, "s") => self.transient_set_value(false),
                (true, "s") => self.transient_set_value(true),
                (true, "k") => self.transient_reset(),
                (false, "p") => self.transient_history(true),
                (false, "n") => self.transient_history(false),
                // transient-quit-seq
                (true, "g") => {}
                _ => self.set_msg(&format!("C-x {} is undefined", key.label())),
            }
            return Ok(());
        }
        if key == cx && self.pending.is_empty() {
            self.pending.push(key);
            return Ok(());
        }
        // q quits the transient unless it is a suffix (magit-blame's
        // "Quit blaming")
        let q_suffix = key.code == "q" && self.pending.is_empty() && menu.find(&key).is_some();
        if (key.code == "q" && !q_suffix) || key.code == "esc" || (key.ctrl && key.code == "g") {
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
            // the margin commands are transient: the menu stays
            Action::Command(cmd) if cmd.starts_with("margin-") => self.margin_command(&cmd),
            Action::Command(cmd) if cmd == "blame-cycle-style" => self.blame_cycle_style(),
            // magit-diff-refresh / magit-log-refresh "s" and "w": refresh
            // the buffer and set or save the defaults
            Action::Command(cmd) if cmd == "transient-set-and-exit" || cmd == "transient-save-and-exit" => {
                self.transient_set_value(cmd == "transient-save-and-exit");
                self.stack.pop();
            }
            Action::Command(cmd) => {
                self.stack.pop();
                // transient--export
                let key = self.value_key(&menu.name);
                self.history_push(&key, self.args_from(&active));
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
                // an option with :allow-empty is the argument alone when
                // the answer is empty
                let read = if *reader == Reader::OptionalNumber {
                    self.read_line_raw(arg, None)?
                } else {
                    self.read_line_prompt(arg, None)?
                };
                let Some(v) = read else {
                    return Ok(None);
                };
                let valid = match reader {
                    Reader::Number { zero } => v.parse::<u64>().is_ok_and(|n| *zero || n > 0),
                    Reader::OptionalNumber => v.is_empty() || v.parse::<u64>().is_ok_and(|n| n > 0),
                    Reader::Similarity => {
                        let digits = v.strip_suffix('%').unwrap_or(&v);
                        !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
                    }
                    _ => true,
                };
                if !valid {
                    let what = match reader {
                        Reader::Number { zero: true } => "a non-negative integer".to_string(),
                        Reader::Number { zero: false } | Reader::OptionalNumber => "a positive integer".to_string(),
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

    /// magit-stash-read-untracked: one prefix argument is equivalent to
    /// "--include-untracked", two to "--all".
    fn untracked_from(&self, active: &BTreeMap<String, bool>) -> Untracked {
        let prefix = self.prefix_numeric();
        if prefix == 16 || *active.get("--all").unwrap_or(&false) {
            Untracked::All
        } else if prefix == 4 || *active.get("--include-untracked").unwrap_or(&false) {
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
            // magit-blame (B)
            "blame-addition" => self.blame(BlameType::Addition, args)?,
            "blame-removal" => self.blame(BlameType::Removal, args)?,
            "blame-reverse" => self.blame(BlameType::Final, args)?,
            "blame-quit" => self.blame_quit(),
            // magit-file-dispatch
            "file-stage" | "file-unstage" | "file-untrack" | "file-rename" | "file-delete" | "file-checkout"
            | "diff-buffer-file" | "log-buffer-file" | "log-trace-definition" | "blob-previous" | "blob-next"
            | "find-file" | "blob-visit-file" | "status-here" | "edit-line-commit" => self.file_command(cmd)?,
            "stage-files" => self.stage_files()?,
            "unstage-files" => self.unstage_files()?,
            "display-repository-buffer" | "display-buffer" => self.display_repository_buffer()?,
            "quit" => self.mode_bury_buffer(),
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
            "discard" if self.ref_thing().is_some() => match self.ref_thing() {
                Some(RefThing::Tag) => self.run_command("tag-delete", &BTreeMap::new())?,
                Some(RefThing::Remote) => self.run_command("remote-remove", &BTreeMap::new())?,
                _ => self.branch_delete()?,
            },
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
            "diff-more-context" | "diff-less-context" => {
                // by the numeric prefix argument
                let count = self.prefix_numeric();
                let count = if cmd == "diff-less-context" { -count } else { count };
                self.set_diff_context(|n| Some((n as i64 + count).max(0) as usize));
            }
            "diff-default-context" => self.set_diff_context(|_| None),

            // -------- commit menu: the message is written in the editor
            // (magit-run-git-with-editor); the terminal is handed over
            "commit-create" => {
                let Some(args) = self.commit_assert(&args, false, false)? else {
                    return Ok(());
                };
                let res = self.with_terminal(|g| g.commit_with_editor(&args, Editor::User));
                self.done("committed", res.map(|()| String::new()));
            }
            // magit-commit-extend (magit-commit-extend-override-date)
            "commit-extend" => {
                let Some(args) = self.commit_assert(&args, false, false)? else {
                    return Ok(());
                };
                if !self.amend_assert("HEAD")? {
                    return Ok(());
                }
                // with a prefix argument keep the committer date
                let date = self.keep_committer_date();
                let env: Vec<(&str, &str)> = date.iter().map(|d| ("GIT_COMMITTER_DATE", d.as_str())).collect();
                let res = g!().commit_extend_env(&args, &env);
                self.done("extended HEAD commit", res.map(|()| String::new()));
            }
            "commit-amend" => {
                if !self.amend_assert("HEAD")? {
                    return Ok(());
                }
                let mut a = vec!["--amend".to_string()];
                a.extend(args.iter().cloned());
                let res = self.with_terminal(|g| g.commit_with_editor(&a, Editor::User));
                self.done("amended HEAD commit", res.map(|()| String::new()));
            }
            // magit-commit-reword: the message only, not the staged changes
            "commit-reword" => {
                if !self.amend_assert("HEAD")? {
                    return Ok(());
                }
                let mut a = vec!["--amend".to_string(), "--only".to_string()];
                a.extend(args.iter().cloned());
                if !a.iter().any(|x| x == "--allow-empty") {
                    a.push("--allow-empty".into());
                }
                // with a prefix argument keep the committer date
                let date = self.keep_committer_date();
                let env: Vec<(&str, &str)> = date.iter().map(|d| ("GIT_COMMITTER_DATE", d.as_str())).collect();
                let res = self.with_terminal(|g| g.commit_with_editor_env(&a, Editor::User, &env));
                self.done("reworded HEAD commit", res.map(|()| String::new()));
            }
            // magit-commit-squash-internal: with magit-commit-squash-confirm
            // the target is always selected from the log, starting at the
            // commit at point
            "commit-fixup" | "commit-squash" | "commit-alter" | "commit-augment" | "commit-revise"
            | "commit-instant-fixup" | "commit-instant-squash" => {
                let (option, edit, nopatch, rebase) = match cmd {
                    "commit-fixup" => ("--fixup=", false, false, false),
                    "commit-squash" => ("--squash=", false, false, false),
                    "commit-alter" => ("--fixup=amend:", true, false, false),
                    "commit-augment" => ("--squash=", true, false, false),
                    "commit-revise" => ("--fixup=reword:", true, true, false),
                    "commit-instant-fixup" => ("--fixup=", false, false, true),
                    _ => ("--squash=", false, false, true),
                };
                let Some(args) = self.commit_assert(&args, nopatch, !edit)? else {
                    return Ok(());
                };
                let commit = self.commit_at_point();
                if let Some(c) = &commit {
                    if rebase && !self.git.is_ancestor(c, "HEAD") {
                        let prompt = format!(
                            "{} isn't an ancestor of HEAD.  [c]reate without rebasing, [s]elect other, [a]bort",
                            self.git.rev_short(c)
                        );
                        let answer = self.read_line_prompt(&prompt, None)?.unwrap_or_default();
                        let rebase = match answer.chars().next() {
                            Some('c') => false,
                            Some('s') => true,
                            _ => {
                                self.set_msg("Quit");
                                return Ok(());
                            }
                        };
                        let action = SelectAction::CommitSquash { option: option.into(), args, edit, rebase };
                        let initial = (!rebase).then(|| c.clone());
                        let msg = format!("Type %p on a commit to {} into it,", &option[2..]);
                        self.log_select(action, Some(&msg), initial);
                        return Ok(());
                    }
                }
                let action = SelectAction::CommitSquash { option: option.into(), args, edit, rebase };
                let msg = format!("Type %p on a commit to {} into it,", &option[2..]);
                self.log_select(action, Some(&msg), commit);
            }

            // -------- branch menu
            "checkout" | "branch-checkout" => {
                let bs = g!().branches();
                // magit-read-other-branch-or-commit: the branch at point
                let current = g!().current_branch();
                let default = self.branch_at_point().filter(|b| Some(b) != current.as_ref() && bs.contains(b));
                if let Some(b) = self.pick("Checkout branch/revision", &bs, default.as_deref())? {
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
            // magit-branch-rename
            "branch-rename" => {
                let bs = g!().branches();
                // magit-read-local-branch
                let default = self.local_branch_at_point().or_else(|| g!().current_branch());
                let Some(old) = self.pick("Rename branch", &bs, default.as_deref())?.cloned() else {
                    return Ok(());
                };
                let Some(new) = self.read_line_prompt(&format!("Rename branch '{old}' to"), None)? else {
                    return Ok(());
                };
                self.branch_rename(&old, &new)?;
            }
            "branch-reset" => {
                let bs = g!().branches();
                if let Some(b) = self.pick("Reset branch", &bs, None)? {
                    if let Some(rev) = self.read_line_prompt("Reset to", Some("HEAD"))? {
                        g!().git_interactive(&["branch", "-f", b, &rev])?;
                        if g!().current_branch().as_deref() == Some(b.as_str()) {
                            let _ = g!().git_interactive(&["reset", "--keep", &rev]);
                        }
                        // with a prefix argument the branch reset to
                        // becomes the upstream
                        let is_branch = g!().git_ok(&["show-ref", "--verify", "--quiet", &format!("refs/heads/{rev}")])
                            || g!().git_ok(&["show-ref", "--verify", "--quiet", &format!("refs/remotes/{rev}")]);
                        if self.prefix_arg.is_some() && is_branch {
                            g!().git_interactive(&["branch", &format!("--set-upstream-to={rev}"), b])?;
                        }
                        self.set_msg("branch reset");
                        self.refresh();
                    }
                }
            }
            "branch-delete" => self.branch_delete()?,

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
            // magit-remote-rename
            "remote-rename" => {
                let rs = g!().remotes();
                let default = self.remote_default();
                if let Some(old) = self.pick("Rename remote", &rs, default.as_deref())?.cloned() {
                    if let Some(new) = self.read_line_prompt(&format!("Rename {old} to"), None)? {
                        if new != old {
                            g!().remote_rename(&old, &new)?;
                            self.remote_cleanup_push_variables(&old, Some(&new));
                            self.set_msg("renamed remote");
                        }
                        self.refresh();
                    }
                }
            }
            // magit-remote-remove
            "remote-remove" => {
                let rs = g!().remotes();
                let default = self.remote_default();
                if let Some(r) = self.pick("Delete remote", &rs, default.as_deref())?.cloned() {
                    g!().remote_remove(&r)?;
                    self.remote_cleanup_push_variables(&r, None);
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
            // magit-rebase-autosquash: the commits after the merge-base
            // with the upstream, else after one selected from the log
            "rebase-autosquash" => match g!().autosquash_base() {
                Some(base) => self.rebase_autosquash(&base, &args)?,
                None => {
                    let msg = "Type %p on a commit to squash into it and then rebase as necessary,";
                    self.log_select(SelectAction::RebaseAutosquash { args: args.clone() }, Some(msg), None);
                }
            },
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
            // magit-tag-delete: the selected tags, else one read
            "tag-delete" => {
                let tags = match self.region_refs(&[RefKind::Tag], false) {
                    Some(tags) => {
                        if !self.confirm(&format!("Delete {} tags", tags.len()))? {
                            return Ok(());
                        }
                        tags
                    }
                    None => {
                        let tags = g!().tags();
                        let default = self.tag_at_point();
                        match self.pick("Delete tag", &tags, default.as_deref())? {
                            Some(t) => vec![t.clone()],
                            None => return Ok(()),
                        }
                    }
                };
                let mut a = vec!["tag", "-d"];
                a.extend(tags.iter().map(String::as_str));
                let res = g!().git_interactive(&a);
                self.done("tag deleted", res.map(|()| String::new()));
            }

            // -------- revert / cherry-pick
            // magit-revert-and-commit / magit-revert-no-commit: the selected
            // commits, else one read (magit-read-branch-or-commit)
            "revert" | "revert-no-commit" => {
                let no_commit = cmd == "revert-no-commit";
                let commits = match self.region_commits() {
                    Some(c) => c,
                    None => {
                        let prompt = if no_commit { "Revert changes" } else { "Revert commit" };
                        let default = self.commit_at_point().map(|c| self.git.rev_short(&c)).or(g!().current_branch());
                        let Some(r) = self.read_line_prompt(prompt, default.as_deref())? else {
                            return Ok(());
                        };
                        vec![r]
                    }
                };
                let mut a = args.clone();
                if no_commit {
                    a.insert(0, "--no-commit".into());
                }
                self.cherry_pick_run(commits, a, true)?;
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
            // magit-cherry-copy / magit-cherry-apply: the selected commits
            // (oldest first), else one read (magit-read-other-branch-or-commit)
            "cherry-pick" | "cherry-apply" => {
                let commits = match self.region_commits() {
                    Some(mut c) => {
                        c.reverse();
                        c
                    }
                    None => {
                        let prompt = if cmd == "cherry-pick" { "Cherry-pick" } else { "Apply changes from commit" };
                        let current = g!().current_branch();
                        let default = self
                            .commit_at_point()
                            .map(|c| self.git.rev_short(&c))
                            .filter(|c| Some(c) != current.as_ref())
                            .or_else(|| self.previous_branch());
                        let Some(r) = self.read_line_prompt(prompt, default.as_deref())? else {
                            return Ok(());
                        };
                        vec![r]
                    }
                };
                let a = if cmd == "cherry-apply" {
                    let mut a = vec!["--no-commit".to_string()];
                    a.extend(args.iter().filter(|x| *x != "--ff").cloned());
                    a
                } else {
                    args.clone()
                };
                self.cherry_pick_run(commits, a, false)?;
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
            // magit-show-refs: the menu in the refs buffer or with a
            // prefix argument, else the refs compared with HEAD
            "show-refs" => {
                if matches!(self.mode, BufferMode::Refs { .. }) || self.prefix_arg.is_some() {
                    self.open_menu("show-refs-menu");
                } else {
                    let args = self.prefix_args("show-refs-menu");
                    self.setup_buffer(BufferMode::Refs { upstream: "HEAD".into() }, args);
                }
            }
            "show-refs-head" => self.setup_buffer(BufferMode::Refs { upstream: "HEAD".into() }, args),
            "show-refs-current" => {
                let upstream = g!().current_branch().unwrap_or_else(|| "HEAD".into());
                self.setup_buffer(BufferMode::Refs { upstream }, args);
            }
            // magit-read-other-branch
            "show-refs-other" => {
                let current = g!().current_branch();
                let mut choices = g!().branches();
                choices.extend(g!().remote_branches());
                choices.retain(|b| Some(b) != current.as_ref());
                let atpoint = self.branch_at_point().filter(|b| Some(b) != current.as_ref());
                let previous = self.previous_branch();
                let default = atpoint.or(previous);
                if let Some(r) = self.pick("Compare with", &choices, default.as_deref())?.cloned() {
                    self.setup_buffer(BufferMode::Refs { upstream: r }, args);
                }
            }
            // magit-refs-set-show-commit-count
            "refs-set-show-commit-count" => {
                if !matches!(self.mode, BufferMode::Refs { .. }) {
                    self.set_msg("Not in a refs buffer");
                    return Ok(());
                }
                let prompt = "Show commit counts for [a]ll refs, [b]ranches only, [n]othing";
                let answer = self.read_line_prompt(prompt, None)?.unwrap_or_default();
                self.refs_counts = match answer.chars().next() {
                    Some('a') => CommitCount::All,
                    Some('b') => CommitCount::Branches,
                    Some('n') => CommitCount::Nothing,
                    _ => {
                        self.set_msg("Quit");
                        return Ok(());
                    }
                };
                self.refresh();
            }
            // magit-delete-thing / magit-file-rename remapped in the
            // sections of the refs buffer
            "rename-thing" => match self.ref_thing() {
                Some(RefThing::Branch) => self.run_command("branch-rename", &BTreeMap::new())?,
                Some(RefThing::Remote) => self.run_command("remote-rename", &BTreeMap::new())?,
                _ => self.file_command("file-rename")?,
            },

            // -------- log / diff / show (arguments come from the l/d menus)
            "log-current" | "log-head" | "log-other" | "log-related" | "log-branches" | "log-all-branches"
            | "log-all" | "log-change" => self.log_view(cmd)?,
            // magit-log-refresh (L g): new arguments for the log buffer shown
            "log-refresh" => match self.mode {
                BufferMode::Log { .. } | BufferMode::LogSelect { .. } => {
                    self.buf_args = args.clone();
                    self.refresh();
                }
                // the log sections of the status buffer
                BufferMode::Status => {
                    self.status_log_args = args.clone();
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
            // with a prefix argument against a commit read
            "staged" | "worktree" if self.prefix_arg.is_some() => {
                let prompt = if kind == "staged" { "Diff index and commit" } else { "Diff working tree and commit" };
                let default = self.branch_or_commit_default();
                let Some(rev) = self.read_line_prompt(prompt, default.as_deref())? else {
                    return Ok(());
                };
                if kind == "staged" {
                    self.show_diff(Some(rev), Some("--cached"), "staged");
                } else {
                    self.show_diff(Some(rev), None, "committed");
                }
            }
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
                let atpoint = match self.current_line().map(|l| l.kind.clone()) {
                    Some(LineKind::Commit { hash, .. }) => Some(hash),
                    _ => match &self.mode {
                        BufferMode::Revision { rev, .. } => Some(rev.clone()),
                        _ => None,
                    },
                };
                // with a prefix argument always read the commit
                let rev = match atpoint {
                    Some(r) if self.prefix_arg.is_none() => Some(r),
                    Some(r) => {
                        let short = self.git.rev_short(&r);
                        self.read_line_prompt("Show commit", Some(&short))?
                    }
                    None => {
                        let current = self.git.current_branch();
                        self.read_line_prompt("Show commit", current.as_deref())?
                    }
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
                // with a prefix argument always read the stash
                let stash = match at {
                    Some(s) if self.prefix_arg.is_none() => Some(s),
                    at => {
                        let names: Vec<String> = self.git.stashes().into_iter().map(|(n, _)| n).collect();
                        self.pick("Show stash", &names, at.as_deref())?.cloned()
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
                    // magit-region-values '(commit branch)
                    LineKind::Ref { kind: RefKind::Local | RefKind::Remote, name } => Some(name.clone()),
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
            BufferMode::Log { .. } | BufferMode::LogSelect { .. } | BufferMode::Reflog { .. } => {
                match line.map(|l| &l.kind) {
                    Some(LineKind::Commit { hash, .. }) => Dwim::Commit(hash.clone()),
                    Some(
                        LineKind::File { section, .. }
                        | LineKind::Hunk { section, .. }
                        | LineKind::DiffLine { section, .. }
                        | LineKind::Meta { section, .. },
                    ) => Dwim::Commit(section.clone()),
                    _ => Dwim::None,
                }
            }
            BufferMode::Stashes { .. } | BufferMode::Cherry { .. } | BufferMode::Refs { .. } => match line.map(|l| &l.kind) {
                Some(LineKind::Commit { hash, .. }) => Dwim::Commit(hash.clone()),
                Some(LineKind::Stash { name, .. }) => Dwim::Stash(name.clone()),
                _ => Dwim::None,
            },
            BufferMode::File { .. } => Dwim::None,
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
            (None, m) if m.is_log() => self.buf_args.clone(),
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

    /// magit-commit-at-point: the commit section at point, else the
    /// revision of a revision or stash buffer.
    fn commit_at_point(&self) -> Option<String> {
        if let Some(LineKind::Commit { hash, .. }) = self.current_line().map(|l| &l.kind) {
            return Some(hash.clone());
        }
        match &self.mode {
            BufferMode::Revision { rev, .. } => Some(rev.clone()),
            BufferMode::Stash { stash } => Some(stash.clone()),
            _ => None,
        }
    }

    /// magit-region-values 'commit: the commits of the selected sibling
    /// commit sections (at least two), top to bottom.
    fn region_commits(&self) -> Option<Vec<String>> {
        let secs = self.region_sections(true)?;
        secs.iter()
            .map(|&s| match &self.buf.all[self.sec(s).start].kind {
                LineKind::Commit { hash, .. } => Some(hash.clone()),
                _ => None,
            })
            .collect()
    }

    /// The previously checked out branch (`@{-1}`), if any.
    fn previous_branch(&self) -> Option<String> {
        self.git
            .git_string(&["rev-parse", "--abbrev-ref", "@{-1}"])
            .ok()
            .filter(|s| !s.is_empty() && s != "HEAD")
    }

    /// magit--cherry-pick: revert or cherry-pick COMMITS (a single "A..B"
    /// is split), handling --mainline for merge commits.
    fn cherry_pick_run(&mut self, commits: Vec<String>, args: Vec<String>, revert: bool) -> Result<()> {
        let command = if revert { "revert" } else { "cherry-pick" };
        let commits: Vec<String> = match commits.as_slice() {
            [one] if one.contains("..") => one.split("..").map(String::from).collect(),
            _ => commits,
        };
        let merges = commits.iter().filter(|c| self.git.is_merge_commit(c)).count();
        let args = if merges == 0 {
            args.into_iter().filter(|a| !a.starts_with("--mainline=")).collect()
        } else if merges != commits.len() {
            self.set_msg(&format!("Cannot {command} merge and non-merge commits at once"));
            return Ok(());
        } else if args.iter().any(|a| a.starts_with("--mainline=")) {
            args
        } else {
            let n = loop {
                let Some(s) = self.read_line_prompt("Replay merges relative to parent", None)? else {
                    return Ok(());
                };
                if let Ok(n) = s.trim().parse::<u32>() {
                    break n;
                }
            };
            let mut a = vec![format!("--mainline={n}")];
            a.extend(args);
            a
        };
        let res = self.with_terminal(|g| g.sequencer_run(command, &args, &commits));
        match res {
            Ok(()) => self.set_msg(command),
            Err(e) => self.set_msg(&format!("{e}")),
        }
        self.refresh();
        Ok(())
    }

    /// magit-log-select: show the log of the current branch to pick a
    /// commit for ACTION, at INITIAL, else at the first commit that is not
    /// a squash!, fixup! or amend! commit. MSG is the usage, "%p" and "%q"
    /// standing for the keys that pick and quit.
    fn log_select(&mut self, action: SelectAction, msg: Option<&str>, initial: Option<String>) {
        let msg = match msg {
            Some(m) if m.ends_with(',') => format!("{m} or %q to abort"),
            Some(m) => m.to_string(),
            None => "Type %p to select commit at point, or %q to abort".to_string(),
        };
        let msg = msg.replace("%p", "C-c C-c").replace("%q", "C-c C-k");
        let rev = self.git.current_branch().unwrap_or_else(|| "HEAD".into());
        let args = self.log_select_args.clone();
        self.setup_buffer(BufferMode::LogSelect { rev, action, msg: msg.clone() }, args);
        match initial {
            Some(c) => {
                self.log_goto_commit(&c);
            }
            None => {
                let commits: Vec<String> = self
                    .lines
                    .iter()
                    .filter(|l| l.heading && l.depth == 0)
                    .filter_map(|l| match &l.kind {
                        LineKind::Commit { hash, .. } => Some(hash.clone()),
                        _ => None,
                    })
                    .collect();
                let target = commits.iter().find(|h| {
                    let s = self.git.git_string(&["log", "-1", "--format=%s", h]).unwrap_or_default();
                    !["squash!", "fixup!", "amend!"].iter().any(|p| s.starts_with(p))
                });
                if let Some(h) = target.cloned() {
                    self.log_goto_commit(&h);
                }
            }
        }
        self.set_msg(&msg);
    }

    /// magit-log-select-pick: bury the log-select buffer and act on the
    /// commit at point.
    fn log_select_pick(&mut self) -> Result<()> {
        let BufferMode::LogSelect { action, .. } = self.mode.clone() else { return Ok(()) };
        let Some(LineKind::Commit { hash, .. }) = self.current_line().map(|l| l.kind.clone()) else {
            return Ok(());
        };
        let commit = self.git.rev_short(&hash);
        self.kill_buffer();
        match action {
            SelectAction::Rebase { cmd, args } => self.rebase_commit(&cmd, &commit, &args),
            SelectAction::RebaseSubset { newbase, args } => {
                let res = self.git.rebase_subset(&newbase, &format!("{commit}^"), &args);
                self.after_rebase(res);
                Ok(())
            }
            SelectAction::RebaseAutosquash { args } => self.rebase_autosquash(&commit, &args),
            SelectAction::CommitSquash { option, args, edit, rebase } => {
                self.commit_squash(&option, &commit, &args, edit, rebase)
            }
        }
    }

    /// magit-rebase-interactive-1 for the interactive rebase CMD of COMMIT
    /// and the commits above it (magit-rebase-remove-commit passes NOASSERT).
    fn rebase_commit(&mut self, cmd: &str, commit: &str, args: &[String]) -> Result<()> {
        if cmd != "rebase-remove-commit" && !self.rebase_assert(commit, cmd != "rebase-reword-commit", args)? {
            return Ok(());
        }
        let res = match cmd {
            "rebase-interactive" => {
                self.with_terminal(|g| g.rebase_interactive(commit, args, Editor::User, Editor::User))
            }
            "rebase-edit-commit" => self.git.rebase_edit_commit(commit, args),
            "rebase-reword-commit" => self.with_terminal(|g| g.rebase_reword_commit(commit, args, Editor::User)),
            _ => self.git.rebase_remove_commit(commit, args),
        };
        self.after_rebase(res);
        Ok(())
    }

    /// magit-rebase-autosquash after BASE (exact, no assertions).
    fn rebase_autosquash(&mut self, base: &str, args: &[String]) -> Result<()> {
        if !self.git.is_ancestor(base, "HEAD") {
            self.set_msg(&format!("{base} isn't an ancestor of HEAD"));
            return Ok(());
        }
        let res = self.git.rebase_autosquash_with(base, args);
        self.after_rebase(res);
        Ok(())
    }

    /// magit-commit-assert: the arguments to commit with, None if there is
    /// nothing to commit (or the in-progress rebase was continued instead).
    /// NOPATCH: no changes are needed; not STRICT: amending arguments make
    /// a commit without changes possible.
    fn commit_assert(&mut self, args: &[String], nopatch: bool, strict: bool) -> Result<Option<Vec<String>>> {
        let unstaged = !self.git.unstaged_files().is_empty();
        let amending = ["--amend", "--allow-empty", "--reset-author", "--signoff"]
            .iter()
            .any(|a| args.iter().any(|x| x == a))
            || args.iter().any(|x| x.starts_with("--author=") || x.starts_with("--date="));
        if nopatch
            || self.git.anything_staged()
            || (unstaged && args.iter().any(|a| a == "--all"))
            || (!strict && amending)
        {
            return Ok(Some(args.to_vec()));
        }
        if self.git.rebase_in_progress()
            && !unstaged
            && self.yes_no("Nothing staged.  Continue in-progress rebase?")?
        {
            let res = self.git.rebase_continue();
            self.after_rebase(res);
            return Ok(None);
        }
        if self.git.gitdir.join("MERGE_MSG").exists() {
            if !self.git.unmerged_files().is_empty() {
                self.set_msg("Unresolved conflicts");
                return Ok(None);
            }
            if unstaged && !self.yes_no("Proceed with merge despite unstaged changes?")? {
                self.set_msg("Abort");
                return Ok(None);
            }
            return Ok(Some(args.to_vec()));
        }
        if !unstaged {
            self.set_msg("Nothing staged (or unstaged)");
            return Ok(None);
        }
        // magit-commit-ask-to-stage
        if self.yes_no("Nothing staged.  Commit all uncommitted changes?")? {
            let mut a = vec!["--all".to_string()];
            a.extend(args.iter().cloned());
            return Ok(Some(a));
        }
        Ok(None)
    }

    /// magit-commit-squash-internal once COMMIT is confirmed: create the
    /// squash commit (OPTION followed by COMMIT), then with REBASE squash
    /// it into COMMIT right away.
    fn commit_squash(&mut self, option: &str, commit: &str, args: &[String], edit: bool, rebase: bool) -> Result<()> {
        if self.git.is_ancestor(commit, "HEAD") && !self.rebase_assert(commit, true, args)? {
            return Ok(());
        }
        let mut a = vec![if edit { "--edit" } else { "--no-edit" }.to_string(), format!("{option}{commit}")];
        if rebase {
            a.push("--no-gpg-sign".into());
            a.extend(args.iter().filter(|x| !x.starts_with("--gpg-sign=")).cloned());
        } else {
            a.extend(args.iter().cloned());
        }
        let res = if edit {
            self.with_terminal(|g| g.commit_with_editor(&a, Editor::User))
        } else {
            self.git.commit(None, &a)
        };
        if let Err(e) = res {
            self.set_msg(&format!("{e}"));
            self.refresh();
            return Ok(());
        }
        if !rebase {
            self.refresh();
            return Ok(());
        }
        // magit-commit-amend-assert, then magit-rebase-interactive-1 with
        // the autosquash arguments, no editor and no assertions
        if !self.amend_assert(commit)? {
            self.refresh();
            return Ok(());
        }
        let ra = ["--autosquash", "--autostash", "--keep-empty"].map(String::from);
        let res = self.git.rebase_interactive(commit, &ra, Editor::Accept, Editor::Accept);
        self.after_rebase(res);
        Ok(())
    }

    /// magit-commit-amend-assert: confirm modifying a published COMMIT.
    fn amend_assert(&mut self, commit: &str) -> Result<bool> {
        let published: Vec<&str> = PUBLISHED_BRANCHES
            .iter()
            .copied()
            .filter(|b| self.git.rev_verify(b) && self.git.is_ancestor(commit, b))
            .collect();
        if published.is_empty() {
            return Ok(true);
        }
        let ok = self.yes_no(&format!(
            "This commit has already been published to {}.\nDo you really want to modify it?",
            published.join(", ")
        ))?;
        if !ok {
            self.set_msg("Abort");
        }
        Ok(ok)
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

    /// C-c C-n in log buffers (magit-log-move-to-parent): to the Nth
    /// parent, N being the numeric prefix argument.
    fn log_move_to_parent(&mut self) {
        let Some(LineKind::Commit { hash, .. }) = self.current_line().map(|l| l.kind.clone()) else { return };
        let parent_rev = format!("{hash}^{}", self.prefix_numeric());
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
                // magit-stage-untracked: with a prefix argument stage
                // the files but not their content
                let mut a = vec!["add"];
                if self.prefix_arg.is_some() {
                    a.push("--intent-to-add");
                }
                a.push("--");
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
    /// file at point. With a prefix argument offer the ignored files and
    /// force adding them.
    fn stage_files(&mut self) -> Result<()> {
        let force = self.prefix_arg.is_some();
        let (prompt, choices) = if force {
            let ignored = self.git.git_lines(&["ls-files", "-z", "--others", "--ignored", "--exclude-standard"]);
            ("Stage ignored file,s", ignored)
        } else {
            let mut choices = self.git.unstaged_files();
            choices.extend(self.git.untracked_files());
            ("Stage file,s", choices)
        };
        let Some(files) = self.pick_files(prompt, &choices)? else {
            return Ok(());
        };
        let mut a = vec!["add"];
        if force {
            a.push("--force");
        }
        a.push("--");
        a.extend(files.iter().map(String::as_str));
        self.git.git_interactive(&a)?;
        self.set_msg("staged");
        self.refresh();
        Ok(())
    }

    /// magit-completing-read-multiple of files from CHOICES (separated by
    /// commas), the default being the file at point or the visited file.
    fn pick_files(&mut self, prompt: &str, choices: &[String]) -> Result<Option<Vec<String>>> {
        if choices.is_empty() {
            self.set_msg("no candidates");
            return Ok(None);
        }
        let default = self
            .current_file()
            .map(|f| f.1)
            .or_else(|| self.visited_file().map(|f| f.1))
            .filter(|f| choices.contains(f));
        let Some(inp) = self.read_line_prompt(prompt, default.as_deref())? else {
            return Ok(None);
        };
        let mut files = Vec::new();
        for f in inp.split(',').map(str::trim).filter(|f| !f.is_empty()) {
            if !choices.iter().any(|c| c == f) {
                self.set_msg("no match");
                return Ok(None);
            }
            files.push(f.to_string());
        }
        Ok(Some(files))
    }

    /// magit-stage-modified (S): `git add -u .`, with a prefix argument
    /// `git add --all .` (also new files), confirming when something is
    /// staged already.
    fn stage_modified(&mut self) -> Result<()> {
        if self.git.anything_staged() && !self.confirm("Stage all changes")? {
            return Ok(());
        }
        if self.prefix_arg.is_some() {
            self.git.stage_all_with_untracked()?;
            self.set_msg("staged all changes");
        } else {
            self.git.stage_all()?;
            self.set_msg("staged all modified");
        }
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
        let Some(files) = self.pick_files("Unstage file,s", &choices)? else {
            return Ok(());
        };
        for f in &files {
            self.git.unstage(f)?;
        }
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
            // with a prefix argument fall back to a 3-way merge
            _ if self.prefix_arg.is_some() => self.reverse_sel(&sel, &["--3way"]),
            _ => self.reverse_sel(&sel, &[]),
        }
    }

    /// magit-reverse-region / -hunk / -hunks / -files: apply the selection
    /// with `--reverse ARGS --reject` (magit-reverse-apply; not with
    /// `--3way`).
    fn reverse_sel(&mut self, sel: &DiffSel, args: &[&str]) -> Result<()> {
        let group = sel.typ().to_string();
        let ctx = self.context();
        let mut a = vec!["--reverse"];
        a.extend(args);
        if !args.contains(&"--3way") {
            a.push("--reject");
        }
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
        // with a prefix argument fall back to a 3-way merge
        let mut three: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a == "--3way").collect();
        if self.prefix_arg.is_some() && three.is_empty() {
            three.push("--3way");
        }
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
            (BufferMode::Log { .. } | BufferMode::LogSelect { .. } | BufferMode::Reflog { .. }, _) => {
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
        let rev = if worktree || rev == "{worktree}" {
            if worktree && rev != "{worktree}" {
                line = line.map(|l| self.visit_offset(l, &file, &rev));
            }
            "{worktree}".to_string()
        } else {
            rev
        };
        if let Err(e) = super::blame::file_contents(&self.git, &rev, &file) {
            self.set_msg(&format!("{e}"));
            return;
        }
        self.find_file(&rev, &file, line.unwrap_or(1));
    }

    /// magit-find-file: the buffer visiting FILE at REV, at LINE.
    fn find_file(&mut self, rev: &str, file: &str, line: usize) {
        self.setup_buffer(BufferMode::File { rev: rev.into(), file: file.into(), blame: None }, Vec::new());
        self.goto_file_line(line);
    }

    /// The line of the file at point (1-based), and its blame chunk.
    pub fn file_line(&self) -> Option<usize> {
        self.file_line_at_point().map(|(l, _)| l)
    }

    fn file_line_at_point(&self) -> Option<(usize, Option<usize>)> {
        let lines = &self.lines;
        let i = self.cursor.min(lines.len().saturating_sub(1));
        // a blame heading belongs to the line after it
        lines[i..].iter().chain(lines[..i].iter().rev()).find_map(|l| match l.kind {
            LineKind::FileLine { line, chunk } => Some((line, chunk)),
            _ => None,
        })
    }

    fn goto_file_line(&mut self, line: usize) {
        if let Some(i) = self.lines.iter().position(|l| matches!(l.kind, LineKind::FileLine { line: n, .. } if n >= line)) {
            self.cursor = i;
        } else {
            self.cursor = self.lines.len().saturating_sub(1);
        }
    }

    /// magit-current-blame-chunk
    fn current_blame_chunk(&self) -> Option<super::blame::Chunk> {
        let (_, chunk) = self.file_line_at_point()?;
        chunk.and_then(|c| self.buf.blame.get(c).cloned())
    }

    /// magit-blame-arguments outside the blame transient: its value
    fn prefix_args(&self, name: &str) -> Vec<String> {
        match self.set_values.get(name).or(self.saved_values.get(name)) {
            Some(v) => v.clone(),
            None => prefix_value(name).iter().map(|s| s.to_string()).collect(),
        }
    }

    /// magit-blame-addition / -removal / -reverse with ARGS.
    fn blame(&mut self, typ: BlameType, args: Vec<String>) -> Result<()> {
        let BufferMode::File { rev, file, blame } = self.mode.clone() else {
            self.set_msg("Buffer isn't visiting a file");
            return Ok(());
        };
        if typ != BlameType::Addition && (rev == "{worktree}" || rev == "{index}") {
            self.set_msg("Only blob buffers can be blamed in reverse");
            return Ok(());
        }
        let tracked = self.git.git_ok(&["ls-files", "--error-unmatch", "--", &file]);
        if rev == "{worktree}" && !tracked {
            self.set_msg("Buffer isn't visiting a tracked file");
            return Ok(());
        }
        // magit-blame--pre-blame-assert / -setup
        let (line, _) = self.file_line_at_point().unwrap_or((1, None));
        match blame {
            Some(b) if b.typ == typ => {
                let Some(chunk) = self.current_blame_chunk() else {
                    self.set_msg("Still blaming, commit data not available yet");
                    return Ok(());
                };
                let (Some(prev_rev), Some(prev_file)) = (chunk.prev_rev.clone(), chunk.prev_file.clone()) else {
                    self.set_msg("Chunk has no further history");
                    return Ok(());
                };
                // magit-blame-visit-other-file, keeping the style
                let blame = Blame { typ, style: b.style, recursive: true, args };
                self.setup_buffer(BufferMode::File { rev: prev_rev, file: prev_file, blame: Some(blame) }, Vec::new());
                self.goto_file_line(chunk.orig_line);
            }
            b => {
                let style = b.map_or(0, |b| b.style);
                let recursive = matches!(&self.mode, BufferMode::File { blame: Some(b), .. } if b.recursive);
                let blame = Blame { typ, style, recursive, args };
                self.mode = BufferMode::File { rev, file, blame: Some(blame) };
                self.refresh();
                self.goto_file_line(line);
            }
        }
        if let Some(e) = self.buf.blame_error.clone() {
            self.set_msg(&e);
        } else {
            self.blame_show_message();
        }
        Ok(())
    }

    /// magit-blame-quit: turn off blaming; a buffer created by a recursive
    /// blame is killed.
    fn blame_quit(&mut self) {
        let BufferMode::File { rev, file, blame: Some(b) } = self.mode.clone() else {
            self.set_msg("Not blaming");
            return;
        };
        if b.recursive {
            self.kill_buffer();
            return;
        }
        let (line, _) = self.file_line_at_point().unwrap_or((1, None));
        self.mode = BufferMode::File { rev, file, blame: None };
        self.refresh();
        self.goto_file_line(line);
    }

    /// magit-blame-cycle-style
    fn blame_cycle_style(&mut self) {
        let BufferMode::File { blame: Some(b), .. } = &mut self.mode else {
            self.set_msg("Not blaming");
            return;
        };
        b.style = (b.style + 1) % STYLES.len();
        let (line, _) = self.file_line_at_point().unwrap_or((1, None));
        self.refresh();
        self.goto_file_line(line);
    }

    /// magit-blame-maybe-show-message: the summary of the chunk at point
    /// in the lines style.
    fn blame_show_message(&mut self) {
        let lines_style = matches!(&self.mode, BufferMode::File { blame: Some(b), .. } if STYLES[b.style] == "lines");
        if let Some(c) = self.current_blame_chunk().filter(|_| lines_style) {
            let msg = c.info.get("summary").cloned().unwrap_or_default();
            self.set_msg(&format!("{msg:?}"));
        }
    }

    /// magit-blame-next-chunk / -previous-chunk; with SAME, only chunks of
    /// the commit at point (magit-blame-next-chunk-same-commit).
    fn blame_goto_chunk(&mut self, forward: bool, same: bool) {
        let Some((_, Some(cur))) = self.file_line_at_point() else {
            self.set_msg(if same { "No more chunks from same commit" } else { "No more chunks" });
            return;
        };
        let rev = self.buf.blame[cur].orig_rev.clone();
        let wanted = |c: usize, chunks: &[super::blame::Chunk]| !same || chunks[c].orig_rev == rev;
        let chunks = &self.buf.blame;
        let target = if forward {
            (cur + 1..chunks.len()).find(|&c| wanted(c, chunks))
        } else {
            (0..cur).rev().find(|&c| wanted(c, chunks))
        };
        let Some(t) = target else {
            self.set_msg(if same { "No more chunks from same commit" } else { "No more chunks" });
            return;
        };
        // the start of the chunk: its heading, if any
        let start = self.lines.iter().position(|l| {
            matches!(l.kind, LineKind::BlameHeading { chunk } if chunk == t)
                || matches!(l.kind, LineKind::FileLine { chunk: Some(c), .. } if c == t)
        });
        if let Some(i) = start {
            self.cursor = i;
        }
        self.blame_show_message();
    }

    /// magit-blob-previous (OLDER) / magit-blob-next
    fn blob_visit(&mut self, older: bool) {
        let BufferMode::File { rev, file, .. } = self.mode.clone() else {
            self.set_msg("Buffer isn't visiting a file or blob");
            return;
        };
        let target = if older { blob_ancestor(&self.git, &rev, &file) } else { blob_successor(&self.git, &rev, &file) };
        let Some((r, f)) = target else {
            self.set_msg(if older { "You have reached the beginning of time" } else { "You have reached the end of time" });
            return;
        };
        let (line, _) = self.file_line_at_point().unwrap_or((1, None));
        self.find_file(&r, &f, line);
        // magit-blob-visit
        if r != "{worktree}" && r != "{index}" {
            let s = self.git.git_string(&["log", "-1", "--format=%s\x1f%ct", &r]).unwrap_or_default();
            let (subject, time) = s.trim_end().split_once('\x1f').unwrap_or((s.trim_end(), "0"));
            let (n, unit) = super::logbuf::age(super::logbuf::now() - time.parse::<f64>().unwrap_or(0.0));
            self.set_msg(&format!("{subject} ({n} {unit} ago)"));
        }
    }

    /// (revision, file) of the file or blob buffer shown
    /// (magit-buffer-revision, magit-file-relative-name).
    fn visited_file(&self) -> Option<(String, String)> {
        match &self.mode {
            BufferMode::File { rev, file, .. } => Some((rev.clone(), file.clone())),
            _ => None,
        }
    }

    /// magit-current-file: the visited file, the file at point, or the
    /// file the log buffer is limited to.
    fn current_file_name(&self) -> Option<String> {
        self.visited_file().map(|f| f.1).or_else(|| self.current_file().map(|f| f.1)).or_else(|| match &self.mode {
            BufferMode::Log { files, .. } => files.first().cloned(),
            _ => None,
        })
    }

    fn is_tracked(&self, file: &str) -> bool {
        self.git.git_ok(&["ls-files", "--error-unmatch", "--", file])
    }

    /// magit-read-file (TRACKED: magit-read-tracked-file) with
    /// require-match, the default being the current file.
    fn read_file_name(&mut self, prompt: &str, tracked: bool) -> Result<Option<String>> {
        let mut choices = self.git.git_lines(&["ls-files", "--full-name", "-z"]);
        if !tracked {
            choices.extend(self.git.untracked_files());
        }
        let default = self.current_file_name().filter(|f| choices.contains(f));
        self.read_choice(prompt, &choices, default.as_deref())
    }

    /// magit-read-file-from-rev
    fn read_file_from_rev(&mut self, rev: &str, prompt: &str) -> Result<Option<String>> {
        let choices = self.git.git_lines(&["ls-tree", "-r", "--full-tree", "--name-only", "-z", rev]);
        let default = self.current_file_name().filter(|f| choices.contains(f));
        self.read_choice(prompt, &choices, default.as_deref())
    }

    /// magit-completing-read with require-match.
    fn read_choice(&mut self, prompt: &str, choices: &[String], default: Option<&str>) -> Result<Option<String>> {
        let Some(v) = self.read_line_prompt(prompt, default)? else {
            return Ok(None);
        };
        if choices.contains(&v) {
            Ok(Some(v))
        } else {
            self.set_msg("[No match]");
            Ok(None)
        }
    }

    /// magit-branch-or-commit-at-point, else the current branch.
    fn branch_or_commit_default(&self) -> Option<String> {
        match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Commit { hash, .. }) => Some(self.git.rev_short(&hash)),
            Some(LineKind::Ref { name, .. }) => Some(name),
            _ => self.git.current_branch(),
        }
    }

    /// The files of the selected file sections (magit-region-values 'file).
    fn region_files(&self) -> Option<Vec<String>> {
        let secs = self.region_sections(true)?;
        secs.iter()
            .map(|&s| match &self.buf.all[self.sec(s).start].kind {
                LineKind::File { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    /// The file commands of magit-file-dispatch (magit-files.el).
    fn file_command(&mut self, cmd: &str) -> Result<()> {
        match cmd {
            // magit-file-stage / magit-file-unstage
            "file-stage" | "file-unstage" => {
                let Some((_, file)) = self.visited_file().filter(|(r, _)| r == "{worktree}") else {
                    self.set_msg("Not visiting a file");
                    return Ok(());
                };
                if cmd == "file-unstage" {
                    self.git.unstage(&file)?;
                    self.set_msg("unstaged");
                } else {
                    let mut a = vec!["add"];
                    if self.git.git_ok(&["check-ignore", "-q", "--", &file]) {
                        if !self.yes_no("Visited file is ignored; stage anyway?")? {
                            self.set_msg("Abort");
                            return Ok(());
                        }
                        a.push("--force");
                    }
                    a.extend(["--", &file]);
                    self.git.git_interactive(&a)?;
                    self.set_msg("staged");
                }
                self.refresh();
            }
            // magit-file-untrack
            "file-untrack" => {
                let files = match self.region_files() {
                    Some(files) => {
                        if !self.is_tracked(&files[0]) {
                            self.set_msg("Already untracked");
                            return Ok(());
                        }
                        if !self.confirm_files("Untrack", &files)? {
                            return Ok(());
                        }
                        files
                    }
                    None => match self.read_file_name("Untrack file", true)? {
                        Some(f) => vec![f],
                        None => return Ok(()),
                    },
                };
                // with a prefix argument even files with staged and
                // unstaged changes
                let mut a = vec!["rm", "--cached"];
                if self.prefix_arg.is_some() {
                    a.push("--force");
                }
                a.push("--");
                a.extend(files.iter().map(String::as_str));
                let res = self.git.git_interactive(&a);
                self.done("untracked", res.map(|()| String::new()));
            }
            // magit-file-rename
            "file-rename" => {
                let Some(file) = self.read_file_name("Rename file", false)? else {
                    return Ok(());
                };
                let dir = match file.rfind('/') {
                    Some(i) => file[..=i].to_string(),
                    None => String::new(),
                };
                let prompt = format!("Move {file} to destination");
                let Some(newname) = self.read_line_prompt(&prompt, Some(&dir).filter(|d| !d.is_empty()).map(|d| d.as_str()))?
                else {
                    return Ok(());
                };
                let top = self.git.toplevel.clone();
                let base = file.rsplit('/').next().unwrap_or(&file).to_string();
                let dst = if newname.ends_with('/') || top.join(&newname).is_dir() {
                    format!("{}/{base}", newname.trim_end_matches('/'))
                } else {
                    newname
                };
                if top.join(&dst).exists() {
                    self.set_msg(&format!("{dst} already exists"));
                    return Ok(());
                }
                let dstdir = top.join(&dst).parent().map(std::path::Path::to_path_buf).unwrap_or(top.clone());
                if !dstdir.is_dir() {
                    self.set_msg(&format!("Destination directory {} does not exist", dstdir.display()));
                    return Ok(());
                }
                if self.is_tracked(&file) {
                    self.git.git_interactive(&["mv", &file, &dst])?;
                } else {
                    std::fs::rename(top.join(&file), top.join(&dst))?;
                }
                // the buffer visiting the file now visits the new name
                if let BufferMode::File { rev, file: f, .. } = &mut self.mode {
                    if rev == "{worktree}" && *f == file {
                        *f = dst.clone();
                    }
                }
                self.set_msg(&format!("renamed {file} to {dst}"));
                self.refresh();
            }
            // magit-file-delete
            "file-delete" => {
                let files = match self.region_files() {
                    Some(files) => {
                        if !self.confirm_files("Delete", &files)? {
                            return Ok(());
                        }
                        files
                    }
                    None => match self.read_file_name("Delete file", false)? {
                        Some(f) => vec![f],
                        None => return Ok(()),
                    },
                };
                let top = self.git.toplevel.clone();
                let dirs: Vec<&String> = files.iter().filter(|f| top.join(f).is_dir()).collect();
                // with a prefix argument even files with uncommitted changes
                let mut args = vec!["rm"];
                if self.prefix_arg.is_some() {
                    args.push("--force");
                }
                if !dirs.is_empty() {
                    let what = if dirs.len() == 1 { "directory" } else { "directories" };
                    let names = dirs.iter().map(|d| d.as_str()).collect::<Vec<_>>().join(",");
                    if !self.yes_no(&format!("Recursively delete {what} \"{names}\":"))? {
                        self.set_msg("Abort");
                        return Ok(());
                    }
                    args.push("-r");
                }
                if self.is_tracked(&files[0]) {
                    args.push("--");
                    args.extend(files.iter().map(String::as_str));
                    let res = self.git.git_interactive(&args);
                    self.done("deleted", res.map(|()| String::new()));
                } else {
                    for f in &files {
                        let p = top.join(f);
                        if p.is_dir() {
                            std::fs::remove_dir_all(p)?;
                        } else {
                            std::fs::remove_file(p)?;
                        }
                    }
                    self.set_msg("deleted");
                    self.refresh();
                }
            }
            // magit-file-checkout
            "file-checkout" => {
                let default = match self.visited_file() {
                    Some((r, _)) if r != "{worktree}" && r != "{index}" => Some(r),
                    _ => self.branch_or_commit_default(),
                };
                let Some(rev) = self.read_line_prompt("Checkout from revision", default.as_deref())? else {
                    return Ok(());
                };
                let Some(file) = self.read_file_from_rev(&rev, "Checkout file")? else {
                    return Ok(());
                };
                let res = self.git.git_interactive(&["checkout", &rev, "--", &file]);
                self.done("checked out", res.map(|()| String::new()));
            }
            // magit-diff-buffer-file: the commit of a blob, else the
            // changes to the file since the current branch
            "diff-buffer-file" => {
                let Some((rev, file)) = self.visited_file() else {
                    self.set_msg("Buffer isn't visiting a file");
                    return Ok(());
                };
                if rev != "{worktree}" && rev != "{index}" {
                    let args = match &self.transient_diff_args {
                        Some(a) => a.clone(),
                        None => self.diff_args.clone(),
                    };
                    self.setup_buffer(BufferMode::Revision { rev, files: vec![file] }, args);
                } else {
                    let line = self.file_line().unwrap_or(1);
                    let range = self.git.current_branch().unwrap_or_else(|| "HEAD".into());
                    let spec = DiffSpec { range: Some(range), typearg: None, typ: "unstaged".into(), files: vec![file.clone()] };
                    let args = self.transient_diff_args.clone().unwrap_or_else(|| self.diff_args.clone());
                    self.setup_buffer(BufferMode::Diff(spec), args);
                    self.goto_file_position(&file, line, &[]);
                }
            }
            // magit-log-buffer-file: with a region, the log of its lines
            "log-buffer-file" | "log-trace-definition" => {
                let Some((rev, file)) = self.visited_file() else {
                    self.set_msg("Buffer isn't visiting a file");
                    return Ok(());
                };
                let rev = if rev == "{worktree}" || rev == "{index}" {
                    self.git.current_branch().unwrap_or_else(|| "HEAD".into())
                } else {
                    rev
                };
                let mut args = self.log_arguments();
                // with a prefix argument follow renames
                if cmd == "log-buffer-file" && self.prefix_arg.is_some() && !args.iter().any(|a| a == "--follow") {
                    args.insert(0, "--follow".into());
                }
                let mut files = vec![file.clone()];
                let lines = if cmd == "log-trace-definition" {
                    let Some(name) = self.which_function() else {
                        self.set_msg("No function at point found");
                        return Ok(());
                    };
                    Some(format!("-L:{}:{file}", regex_quote(&name).replace(':', "\\:")))
                } else {
                    self.mark.take().and_then(|m| {
                        let line_at = |p: usize| match self.buf.all.get(p).map(|l| &l.kind) {
                            Some(LineKind::FileLine { line, .. }) => Some(*line),
                            _ => None,
                        };
                        let (a, b) = (line_at(m)?, self.file_line()?);
                        Some(format!("-L{},{}:{file}", a.min(b), a.max(b)))
                    })
                };
                if let Some(l) = lines {
                    args.retain(|a| !a.starts_with("-L"));
                    args.insert(0, l);
                    files.clear();
                }
                self.setup_buffer(BufferMode::Log { revs: vec![rev], files }, args);
            }
            "blob-previous" => self.blob_visit(true),
            "blob-next" => self.blob_visit(false),
            // magit-find-file
            "find-file" => {
                let default = self.branch_or_commit_default();
                let Some(rev) = self.read_line_prompt("Find file from revision", default.as_deref())? else {
                    return Ok(());
                };
                let from = if rev == "{worktree}" || rev == "{index}" { "HEAD" } else { rev.as_str() };
                let Some(file) = self.read_file_from_rev(from, "Find file")? else {
                    return Ok(());
                };
                let line = match self.visited_file() {
                    Some((_, f)) if f == file => self.file_line().unwrap_or(1),
                    _ => 1,
                };
                if let Err(e) = super::blame::file_contents(&self.git, &rev, &file) {
                    self.set_msg(&format!("{e}"));
                    return Ok(());
                }
                self.find_file(&rev, &file, line);
            }
            // magit-blob-visit-file
            "blob-visit-file" => {
                let Some((_, file)) = self.visited_file() else {
                    self.set_msg("Not visiting a blob");
                    return Ok(());
                };
                let line = self.file_line().unwrap_or(1);
                self.find_file("{worktree}", &file, line);
            }
            // magit-status-here: the status buffer, at the visited
            // position in the diff of its unstaged or staged changes
            "status-here" => {
                let pos = self.visited_file().map(|(_, f)| (f, self.file_line().unwrap_or(1)));
                self.switch_to_mode(BufferMode::Status);
                if let Some((file, line)) = pos {
                    self.goto_file_position(&file, line, &["unstaged", "staged"]);
                }
            }
            "edit-line-commit" => self.edit_line_commit()?,
            _ => {}
        }
        Ok(())
    }

    /// magit-commit-extend / -reword with a prefix argument (and
    /// magit-commit-*-override-date t): the committer date of HEAD, kept.
    fn keep_committer_date(&self) -> Option<String> {
        self.prefix_arg?;
        self.git.git_string(&["log", "-1", "--format=%cD", "HEAD"]).ok()
    }

    /// magit-branch-rename OLD to NEW (with a prefix argument even if NEW
    /// exists), moving its push-remote setup (magit-branch-rename-push-
    /// target t): branch.OLD.pushRemote becomes branch.NEW.pushRemote and,
    /// if confirmed, the push-target is renamed on the remote.
    fn branch_rename(&mut self, old: &str, new: &str) -> Result<()> {
        let old = old.strip_prefix("heads/").unwrap_or(old);
        let force = self.prefix_arg.is_some();
        if old == new {
            anyhow::bail!("Old and new branch names are the same");
        }
        if !force && self.git.git_ok(&["show-ref", "--verify", "--quiet", &format!("refs/heads/{new}")]) {
            anyhow::bail!("Branch `{new}' already exists");
        }
        let config = |k: &str| self.git.git_string(&["config", k]).ok().filter(|v| !v.is_empty());
        let push_remote = config(&format!("branch.{old}.pushRemote"));
        let remote = push_remote.clone().or_else(|| config("remote.pushDefault"));
        self.git.git_interactive(&["branch", if force { "-M" } else { "-m" }, old, new])?;
        if let Some(r) = &push_remote {
            let _ = self.git.git_ok(&["config", "--unset", &format!("branch.{old}.pushRemote")]);
            let _ = self.git.git_ok(&["config", &format!("branch.{new}.pushRemote"), r]);
        }
        if let Some(remote) = remote {
            let (o, n) = (format!("{remote}/{old}"), format!("{remote}/{new}"));
            if self.git.rev_verify(&o)
                && !self.git.rev_verify(&n)
                && self.yes_no(&format!("Also rename \"{old}\" to \"{new}\" on \"{remote}\"?"))?
            {
                let src = format!("refs/remotes/{o}:refs/heads/{new}");
                let del = format!(":refs/heads/{old}");
                let res = self.git.git_interactive(&["push", "-v", &remote, &src, &del]);
                self.done("renamed", res.map(|()| String::new()));
                return Ok(());
            }
        }
        self.set_msg("renamed");
        self.refresh();
        Ok(())
    }

    /// What the section at point is in the refs buffer.
    fn ref_thing(&self) -> Option<RefThing> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Tag, .. }) => Some(RefThing::Tag),
            Some(LineKind::Ref { .. }) => Some(RefThing::Branch),
            Some(LineKind::Remote { .. }) => Some(RefThing::Remote),
            _ => None,
        }
    }

    /// The branch of the branch section at point (magit-section-value-if
    /// 'branch).
    fn branch_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Local | RefKind::Remote, name }) => Some(name.clone()),
            _ => None,
        }
    }

    /// magit-local-branch-at-point
    fn local_branch_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Local, name }) => Some(name.clone()),
            _ => None,
        }
    }

    /// magit-tag-at-point
    fn tag_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Tag, name }) => Some(name.clone()),
            _ => None,
        }
    }

    /// The default of magit-read-remote: magit-remote-at-point (the remote
    /// section, or the remote of the remote branch section at point),
    /// else magit-get-remote.
    fn remote_default(&self) -> Option<String> {
        let at = match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Remote { name }) => Some(name.clone()),
            Some(LineKind::Ref { kind: RefKind::Remote, .. }) => self.sec(self.cur_sec()).parent.and_then(|parent| {
                match &self.buf.all[self.sec(parent).start].kind {
                    LineKind::Remote { name } => Some(name.clone()),
                    _ => None,
                }
            }),
            _ => None,
        };
        at.or_else(|| {
            let branch = self.git.current_branch()?;
            self.git.git_string(&["config", &format!("branch.{branch}.remote")]).ok().filter(|r| !r.is_empty() && r != ".")
        })
        .or_else(|| {
            let rs = self.git.remotes();
            if rs.len() == 1 {
                rs.first().cloned()
            } else {
                rs.into_iter().find(|r| r == "origin")
            }
        })
    }

    /// magit-remote--cleanup-push-variables: remote.pushDefault and the
    /// branch.*.pushRemote that are REMOTE are set to NEW, or unset.
    fn remote_cleanup_push_variables(&self, remote: &str, new: Option<&str>) {
        let set = |var: &str| match new {
            Some(n) => self.git.git_ok(&["config", var, n]),
            None => self.git.git_ok(&["config", "--unset", var]),
        };
        if self.git.git_string(&["config", "remote.pushDefault"]).ok().as_deref() == Some(remote) {
            set("remote.pushDefault");
        }
        let pattern = format!("^{}$", regex_quote(remote));
        for var in self.git.git_lines_pub(&["config", "--name-only", "--get-regexp", "^branch\\.[^.]*\\.pushRemote", &pattern]) {
            set(&var);
        }
    }

    /// magit-region-values of the ref sections of KINDS (at least two if
    /// MULTIPLE), if the region selects such sections only.
    fn region_refs(&self, kinds: &[RefKind], multiple: bool) -> Option<Vec<String>> {
        if self.mark.is_none() || self.mark == Some(self.point()) {
            return None;
        }
        let secs = self.region_sections(multiple)?;
        secs.iter()
            .map(|&s| match &self.buf.all[self.sec(s).start].kind {
                LineKind::Ref { kind, name } if kinds.contains(kind) => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// magit-branch-merged-p: BRANCH is merged into the current branch
    /// (never when HEAD is detached) and, if it has one, its upstream.
    fn branch_merged(&self, branch: &str) -> bool {
        match self.git.current_branch() {
            Some(c) => self.branch_merged_into(branch, Some(&c)),
            None => false,
        }
    }

    /// magit-branch-merged-p with TARGET; None: into any other local
    /// branch.
    fn branch_merged_into(&self, branch: &str, target: Option<&str>) -> bool {
        if self.upstream_branch(branch).is_some_and(|u| !self.git.is_ancestor(branch, &u)) {
            return false;
        }
        match target {
            Some(t) => self.git.is_ancestor(branch, t),
            None => self
                .git
                .git_lines(&["for-each-ref", "--contains", branch, "--format=%(refname:short)", "refs/heads"])
                .iter()
                .any(|b| b != branch),
        }
    }

    /// magit-get-upstream-branch
    fn upstream_branch(&self, branch: &str) -> Option<String> {
        self.git
            .git_string(&["rev-parse", "--abbrev-ref", &format!("{branch}@{{upstream}}")])
            .ok()
            .filter(|u| !u.is_empty())
    }

    /// magit-get-indirect-upstream-branch (forced): the upstream of
    /// BRANCH if it is the remote branch of the same name and can be
    /// fast-forwarded to BRANCH.
    fn indirect_upstream_branch(&self, branch: &str) -> Option<String> {
        let remote = self.git.git_string(&["config", &format!("branch.{branch}.remote")]).ok()?;
        if remote.is_empty() || remote == "." {
            return None;
        }
        let upstream = self.upstream_branch(branch)?;
        (upstream.strip_prefix(&format!("{remote}/")) == Some(branch) && self.git.is_ancestor(&upstream, branch))
            .then_some(upstream)
    }

    /// The default of magit-read-branch-prefer-other: of the branches at
    /// the commit at point the only one, else one that isn't current,
    /// else the previous branch.
    fn branch_prefer_other_default(&self) -> Option<String> {
        // (magit-section-value-if 'branch)
        if let Some(b) = self.branch_at_point() {
            return Some(b);
        }
        let current = self.git.current_branch();
        let atrev: Vec<String> = match self.commit_at_point() {
            Some(c) => self.git.git_lines(&[
                "for-each-ref",
                "--points-at",
                &c,
                "--format=%(refname:short)",
                "refs/heads",
                "refs/remotes",
            ]),
            None => Vec::new(),
        };
        let previous = self
            .git
            .git_string(&["rev-parse", "--abbrev-ref", "@{-1}"])
            .ok()
            .filter(|b| !b.is_empty() && b != "HEAD");
        (atrev.len() == 1)
            .then(|| atrev[0].clone())
            .or_else(|| atrev.iter().find(|b| Some(*b) != current.as_ref()).cloned())
            .or(previous)
            .or_else(|| atrev.first().cloned())
    }

    /// magit-branch-delete: delete the branches selected in the refs
    /// buffer, else one read (local or remote-tracking). Deleting an
    /// unmerged branch has to be confirmed, unless forced by a prefix
    /// argument. Deleting remote-tracking branches offers to delete the
    /// branches on the remote too; deleting the current branch to detach
    /// HEAD or to check out another branch first.
    fn branch_delete(&mut self) -> Result<()> {
        let mut force = self.prefix_arg.is_some();
        // (equal force '(4)): no confirmation for the current branch
        let forced = self.prefix_arg == Some(PrefixArg::Raw(4));
        let mut confirmed = false;
        let mut branches = match self.region_refs(&[RefKind::Local, RefKind::Remote], true) {
            Some(bs) if bs.len() > 1 => {
                if !self.confirm(&format!("Delete {} branches", bs.len()))? {
                    return Ok(());
                }
                bs
            }
            _ => {
                let mut choices = self.git.branches();
                choices.extend(self.git.remote_branches());
                let default = self.branch_prefer_other_default();
                let prompt = if force { "Force delete branch" } else { "Delete branch" };
                match self.pick(prompt, &choices, default.as_deref())? {
                    Some(b) => vec![b.clone()],
                    None => return Ok(()),
                }
            }
        };
        if !force {
            let unmerged: Vec<String> = branches.iter().filter(|b| !self.branch_merged(b)).cloned().collect();
            if !unmerged.is_empty() {
                let prompt = match unmerged.as_slice() {
                    [b] => format!("Delete unmerged branch {b}"),
                    _ => format!("Delete {} unmerged branches", unmerged.len()),
                };
                // 'noabort: declining deletes the merged ones only
                if self.confirm(&prompt)? {
                    force = true;
                    confirmed = true;
                } else {
                    branches.retain(|b| !unmerged.contains(b));
                    if branches.is_empty() {
                        self.set_msg("Abort");
                        return Ok(());
                    }
                }
            }
        }
        let fulls: Vec<Option<String>> = branches
            .iter()
            .map(|b| {
                self.git.git_string(&["rev-parse", "--verify", "--symbolic-full-name", b]).ok().filter(|f| !f.is_empty())
            })
            .collect();
        let ambiguous = fulls.iter().filter(|f| f.is_none()).count();
        if ambiguous > 0 {
            let what = if ambiguous == 1 {
                let i = fulls.iter().position(Option::is_none).unwrap_or(0);
                format!("{} is", branches[i])
            } else if ambiguous == fulls.len() {
                format!("These {ambiguous} names are")
            } else {
                format!("{ambiguous} of these names are")
            };
            self.set_msg(&format!("{what} ambiguous; please cleanup using git directly"));
            return Ok(());
        }
        let fulls: Vec<String> = fulls.into_iter().flatten().collect();
        if fulls[0].starts_with("refs/remotes/") {
            return self.delete_remote_tracking_branches(&branches, &fulls, force);
        }
        if branches.len() > 1 {
            let current = self.git.current_branch();
            branches.retain(|b| Some(b) != current.as_ref());
            for b in &branches {
                let _ = self.git.git_ok(&["config", "--unset", &format!("branch.{b}.pushRemote")]);
            }
            let mut a = vec!["branch", if force { "-D" } else { "-d" }];
            a.extend(branches.iter().map(String::as_str));
            let res = self.git.git_interactive(&a);
            self.done(&format!("deleted {}", branches.join(", ")), res.map(|()| String::new()));
            return Ok(());
        }
        let branch = branches.remove(0);
        if self.git.current_branch().as_deref() == Some(branch.as_str()) {
            let mut target = self.indirect_upstream_branch(&branch);
            if target.is_none() || target.as_deref() == Some(branch.as_str()) {
                target = self.git.main_branch();
            }
            let target = target.filter(|t| *t != branch);
            let prompt = match &target {
                Some(t) => format!("Branch {branch} is checked out.  [d]etach HEAD & delete, [c]heckout {t} & delete, [a]bort"),
                None => format!("Branch {branch} is checked out.  [d]etach HEAD & delete, [a]bort"),
            };
            let answer = self.read_line_prompt(&prompt, None)?.unwrap_or_default();
            let checkout = match (answer.chars().next(), &target) {
                (Some('d'), _) => None,
                (Some('c'), Some(t)) => Some(t.clone()),
                _ => {
                    self.set_msg("Abort");
                    return Ok(());
                }
            };
            if !forced
                && !confirmed
                && !self.branch_merged_into(&branch, checkout.as_deref())
                && !self.confirm(&format!("Delete unmerged branch {branch}"))?
            {
                return Ok(());
            }
            match &checkout {
                Some(t) => self.git.git_interactive(&["checkout", "-q", t])?,
                None => self.git.git_interactive(&["checkout", "-q", "--detach"])?,
            }
            force = true;
        }
        // magit-branch-unset-pushRemote
        let _ = self.git.git_ok(&["config", "--unset", &format!("branch.{branch}.pushRemote")]);
        let res = self.git.git_interactive(&["branch", if force { "-D" } else { "-d" }, &branch]);
        self.done(&format!("deleted {branch}"), res.map(|()| String::new()));
        Ok(())
    }

    /// magit-branch-delete of the remote-tracking BRANCHES (their full
    /// refs FULLS, all of the remote of the first): also on the remote if
    /// confirmed (with FORCE without verifying), else just the local refs.
    fn delete_remote_tracking_branches(&mut self, branches: &[String], fulls: &[String], force: bool) -> Result<()> {
        let short = &fulls[0]["refs/remotes/".len()..];
        let Some(remote) = self.git.remotes().into_iter().find(|r| short.starts_with(&format!("{r}/"))) else {
            self.set_msg(&format!("Invalid branch name {short}"));
            return Ok(());
        };
        let offset = remote.len() + 1;
        let was: Vec<String> = fulls.iter().map(|f| format!("Delete {f} (was {})", self.git.rev_short(f))).collect();
        let prompt = match fulls {
            [f] => format!("Deleting local {f}.  Also delete on {remote}?"),
            _ => format!("Deleting {} local refs.  Also delete on {remote}?", fulls.len()),
        };
        if !self.yes_no(&prompt)? {
            let mut res = Ok(());
            for f in fulls {
                res = res.and(self.git.git_interactive(&["update-ref", "-d", f]));
            }
            self.done(&was.join("; "), res.map(|()| String::new()));
            return Ok(());
        }
        let refspecs: Vec<String> = branches.iter().map(|b| format!("refs/heads/{}", &b[offset.min(b.len())..])).collect();
        let mut a = vec!["push", "--delete"];
        if force {
            a.push("--no-verify");
        }
        a.push(&remote);
        a.extend(refspecs.iter().map(String::as_str));
        if let Err(e) = self.git.git_interactive(&a) {
            // magit-delete-remote-branch-sentinel: the branches that no
            // longer exist on the remote; delete just their tracking refs
            let on_remote: Vec<String> = self
                .git
                .git_lines_pub(&["ls-remote", "--heads", &remote])
                .iter()
                .filter_map(|l| l.split('\t').nth(1).map(str::to_string))
                .collect();
            let gone: Vec<&String> = fulls
                .iter()
                .zip(&refspecs)
                .filter(|(f, r)| !on_remote.contains(r) && self.git.git_ok(&["show-ref", "--verify", "--quiet", f]))
                .map(|(f, _)| f)
                .collect();
            if gone.is_empty() {
                self.set_msg(&format!("{e}"));
                return Ok(());
            }
            let mut res = Ok(());
            for f in gone {
                res = res.and(self.git.git_interactive(&["update-ref", "-d", f]));
            }
            let msg = "Some remote branches no longer exist.  Deleting just the local tracking refs instead...";
            self.done(msg, res.map(|()| String::new()));
            return Ok(());
        }
        self.done(&was.join("; "), Ok(String::new()));
        Ok(())
    }

    /// magit-which-function: the name of the definition around point.
    fn which_function(&self) -> Option<String> {
        const KEYWORDS: [&str; 7] = ["fn", "def", "func", "function", "(defun", "class", "sub"];
        let i = self.cursor.min(self.lines.len().checked_sub(1)?);
        self.lines[..=i].iter().rev().filter(|l| matches!(l.kind, LineKind::FileLine { .. })).find_map(|l| {
            let words: Vec<&str> = l.text.split_whitespace().collect();
            let k = words.iter().position(|w| KEYWORDS.contains(w))?;
            let name: String = words
                .get(k + 1)?
                .chars()
                .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ':' | '.' | '$'))
                .collect();
            (!name.is_empty()).then_some(name)
        })
    }

    /// magit-edit-line-commit: make the commit that added the line at
    /// point editable, by an interactive rebase if it is reachable from
    /// HEAD, else by checking it (or a branch at it) out.
    fn edit_line_commit(&mut self) -> Result<()> {
        let Some((rev, file)) = self.visited_file() else {
            self.set_msg("Buffer isn't visiting a file");
            return Ok(());
        };
        let blame_type = match &self.mode {
            BufferMode::File { blame: Some(b), .. } => Some(b.typ),
            _ => None,
        };
        // with a prefix argument the commit that removes the line
        let typ = if self.prefix_arg.is_some() { BlameType::Removal } else { BlameType::Addition };
        let chunk = if blame_type == Some(typ) {
            self.current_blame_chunk()
        } else {
            // magit-current-blame-chunk: blame the line at point, in
            // reverse if the buffer is blamed so
            let line = self.file_line().unwrap_or(1);
            let r = (rev != "{worktree}" && rev != "{index}").then_some(rev.as_str());
            let range = vec!["-L".to_string(), format!("{line},{line}")];
            let reverse = matches!(blame_type, Some(BlameType::Final | BlameType::Removal));
            super::blame::run_blame_parsed(&self.git, r, &file, &range, reverse, typ)?.into_iter().next()
        };
        let Some(chunk) = chunk else {
            self.set_msg("No chunk at point");
            return Ok(());
        };
        if chunk.orig_rev.chars().all(|c| c == '0') {
            self.set_msg("This line has not been committed yet");
            return Ok(());
        }
        let rebase = self.git.is_ancestor(&chunk.orig_rev, "HEAD");
        if rebase {
            let args = self.prefix_args("rebase-menu");
            self.rebase_commit("rebase-edit-commit", &chunk.orig_rev, &args)?;
        } else {
            let branch = self
                .git
                .git_lines(&["for-each-ref", "--points-at", &chunk.orig_rev, "--format=%(refname:short)", "refs/heads"])
                .into_iter()
                .next();
            let target = branch.unwrap_or_else(|| chunk.orig_rev.clone());
            let res = self.git.checkout(&target);
            self.done("checked out", res.map(|()| String::new()));
        }
        if chunk.orig_file != file || rev != "{worktree}" {
            self.find_file("{worktree}", &chunk.orig_file, chunk.orig_line);
            if let Some(t) = blame_type {
                let args = self.prefix_args("blame-menu");
                self.blame(t, args)?;
            }
        }
        Ok(())
    }

    /// The name of the buffer of MODE (magit-generate-buffer-name-default-function).
    fn buffer_name_of(&self, mode: &BufferMode) -> String {
        let repo = self.git.toplevel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let m = match mode {
            BufferMode::Status => "magit",
            BufferMode::Diff(_) => "magit-diff",
            BufferMode::Revision { .. } => "magit-revision",
            BufferMode::Stash { .. } => "magit-stash",
            BufferMode::Log { .. } => "magit-log",
            BufferMode::Reflog { .. } => "magit-reflog",
            BufferMode::Stashes { .. } => "magit-stashes",
            BufferMode::Cherry { .. } => "magit-cherry",
            BufferMode::LogSelect { .. } => "magit-log-select",
            BufferMode::Refs { .. } => "magit-refs",
            BufferMode::File { rev, file, .. } => return super::blame::buffer_name(rev, file),
        };
        format!("{m}: {repo}")
    }

    /// magit-display-repository-buffer (J, and G of magit-file-dispatch)
    fn display_repository_buffer(&mut self) -> Result<()> {
        // magit-mode-get-buffers in buffer-list order: the buffer shown,
        // those below it, then the buried ones
        let mut names = vec![self.buffer_name_of(&self.mode)];
        names.extend(self.buried.iter().rev().map(|b| self.buffer_name_of(&b.mode)));
        names.extend(self.background.iter().map(|b| self.buffer_name_of(&b.mode)));
        let status = self.buffer_name_of(&BufferMode::Status);
        let default = names.contains(&status).then_some(status);
        let Some(name) = self.read_choice("Display magit buffer", &names, default.as_deref())? else {
            return Ok(());
        };
        if let Some(i) = self.buried.iter().position(|b| self.buffer_name_of(&b.mode) == name) {
            let b = self.buried.remove(i);
            self.raise_buffer(b);
        } else if let Some(i) = self.background.iter().position(|b| self.buffer_name_of(&b.mode) == name) {
            let b = self.background.remove(i);
            self.raise_buffer(b);
        }
        Ok(())
    }

    /// Show the buffer B (taken from below or from the buried ones) over
    /// the buffer shown.
    fn raise_buffer(&mut self, b: SavedBuffer) {
        let saved = SavedBuffer {
            mode: std::mem::replace(&mut self.mode, b.mode),
            args: std::mem::replace(&mut self.buf_args, b.args),
            buf: std::mem::replace(&mut self.buf, b.buf),
            vis: std::mem::replace(&mut self.vis, b.vis),
            cursor: self.cursor,
            scroll: self.scroll,
        };
        self.buried.push(saved);
        self.view_text = None;
        self.lines = self.buf.visible(&self.vis);
        self.cursor = b.cursor.min(self.lines.len().saturating_sub(1));
        self.scroll = b.scroll;
        self.refresh();
    }

    /// Switch to the buffer of MODE: the one shown, a buried one, or a new one.
    fn switch_to_mode(&mut self, mode: BufferMode) {
        if self.mode.same_kind(&mode) {
            self.view_text = None;
            self.refresh();
        } else if let Some(i) = self.buried.iter().position(|b| b.mode.same_kind(&mode)) {
            let b = self.buried.remove(i);
            self.raise_buffer(b);
        } else if let Some(i) = self.background.iter().position(|b| b.mode.same_kind(&mode)) {
            let b = self.background.remove(i);
            self.raise_buffer(b);
        } else {
            self.setup_buffer(mode, Vec::new());
        }
    }

    /// magit-diff--goto-file-position / magit-status--goto-file-position:
    /// go to LINE of FILE in the diffs of GROUPS (all when empty): the
    /// diff line, else the file section; the sections around it are shown.
    fn goto_file_position(&mut self, file: &str, line: usize, groups: &[&str]) {
        let groups: Vec<String> = if groups.is_empty() {
            self.buf.diffs.iter().map(|(g, _)| g.clone()).collect()
        } else {
            groups.iter().map(|g| g.to_string()).collect()
        };
        let mut target = None;
        'find: for group in &groups {
            let Some(fd) = self.buf.diff(group, file) else { continue };
            for (h, hunk) in fd.hunks.iter().enumerate() {
                let Some(r) = hunk.range() else { continue };
                let mut n = r.new_start;
                for (li, l) in hunk.lines.iter().enumerate() {
                    if l.starts_with('-') || l.starts_with('\\') {
                        continue;
                    }
                    if n == line {
                        let kind = LineKind::DiffLine { section: group.clone(), path: file.into(), hunk: h, line: li };
                        target = self.buf.all.iter().position(|b| b.kind == kind);
                        break 'find;
                    }
                    n += 1;
                }
            }
        }
        if target.is_none() {
            target = groups.iter().find_map(|group| {
                let kind = LineKind::File { section: group.clone(), path: file.into() };
                self.buf.all.iter().position(|b| b.kind == kind)
            });
        }
        let Some(p) = target else { return };
        // magit-section-reveal
        let ancestors = self.buf.all[p].ancestors.clone();
        for a in ancestors {
            self.vis.insert(a, false);
        }
        self.lines = self.buf.visible(&self.vis);
        self.cursor = self.visible_index_near(p);
    }

    /// The keys of a file or blob buffer: magit-blame-read-only-mode-map
    /// while blaming, else magit-blob-mode-map. Returns whether KEY is
    /// bound there.
    fn file_key(&mut self, key: &Key) -> Result<bool> {
        let BufferMode::File { blame, .. } = &self.mode else { return Ok(false) };
        let blaming = blame.is_some();
        let typ = blame.as_ref().map(|b| b.typ);
        let label = key.label();
        match label.as_str() {
            "b" | "r" | "f" => {
                let t = match label.as_str() {
                    "b" => BlameType::Addition,
                    "r" => BlameType::Removal,
                    _ => BlameType::Final,
                };
                let args = match &self.mode {
                    BufferMode::File { blame: Some(b), .. } if Some(t) == typ => b.args.clone(),
                    _ => self.prefix_args("blame-menu"),
                };
                self.blame(t, args)?;
            }
            "B" => self.open_menu("blame-menu"),
            "q" if blaming => self.blame_quit(),
            "q" => self.kill_buffer(),
            "n" if blaming => self.blame_goto_chunk(true, false),
            "p" if blaming => self.blame_goto_chunk(false, false),
            "N" if blaming => self.blame_goto_chunk(true, true),
            "P" if blaming => self.blame_goto_chunk(false, true),
            "n" => self.blob_visit(false),
            "p" => self.blob_visit(true),
            "c" if blaming => self.blame_cycle_style(),
            "M-w" if blaming => match self.current_blame_chunk() {
                Some(c) => self.set_msg(&c.orig_rev),
                None => self.set_msg("No chunk at point"),
            },
            "RET" | "SPC" | "backspace" if blaming => match self.current_blame_chunk() {
                Some(c) if c.orig_rev.chars().all(|x| x == '0') => self.set_msg("Not Yet Committed"),
                Some(c) => self.show_commit(&c.orig_rev),
                None => self.set_msg("No chunk at point"),
            },
            "g" => {
                let (line, _) = self.file_line_at_point().unwrap_or((1, None));
                self.refresh();
                self.goto_file_line(line);
            }
            "C-n" | "down" | "C-p" | "up" => {
                let before = self.current_blame_chunk().map(|c| c.final_line);
                self.forward_line(if matches!(label.as_str(), "C-n" | "down") { 1 } else { -1 });
                if blaming && self.current_blame_chunk().map(|c| c.final_line) != before {
                    self.blame_show_message();
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
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
            // magit-visit-ref (magit-visit-ref-behavior nil)
            LineKind::Ref { name, .. } => self.show_commit(&name),
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
                    | BufferMode::LogSelect { .. }
                    | BufferMode::Reflog { .. }
                    | BufferMode::Stashes { .. }
                    | BufferMode::Cherry { .. }
                    | BufferMode::Refs { .. }
                    | BufferMode::File { .. } => {
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
            // the commit at point, else one selected from the log
            // (magit-rebase-interactive-1)
            "rebase-interactive" | "rebase-edit-commit" | "rebase-reword-commit" | "rebase-remove-commit" => {
                match self.commit_at_point() {
                    Some(commit) => self.rebase_commit(cmd, &commit, args)?,
                    None => {
                        let msg = match cmd {
                            "rebase-interactive" => "Type %p on a commit to rebase it and all commits above it,",
                            "rebase-edit-commit" => "Type %p on a commit to edit it,",
                            "rebase-reword-commit" => "Type %p on a commit to reword its message,",
                            _ => "Type %p on a commit to remove it,",
                        };
                        let action = SelectAction::Rebase { cmd: cmd.to_string(), args: args.to_vec() };
                        self.log_select(action, Some(msg), None);
                    }
                }
            }
            "rebase-subset" => {
                let default = self.git.upstream_shortname();
                let Some(newbase) = self.read_line_prompt("Rebase subset onto", default.as_deref())? else {
                    return Ok(());
                };
                let msg = format!("Type %p on a commit to rebase it and commits above it onto {newbase},");
                self.log_select(SelectAction::RebaseSubset { newbase, args: args.to_vec() }, Some(&msg), None);
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
    if let Some(f) = super::transient::default_values_file() {
        app.load_values(f);
    }
    app.run()
}
