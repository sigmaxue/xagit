//! ui/app.rs — application state machine, event loop and command dispatch.
//!
//! `impl App` is spread over the `app::*` submodules (see README.md).

use super::keys::{key_from_label, Key};
use super::blame::{blob_ancestor, blob_successor, build_file, style_shows_message, Blame, BlameType, ECHO_STYLE, STYLES};
use super::transient::{
    blame_menu, diff_refresh_menu, file_dispatch_menu, log_refresh_menu, prefix_value, registry, resolve_menu,
    show_refs_menu, Action,
    Lookup, Menu, MenuEntry, Reader, RepoState, STICKY_COMMANDS, VALUE_COMMANDS,
};
use super::diffbuf::{build_diff, build_revision, build_stash, DiffSpec};
use super::help;
use super::logbuf::{
    build_cherry, build_log, build_reflog, build_stashes, set_commit_limit, Limit, Margin, REFLOG_LIMIT,
};
use super::minibuf::{add_to_history, Completion, LineEditor, Step};
use super::options::Options;
use super::paint::{self, Row};
use super::theme::{Face, Style, Theme};
use super::refs::{build_refs, deferred_ref, CommitCount, RefKind};
use super::smerge::{self, Keep};
use super::view::{build_status, status_log_args, BufferLine, LineKind, Section, StatusBuffer, Visibility};
use crate::git::apply::diff_context;
use crate::git::diff::{FileDiff, Scope};
use crate::git::exec::Editor;
use crate::git::status::{file_status, FileStatus, Status};
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

/// Give the terminal back as it was: colors and attributes reset, the
/// cursor shown, the main screen, cooked mode.
fn restore_terminal() {
    let _ = execute!(
        stdout(),
        crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
        crossterm::style::ResetColor,
        crossterm::cursor::Show,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}

mod gitvar;
mod jobs;
mod isearch;
mod run;
mod remote;
mod reshelve;
mod bisect;
mod jump;
mod notes;
mod patch;
mod spread;
mod submodule;
mod subtree;
mod gitignore;
mod stash;
mod tag;
mod worktree;
mod rebase_todo;
mod resolve;
mod render;
mod input;
mod sections;
mod menus;
mod apply;
mod commands;
mod views;
mod diffsel;
mod visit;
mod refs;
mod buffers;
mod extra;

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
    /// the query of the buffer search (C-s), highlighted while active
    search_query: Option<String>,
    /// the last query, offered as the default of the next search
    last_search: String,
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
    /// C-x was typed (the prefix of C-x C-c)
    cx_prefix: bool,
    /// C-c ^ was typed on a hunk (magit-hunk-section-smerge-map)
    smerge_prefix: bool,
    /// C-c typed in the git-rebase-mode buffer
    rebase_todo_cc: bool,
    /// the prefix argument of the next command (current-prefix-arg)
    prefix_arg: Option<PrefixArg>,
    /// C-u / M-- was typed and digits are read (universal-argument-map)
    reading_arg: bool,
    /// transient--helpp: `?` was typed in a transient, the next key
    /// shows the help of its suffix
    transient_helpp: bool,
    /// the transient is suspended to show help (transient-resume-mode):
    /// the text view and its scroll position to restore on `q`
    transient_help: Option<(Option<(String, String)>, usize)>,
    /// transient--stack after transient-suspend (C-z): the transients
    /// and their values, resumed by transient-resume
    suspended: Vec<(String, BTreeMap<String, bool>)>,
    /// where info manuals and the Lisp sources are found (help)
    help_dirs: Vec<std::path::PathBuf>,
    /// magit-blame-previous-chunk: (buffer, commit, first line) of the
    /// chunk point was in after the last command
    blame_previous_chunk: Option<(String, String, usize)>,
    /// magit--right-margin-config of the buffer of each mode (permanent)
    margins: HashMap<&'static str, Margin>,
    /// magit-refs-show-commit-count of the refs buffer
    refs_counts: CommitCount,
    message: String,
    quit: bool,
    /// pending text-mode buffer shown instead of status (diff/log/show output)
    view_text: Option<(String, String)>, // (title, content)
    view_scroll: usize,
    /// the git-rebase-todo being edited (BufferMode::RebaseTodo)
    rebase_todo: Option<RebaseTodoEdit>,
    /// the conflict resolution view (BufferMode::Resolve)
    resolve: Option<resolve::ResolveEdit>,
    /// directory xagit was started in (magit-git-command's default-directory)
    pub cwd: std::path::PathBuf,
    /// arguments of new diff / revision / stash buffers (the current value
    /// of magit-diff, initially magit-diff-mode's "--stat --no-ext-diff")
    pub diff_args: Vec<String>,
    /// arguments of new log buffers (the current value of magit-log-mode,
    /// initially "--graph -n256 --decorate")
    pub log_args: Vec<String>,
    /// answers used instead of reading from the terminal (tests)
    scripted: std::cell::RefCell<std::collections::VecDeque<String>>,
    /// answers typed as keys into the minibuffer (tests), used when there
    /// are no scripted answers
    scripted_keys: std::cell::RefCell<std::collections::VecDeque<Vec<Key>>>,
    /// keys answering the question being read at once (y-or-n-p,
    /// read-char-choice): key code -> answer
    instant_keys: std::cell::RefCell<Vec<(String, String)>>,
    /// the INITIAL-INPUT of the next question read
    initial_input: std::cell::RefCell<Option<String>>,
    /// set while the search prompt (C-s / C-r) is read; a second C-s / C-r
    /// there submits the text typed so far, and records the direction
    search_read: std::cell::RefCell<Option<SearchRead>>,
    /// the scopes read by magit-branch-configure and
    /// magit-remote-configure (transient-scope)
    var_scopes: HashMap<String, Option<String>>,
    /// the transient a suffix is invoked from (transient-current-command)
    invoking_menu: Option<String>,
    /// the rows of a help window shown above the minibuffer while a
    /// question is read (read-multiple-choice's help)
    minibuffer_above: std::cell::RefCell<Vec<String>>,
    /// user-init-file: the options file read (enable-command saves
    /// there)
    user_init_file: Option<std::path::PathBuf>,
    /// minibuffer-history: the inputs of prompts without a history of
    /// their own, newest first (not saved)
    minibuffer_history: std::cell::RefCell<Vec<String>>,
    /// magit-revision-history: the branches and commits read, newest
    /// first (not saved)
    revision_history: std::cell::RefCell<Vec<String>>,
    /// the rows drawn last (the minibuffer restores those it covered)
    screen: std::cell::RefCell<Vec<Row>>,
    /// the terminal size `screen` was painted at
    painted_size: (usize, usize),
    /// whether the terminal may no longer show `screen` (something else
    /// wrote to it), so the next frame must be painted in full
    repaint_all: bool,
    /// put point on magit-status-initial-section when the next refresh
    /// lands (the first one, which may finish on a worker)
    initial_section: bool,
    /// the faces' colors for this terminal (detected when run in one)
    theme: Theme,
    /// the history of each infix (its values read, newest first)
    infix_history: std::cell::RefCell<BTreeMap<String, Vec<String>>>,
    /// transient-history-file
    history_file: Option<std::path::PathBuf>,
    /// the user options (magit-no-confirm and friends)
    options: Options,
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
    /// refreshes and git processes running in the background
    jobs: jobs::Jobs,
    /// HEAD as the header line shows it (taken when refreshing)
    head_label: String,
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
    /// git-rebase-mode: the todo list of an interactive rebase edited in
    /// xagit (the list itself is App::rebase_todo)
    RebaseTodo,
    /// the built-in conflict resolution view (`e` with conflicts): its
    /// state is App::resolve
    Resolve,
}

/// An interactive rebase whose todo list is being edited in xagit.
#[derive(Debug, Clone)]
pub struct RebaseTodoEdit {
    pub todo: super::rebase_todo::Todo,
    pub cursor: usize,
    pub mark: Option<usize>,
    /// what git is run with once the list is finished
    pub target: RebaseTodoTarget,
}

/// Where the finished todo list goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebaseTodoTarget {
    /// start `git rebase -i` for COMMIT with ARGS
    Start { commit: String, args: Vec<String> },
    /// `git rebase --edit-todo` of the rebase in progress
    InProgress,
}

/// What is done with the commit picked in a log-select buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectAction {
    /// magit-rebase-interactive / -edit-commit / -reword-commit /
    /// -remove-commit (CMD is the xagit command)
    Rebase { cmd: String, args: Vec<String> },
    /// magit-rebase-subset onto NEWBASE
    RebaseSubset { newbase: String, args: Vec<String> },
    /// magit-rebase-autosquash
    RebaseAutosquash { args: Vec<String> },
    /// magit-commit-squash-internal with OPTION ("--fixup=", "--squash=",
    /// "--fixup=amend:" or "--fixup=reword:")
    CommitSquash { option: String, args: Vec<String>, edit: bool, rebase: bool },
    /// magit-reshelve-since, signing with KEYID
    ReshelveSince { keyid: Option<String> },
    /// magit-commit-autofixup with ARGS
    Autofixup { args: Vec<String> },
    /// magit-commit-absorb-modules
    AbsorbModules,
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

/// Whether REMOTE is an url or path rather than the name of a remote
/// ("\\(\\`.\\{0,2\\}/\\|[:@]\\)").
fn unnamed_remote(remote: &str) -> bool {
    remote.contains([':', '@']) || remote.chars().take(3).position(|c| c == '/').is_some()
}

/// magit--unnamed-upstream-p: REMOTE an url or path, MERGE a full ref.
fn unnamed_upstream(remote: Option<&str>, merge: Option<&str>) -> bool {
    remote.is_some_and(|r| r.contains([':', '@']) || r.starts_with('/') || r.starts_with("./") || r.starts_with("../"))
        && merge.is_some_and(|m| m.starts_with("refs/"))
}

/// magit--valid-upstream-p: REMOTE "." or one of REMOTES, MERGE a full ref.
fn valid_upstream(remote: Option<&str>, merge: Option<&str>, remotes: &[String]) -> bool {
    remote.is_some_and(|r| r == "." || remotes.iter().any(|x| x == r)) && merge.is_some_and(|m| m.starts_with("refs/"))
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
    /// search in the current buffer (C-s / C-r)
    IsearchForward,
    IsearchBackward,
    Visit,
    Menu(&'static str),
    Command(&'static str),
}

impl Binding {
    /// Whether this is a binding of the global map, which file buffers
    /// have too.
    fn is_global(&self) -> bool {
        matches!(
            self,
            Binding::Down
                | Binding::Up
                | Binding::SetMark
                | Binding::KeyboardQuit
                | Binding::IsearchForward
                | Binding::IsearchBackward
                | Binding::Command("execute-extended-command")
        )
    }
}

/// The history list of a minibuffer read (its HIST argument).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hist<'a> {
    /// minibuffer-history
    Minibuffer,
    /// magit-revision-history
    Revision,
    /// the history of the infix of that argument
    Infix(&'a str),
    /// none (y-or-n-p)
    None,
}

/// The state of one read of the search prompt. While reading, a further
/// C-s / C-r submits the text typed so far (as RET would) and records
/// the direction, so repeating C-s searches without a RET in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchRead {
    Reading,
    Forward,
    Backward,
}

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
            PrefixArg::Raw(n) => format!("C-u ({n})"),
            PrefixArg::Num(n) => format!("C-u {n}"),
            PrefixArg::Minus => "C-u -".into(),
        }
    }
}

/// The name of the Emacs command of the binding B.
pub fn binding_command(b: &Binding) -> String {
    let name = match b {
        Binding::Bury => "magit-mode-bury-buffer",
        Binding::Refresh => "magit-refresh",
        Binding::Down => "next-line",
        Binding::Up => "previous-line",
        Binding::SectionForward => "magit-section-forward",
        Binding::SectionBackward => "magit-section-backward",
        Binding::SiblingForward => "magit-section-forward-sibling",
        Binding::SiblingBackward => "magit-section-backward-sibling",
        Binding::Toggle => "magit-section-toggle",
        Binding::Cycle => "magit-section-cycle",
        Binding::CycleGlobal => "magit-section-cycle-global",
        Binding::CycleDiffs => "magit-section-cycle-diffs",
        Binding::ShowLevel(n) => return format!("magit-section-show-level-{n}"),
        Binding::ShowLevelAll(n) => return format!("magit-section-show-level-{n}-all"),
        Binding::SetMark => "set-mark-command",
        Binding::KeyboardQuit => "keyboard-quit",
        Binding::IsearchForward => "isearch-forward",
        Binding::IsearchBackward => "isearch-backward",
        Binding::Visit => "magit-visit-thing",
        Binding::Command("stage-all") => "magit-stage-modified",
        Binding::Command("jump") => "magit-status-jump",
        Binding::Command("section-info") => "magit-describe-section",
        Binding::Command("cherries") => "magit-cherry",
        Binding::Command("display-buffer") => "magit-display-repository-buffer",
        Binding::Command("untrack") => "magit-file-untrack",
        Binding::Command("rename-thing") => "magit-file-rename",
        Binding::Command("discard") => "magit-discard",
        Binding::Command("execute-extended-command") => "execute-extended-command",
        Binding::Command(c) => return format!("magit-{c}"),
        // Keep one source of truth for prefix command names (the same
        // function the help module uses), including its special cases.
        Binding::Menu(m) => return crate::ui::help::prefix_command(m),
    };
    name.to_string()
}

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
            "s" => Some(IsearchForward),
            "r" => Some(IsearchBackward),
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
        "M-x" => Command("execute-extended-command"),
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

/// ASCII-folded bytes of TEXT. Folding leaves byte lengths unchanged, so
/// indices stay at UTF-8 character boundaries and non-ASCII text is matched
/// literally (no locale-dependent case conversion).
fn folded_bytes(text: &str) -> Vec<u8> {
    text.bytes().map(|b| b.to_ascii_lowercase()).collect()
}

/// The byte ranges of QUERY's occurrences in TEXT.
fn search_ranges(text: &str, query: &[u8]) -> Vec<(usize, usize)> {
    let folded = folded_bytes(text);
    if query.is_empty() || query.len() > folded.len() {
        return Vec::new();
    }
    let mut v = Vec::new();
    let mut at = 0;
    while at + query.len() <= folded.len() {
        if &folded[at..at + query.len()] == query {
            v.push((at, at + query.len()));
            at += query.len();
        } else {
            at += 1;
        }
    }
    v
}

/// TEXT split into (part, is-a-match) pieces for the search highlighting.
fn search_parts<'a>(text: &'a str, query: &[u8]) -> Vec<(&'a str, bool)> {
    let mut out = Vec::new();
    let mut last = 0;
    for (a, b) in search_ranges(text, query) {
        if !text.is_char_boundary(a) || !text.is_char_boundary(b) {
            continue;
        }
        if a > last {
            out.push((&text[last..a], false));
        }
        out.push((&text[a..b], true));
        last = b;
    }
    if last < text.len() {
        out.push((&text[last..], false));
    }
    out
}

impl App {
    pub fn new(git: Git) -> App {
        let mut app = App::unbuilt(git);
        app.refresh();
        app
    }

    /// An App whose status buffer is not built yet: `run` builds it in
    /// the background, so the terminal shows something right away.
    fn unbuilt(git: Git) -> App {
        let cwd = git.toplevel.clone();
        App {
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
            search_query: None,
            last_search: String::new(),
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
            cx_prefix: false,
            smerge_prefix: false,
            rebase_todo_cc: false,
            prefix_arg: None,
            reading_arg: false,
            transient_helpp: false,
            transient_help: None,
            suspended: Vec::new(),
            help_dirs: help::default_dirs(),
            blame_previous_chunk: None,
            margins: HashMap::new(),
            refs_counts: CommitCount::Nothing,
            message: String::new(),
            quit: false,
            view_text: None,
            view_scroll: 0,
            rebase_todo: None,
            resolve: None,
            cwd,
            diff_args: vec!["--stat".into(), "--no-ext-diff".into()],
            log_args: vec!["--graph".into(), "-n256".into(), "--decorate".into()],
            scripted: std::cell::RefCell::new(std::collections::VecDeque::new()),
            scripted_keys: std::cell::RefCell::new(std::collections::VecDeque::new()),
            instant_keys: std::cell::RefCell::new(Vec::new()),
            initial_input: std::cell::RefCell::new(None),
            search_read: std::cell::RefCell::new(None),
            var_scopes: HashMap::new(),
            invoking_menu: None,
            minibuffer_above: std::cell::RefCell::new(Vec::new()),
            user_init_file: None,
            minibuffer_history: std::cell::RefCell::new(Vec::new()),
            revision_history: std::cell::RefCell::new(Vec::new()),
            screen: std::cell::RefCell::new(Vec::new()),
            painted_size: (0, 0),
            repaint_all: true,
            initial_section: true,
            theme: Theme::default(),
            infix_history: std::cell::RefCell::new(BTreeMap::new()),
            history_file: None,
            options: Options::default(),
            prompts: std::cell::RefCell::new(Vec::new()),
            in_terminal: false,
            set_values: HashMap::new(),
            saved_values: BTreeMap::new(),
            values_file: None,
            history: HashMap::new(),
            menu_history: (Vec::new(), 0),
            jobs: jobs::Jobs::default(),
            head_label: String::new(),
        }
    }

    /// magit-status-initial-section '(1): the first section after the headers
    fn goto_initial_section(&mut self) {
        if let Some(i) = self.lines.iter().position(|l| l.heading && l.depth == 0 && l.kind != LineKind::Head) {
            self.cursor = i;
        }
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

    /// The top-level directory of the repository shown.
    pub fn toplevel(&self) -> &std::path::Path {
        &self.git.toplevel
    }

    /// The major mode of the buffer shown.
    pub fn mode(&self) -> &BufferMode {
        &self.mode
    }

    /// Whether xagit is about to exit (the last buffer was buried).
    pub fn quitting(&self) -> bool {
        self.quit
    }

    /// The header line of the buffer shown (empty for the status buffer).
    pub fn header_line(&self) -> &str {
        &self.buf.title
    }

    /// The refresh function of the buffer's mode, run on this thread
    /// (`refresh` is in jobs.rs).
    fn build_buffer(&self) -> StatusBuffer {
        self.buffer_spec().build()
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
        self.clear_search_state();
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
    /// Magit buffers of the repository but the status buffer, with one
    /// kill the buffer shown, else bury it (magit-mode-quit-window): the
    /// buffer below is shown and the buried one stays around (J, or
    /// setting up a buffer of its mode, shows it again). The status
    /// buffer stays: xagit quits with C-x C-c.
    fn mode_bury_buffer(&mut self) {
        if self.prefix_numeric() >= 16 {
            self.view_text = None;
            self.buried.clear();
            self.background.clear();
            if self.mode != BufferMode::Status {
                self.show_status_buffer();
            }
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
    /// buffer below (refreshed). The last buffer gives way to the status
    /// buffer, which is not killed (C-x C-c quits).
    fn kill_buffer(&mut self) {
        if self.view_text.take().is_some() {
            self.set_msg("");
            return;
        }
        self.clear_search_state();
        let Some(b) = self.buried.pop() else {
            if self.mode == BufferMode::Status {
                self.set_msg("Type C-x C-c to quit xagit");
            } else {
                self.show_status_buffer();
            }
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

    /// Show the status buffer in place of the buffer shown (killed).
    fn show_status_buffer(&mut self) {
        self.clear_search_state();
        self.mode = BufferMode::Status;
        self.buf_args.clear();
        self.buf = StatusBuffer::default();
        self.vis.clear();
        self.lines.clear();
        self.cursor = 0;
        self.scroll = 0;
        self.refresh();
    }

    /// save-buffers-kill-terminal (C-x C-c): quit xagit.
    fn kill_terminal(&mut self) {
        self.view_text = None;
        self.quit = true;
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
        Ok(self.pick_rev(prompt, &revs, None)?.cloned())
    }

    /// Read a commit, defaulting to the commit at point
    /// (magit-read-other-branch-or-commit).
    fn read_commit(&mut self, prompt: &str) -> Result<Option<String>> {
        match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Commit { hash, .. }) => {
                let short = self.git.rev_short(&hash);
                self.read_rev(prompt, Some(&short))
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

    // ------------------------------------------------------------ event loop

    pub fn run(&mut self) -> Result<()> {
        self.in_terminal = true;
        self.jobs.enabled = true;
        self.theme = Theme::detect();
        // a panic must not leave the terminal in raw mode on the
        // alternate screen; one on a worker (a refresh) is reported by
        // the worker and must not take the terminal away
        let main = std::thread::current().id();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().id() == main {
                restore_terminal();
                previous(info);
            }
        }));
        let _ = enable_raw_mode();
        let _ = execute!(stdout(), EnterAlternateScreen, crossterm::cursor::Hide);
        // the first build (see `main_run`), now on a worker
        self.flush_refresh();
        self.draw();
        let mut frame = self.jobs.frame();
        loop {
            if self.quit && self.quit_with_jobs() {
                break;
            }
            // wait for a key, but keep showing the work done meanwhile
            if !event::poll(self.jobs.tick())? {
                let done = self.poll_jobs();
                // redraw when work is done or the busy indicator turns
                if done || self.jobs.frame() != frame {
                    self.draw();
                    frame = self.jobs.frame();
                }
                continue;
            }
            match event::read()? {
                Event::Key(ev) => {
                    if ev.kind == crossterm::event::KeyEventKind::Release {
                        continue;
                    }
                    let key = Key::from_event(&ev);
                    // moving around is done in the buffer shown; any
                    // other command works on the refreshed one
                    // (before the first build there is nothing to move in)
                    let moves = self.only_moves(&key) && !self.lines.is_empty();
                    if !moves {
                        self.settle();
                    }
                    if let Err(e) = self.handle_key(key) {
                        self.set_msg(&format!("{e:#}"));
                    }
                    // other commands may have written to the terminal
                    // (the minibuffer, hooks of interactive git commands)
                    if !moves {
                        self.repaint_all = true;
                    }
                }
                // a new size: the frame is laid out again below
                Event::Resize(..) => {
                    let _ = execute!(stdout(), Clear(ClearType::All));
                    self.repaint_all = true;
                }
                _ => {}
            }
            self.poll_jobs();
            self.draw();
            frame = self.jobs.frame();
        }
        self.save_history();
        restore_terminal();
        Ok(())
    }

    fn handle_key(&mut self, key: Key) -> Result<()> {
        if self.universal_argument(&key) {
            return Ok(());
        }
        // the refreshes the command asks for are done once, after it
        self.jobs.depth += 1;
        let r = self.dispatch_key(key);
        self.jobs.depth -= 1;
        if self.jobs.depth == 0 {
            self.flush_refresh();
        }
        // magit-blame's post-command hook reads the chunk at point
        if matches!(self.mode, BufferMode::File { blame: Some(_), .. }) {
            self.settle();
        }
        self.blame_post_command();
        // the prefix argument is for one command; prefix keys (C-c, the
        // first keys of a transient's key sequence) pass it on
        if !self.cc_prefix && !self.cx_prefix && !self.smerge_prefix && self.pending.is_empty() {
            self.prefix_arg = None;
        }
        r
    }

    /// universal-argument (C-u), negative-argument (M--, C--, C-M--),
    /// digit-argument (M-0..M-9, C-0..C-9, C-M-0..C-M-9) and the digits,
    /// `-` and C-u of universal-argument-map. True if KEY was consumed.
    fn universal_argument(&mut self, key: &Key) -> bool {
        let arg = self.prefix_arg;
        let digit_arg = self.digit_argument_key(key);
        // negative-argument
        let negative = |arg: Option<PrefixArg>| match arg {
            Some(PrefixArg::Num(n)) => Some(PrefixArg::Num(-n)),
            Some(PrefixArg::Minus) => None,
            _ => Some(PrefixArg::Minus),
        };
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
            (false, "M--") | (true, "-") | (true, "M--") => negative(arg),
            _ if digit_arg.is_some() => {
                let digit = digit_arg.unwrap_or(0);
                match arg {
                    Some(PrefixArg::Num(n)) => Some(PrefixArg::Num(n * 10 + if n < 0 { -digit } else { digit })),
                    // -0 is just -, so that -01 works
                    Some(PrefixArg::Minus) if digit == 0 => Some(PrefixArg::Minus),
                    Some(PrefixArg::Minus) => Some(PrefixArg::Num(-digit)),
                    _ => Some(PrefixArg::Num(digit)),
                }
            }
            _ if !self.reading_arg => return false,
            (true, "g") => {
                self.reading_arg = false;
                self.prefix_arg = None;
                self.set_msg("Quit");
                return true;
            }
            // universal-argument-minus: after digits `-` is a command
            (false, "-") if !matches!(arg, Some(PrefixArg::Num(_))) => negative(arg),
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

    /// The digit of KEY if it is digit-argument: M-0..M-9 (but M-1..M-4
    /// are magit-section-show-level-N-all in Magit buffers), C-0..C-9,
    /// C-M-0..C-M-9, and the digits of universal-argument-map while the
    /// argument is read.
    fn digit_argument_key(&self, key: &Key) -> Option<i64> {
        let digit = |s: &str| match s.as_bytes() {
            [d] if d.is_ascii_digit() => Some(i64::from(d - b'0')),
            _ => None,
        };
        if let Some(d) = key.code.strip_prefix("M-").and_then(digit) {
            let section_level = !key.ctrl && (1..=4).contains(&d) && !matches!(self.mode, BufferMode::File { .. });
            return (!section_level).then_some(d);
        }
        let d = digit(&key.code)?;
        (key.ctrl || self.reading_arg).then_some(d)
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
        // transient-resume-mode: the help of a suspended transient
        if self.transient_help.is_some() {
            self.transient_help_key(&key);
            return Ok(());
        }
        // transient popup takes precedence
        if let Some((name, active)) = self.stack.last().cloned() {
            if let Some(menu) = self.menus.get(&name).cloned() {
                return self.handle_transient_key(key, menu, active);
            }
        }
        let r = self.dispatch_buffer_key(key);
        // transient-setup outside a transient replaces the suspended one
        if !self.stack.is_empty() {
            self.suspended.clear();
        }
        r
    }

    /// A key outside transients.
    fn dispatch_buffer_key(&mut self, key: Key) -> Result<()> {
        // git-rebase-mode-map
        if self.rebase_todo_key(&key)? {
            return Ok(());
        }
        // the built-in conflict resolution view
        if self.resolve_key(&key)? {
            return Ok(());
        }
        if std::mem::take(&mut self.smerge_prefix) {
            // magit-hunk-section-smerge-map
            let how = match (key.ctrl, key.code.as_str()) {
                (false, "enter") => Keep::Current,
                (false, "a") => Keep::All,
                (false, "u") => Keep::Upper,
                (false, "b") => Keep::Base,
                (false, "l") => Keep::Lower,
                _ => {
                    self.set_msg(&format!("C-c ^ {} is undefined", key.label()));
                    return Ok(());
                }
            };
            return self.call_smerge(how, false);
        }
        if std::mem::take(&mut self.cc_prefix) {
            let select = matches!(self.mode, BufferMode::LogSelect { .. });
            let blaming = matches!(self.mode, BufferMode::File { blame: Some(_), .. });
            let in_hunk = self.view_text.is_none()
                && matches!(self.current_line().map(|l| &l.kind), Some(LineKind::Hunk { .. } | LineKind::DiffLine { .. }));
            match (key.ctrl, key.code.as_str()) {
                (true, "n") if self.mode.is_log() => self.log_move_to_parent(),
                (true, "c") if select => return self.log_select_pick(),
                (true, "k") if select => self.kill_buffer(),
                // magit-blame-mode-map
                (true, "q") if blaming => self.blame_quit(),
                // smerge-command-prefix in magit-hunk-section-map
                (false, "^") if in_hunk => {
                    self.smerge_prefix = true;
                    self.set_msg("C-c ^-");
                }
                // the global binding of magit-file-dispatch
                (false, "M-g") => self.open_menu("file-dispatch"),
                _ => self.set_msg(&format!("C-c {} is undefined", key.label())),
            }
            return Ok(());
        }
        if std::mem::take(&mut self.cx_prefix) {
            match (key.ctrl, key.code.as_str()) {
                // save-buffers-kill-terminal
                (true, "c") => self.kill_terminal(),
                (true, "g") => self.set_msg("Quit"),
                _ => self.set_msg(&format!("C-x {} is undefined", key.label())),
            }
            return Ok(());
        }
        if key.ctrl && key.code == "x" {
            self.cx_prefix = true;
            self.set_msg("C-x-");
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
            && !binding.is_global()
        {
            self.set_msg(&format!("{} is undefined", key.label()));
            return Ok(());
        }
        // magit-stash-section-map remaps magit-cherry-pick ("A") to
        // magit-stash-pop on a stash section.  Lowercase "a" is already
        // remapped to stash-apply by apply_cmd.
        if self.view_text.is_none()
            && !key.ctrl
            && key.code == "A"
            && matches!(self.current_line().map(|l| &l.kind), Some(LineKind::Stash { .. }))
        {
            return self.run_command("stash-pop", &BTreeMap::new());
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
                self.clear_region();
                self.clear_search_state();
                self.set_msg("Quit");
            }
            Binding::IsearchForward => self.isearch(true)?,
            Binding::IsearchBackward => self.isearch(false)?,
            Binding::Visit => self.visit_thing()?,
            Binding::Menu(m) => self.open_menu(m),
            Binding::Command(c) => self.run_command(c, &BTreeMap::new())?,
        }
        Ok(())
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
            .status().map_err(|e| crate::git::exec::spawn_failure(dir, args, e))?;
        if !st.success() {
            anyhow::bail!("{} failed ({st})\nin {}\nOutput was shown in the terminal (interactive command).", crate::git::exec::command_label(args), crate::git::exec::sanitize(&dir.display().to_string()));
        }
        Ok(())
    }
}

/// Program entry helper: launch the TUI for cwd (or first arg path).
pub fn main_run(path: Option<&str>) -> Result<()> {
    let dir = std::path::PathBuf::from(path.unwrap_or("."));
    let abs = dir.canonicalize().unwrap_or(dir);
    let git = Git::discover(&abs)?;
    let mut app = App::unbuilt(git);
    if abs.is_dir() {
        app.cwd = abs;
    }
    // reading the options and the saved arguments asks for refreshes: the
    // status buffer is built once, after them, on a worker started by `run`
    app.jobs.depth += 1;
    if let Some(f) = super::options::default_options_file() {
        app.load_options(&f);
    }
    if let Some(f) = super::transient::default_values_file() {
        app.load_values(f);
    }
    if let Some(f) = super::transient::default_history_file() {
        app.load_history(f);
    }
    app.jobs.depth -= 1;
    app.jobs.wanted = true;
    app.run()
}

/// Whether REV is a stash (stash@{N}).
fn is_stash_ref(rev: &str) -> bool {
    rev.strip_prefix("stash@{")
        .and_then(|r| r.strip_suffix('}'))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The instant key answering any key with its code (read-event).
const ANY_KEY: &str = "<any>";

/// rmc--add-key-description on a terminal without underlining: the key
/// C in NAME upcased in brackets, else (or for a special key) its
/// description before NAME.
fn rmc_key_description(c: char, name: &str) -> String {
    let desc = match c {
        ' ' => "SPC".to_string(),
        '\t' => "TAB".into(),
        '\r' => "RET".into(),
        '\u{1b}' => "ESC".into(),
        '\u{7f}' => "DEL".into(),
        c => c.to_string(),
    };
    match name.char_indices().find(|(_, x)| *x == c) {
        Some((i, _)) if desc.chars().count() == 1 => {
            let rest = &name[i + c.len_utf8()..];
            format!("{}[{}]{rest}", &name[..i], c.to_uppercase())
        }
        _ => format!("{desc} {name}"),
    }
}

/// The help of disabled-command-function for CMD invoked by M-x: the
/// first paragraph of its documentation, indented.
fn disabled_command_help(cmd: &str) -> String {
    let doc = match cmd {
        "magit-clean" => {
            "Remove untracked files from the working tree.\n\
             With a prefix argument also remove ignored files,\n\
             with two prefix arguments remove ignored files only."
        }
        _ => "<< not documented >>",
    };
    let doc: Vec<String> = doc.lines().map(|l| if l.is_empty() { String::new() } else { format!("   {l}") }).collect();
    format!(
        "You have invoked the disabled command {cmd}.\n\
         It is disabled because new users often find it confusing.\n\
         Here's the first part of its description:\n\n{}\n\n\
         Do you want to use this command anyway?\n\n\
         You can now type:\n \
         n    (also C-g) to cancel--don't try the command; it remains disabled.\n \
         y    to enable the command (no questions if you use it again).\n \
         SPC  to try the command just this once, but leave it disabled.\n \
         !    to enable it and all the disabled commands for this session.",
        doc.join("\n")
    )
}

/// PROMPT followed by a space unless empty or ending with one (the
/// `padded` of y-or-n-p and yes-or-no-p).
fn padded(prompt: &str) -> String {
    if prompt.is_empty() || prompt.ends_with(' ') {
        prompt.to_string()
    } else {
        format!("{prompt} ")
    }
}

#[cfg(test)]
mod error_display_tests {
    use super::*;

    #[test]
    fn multiline_errors_open_complete_scrollable_details() {
        let td = tempfile::tempdir().unwrap();
        let git = Git { toplevel: td.path().into(), gitdir: td.path().join(".git") };
        let mut app = App::new(git);
        let text = format!("git push failed (exit status: 1)\n{}", "detail\n".repeat(100));
        app.set_msg(&text);
        assert!(!app.message().contains('\n'));
        assert!(app.message().contains("details"));
        assert_eq!(app.text_view().unwrap().1, text);
        assert_eq!(app.text_view_scroll(), 0);
        app.handle_key(super::super::keys::key_from_label("n")).unwrap();
        assert_eq!(app.text_view_scroll(), 1);
        app.handle_key(super::super::keys::key_from_label("q")).unwrap();
        assert!(app.text_view().is_none());
    }

    #[test]
    fn checkout_failure_opens_git_diagnostics_not_only_exit_code() {
        let td = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new("git").args(["init", "-q"]).current_dir(td.path()).status().unwrap().success());
        let git = Git::discover(td.path()).unwrap();
        let error = git.checkout("ui-missing-branch").unwrap_err();
        let mut app = App::new(git);
        app.done("checked out", Err(error));
        let details = &app.text_view().unwrap().1;
        assert!((details.contains("pathspec") || details.contains("路径规格")) && details.contains("ui-missing-branch"));
        assert!(details.contains(&td.path().display().to_string()));
        assert!(app.message().contains("details"));
    }

    #[test]
    fn long_line_and_credentials_are_handled() {
        let td = tempfile::tempdir().unwrap();
        let mut app = App::new(Git { toplevel: td.path().into(), gitdir: td.path().join(".git") });
        app.set_msg(&format!("failure https://user:fakepass@example.com/r {}", "x".repeat(300)));
        assert!(!app.message().contains("fakepass"));
        assert!(!app.text_view().unwrap().1.contains("fakepass"));
        app.set_msg("short notification");
        assert_eq!(app.message(), "short notification");
    }
}
