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
mod render;
mod input;
mod sections;
mod menus;
mod apply;

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
            cwd,
            diff_args: vec!["--stat".into(), "--no-ext-diff".into()],
            log_args: vec!["--graph".into(), "-n256".into(), "--decorate".into()],
            scripted: std::cell::RefCell::new(std::collections::VecDeque::new()),
            scripted_keys: std::cell::RefCell::new(std::collections::VecDeque::new()),
            instant_keys: std::cell::RefCell::new(Vec::new()),
            initial_input: std::cell::RefCell::new(None),
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

    // ------------------------------------------------------------ commands

    /// Read the value of the option ARG (transient-infix-read). None: the
    /// user quit (nothing changes); Some(None): unset; Some(Some(v)): set
    /// to ARG followed by V.
    fn read_option(&mut self, arg: &str, reader: &Reader) -> Result<Option<Option<String>>> {
        let value = self.read_option_value(arg, reader)?;
        // transient-infix-read: the value read becomes the newest of the
        // infix's history
        // magit-tag:--local-user has the :history-key magit:--gpg-sign
        let key = if *reader == Reader::GpgKey { "--gpg-sign=" } else { arg };
        if let Some(Some(v)) = &value {
            add_to_history(self.infix_history.borrow_mut().entry(key.to_string()).or_default(), v, true);
        }
        Ok(value)
    }

    fn read_option_value(&mut self, arg: &str, reader: &Reader) -> Result<Option<Option<String>>> {
        let value = match reader {
            Reader::Choices(choices) => {
                let prompt = format!("{arg} [unspecified, {}]", choices.join(", "));
                let Some(v) = self.read_infix(&prompt, Some("unspecified"), arg)?.filter(|v| !v.is_empty()) else {
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
            Reader::GpgKey => {
                let Some(v) = self.read_gpg_signing_key(arg)? else {
                    return Ok(None);
                };
                // only --gpg-sign has :allow-empty
                if v.is_empty() && arg != "--gpg-sign=" {
                    return Ok(None);
                }
                v
            }
            Reader::Pick(choices) => {
                let table = Completion { candidates: choices.clone(), require_match: true, multiple: false };
                let read = self.completing_read(arg, None, Hist::Infix(arg), Some(table))?;
                let Some(v) = read.filter(|v| !v.is_empty()) else {
                    return Ok(None);
                };
                v
            }
            // magit-read-files of magit-modified-files, the file at point
            // the initial input
            Reader::Files | Reader::TrackedFiles => {
                let candidates = if *reader == Reader::Files {
                    self.git.git_lines(&["diff-index", "-z", "--name-only", "--submodule=short", "HEAD", "--"])
                } else {
                    self.git.git_lines(&["ls-files", "-z", "--full-name"])
                };
                *self.initial_input.borrow_mut() = self.current_file_name();
                let table = Completion { candidates, require_match: false, multiple: true };
                let Some(input) = self.read_line_hist("Limit to file,s: ", None, Hist::Infix(arg), Some(table))? else {
                    return Ok(None);
                };
                let files: Vec<&str> = input.split(',').map(str::trim).filter(|f| !f.is_empty()).collect();
                if files.is_empty() {
                    return Ok(None);
                }
                files.join(" ")
            }
            Reader::Revision => {
                let mut candidates = vec!["HEAD".to_string()];
                candidates.extend(self.git.refnames());
                let default = self.branch_or_commit_at_point().or_else(|| self.git.current_branch());
                let table = Completion { candidates, ..Default::default() };
                let Some(v) = self.read_any(arg, default.as_deref(), Hist::Infix(arg), table)? else {
                    return Ok(None);
                };
                v
            }
            Reader::NotesRef => {
                let Some(v) = self.notes_read_ref(arg, arg)? else {
                    return Ok(None);
                };
                v
            }
            Reader::CharCase(prompt, clauses) => {
                let labels: Vec<&str> = clauses.iter().map(|(_, l, _)| l.as_str()).collect();
                let prompt = format!("{prompt}{}, or [C-g] to abort ", labels.join(", "));
                let keys: Vec<char> = clauses.iter().map(|(k, _, _)| *k).collect();
                let Some(c) = self.read_char_choice(&prompt, &keys)? else {
                    return Ok(None);
                };
                match clauses.iter().find(|(k, _, _)| *k == c) {
                    Some((_, _, v)) => v.clone(),
                    None => return Ok(None),
                }
            }
            Reader::Person | Reader::PatchBase | Reader::RangeOrCommit | Reader::ExistingDirectory => {
                let Some(v) = self.read_patch_infix(reader, arg)? else {
                    return Ok(None);
                };
                v
            }
            Reader::SubtreePrefix => {
                let Some(v) = self.subtree_read_prefix(arg)? else {
                    return Ok(None);
                };
                v
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
                let read = self.read_infix(arg, None, arg)?;
                let read = if *reader == Reader::OptionalNumber { read } else { read.filter(|v| !v.is_empty()) };
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

    #[allow(clippy::too_many_lines)]
    fn run_command(&mut self, cmd: &str, active: &BTreeMap<String, bool>) -> Result<()> {
        // magit-branch-configure / magit-remote-configure: the scope the
        // current branch or remote, unless with a prefix argument or
        // invoked from magit-branch / magit-remote, then the one read
        if cmd == "branch-config-menu" || cmd == "remote-configure-menu" {
            let branch = cmd == "branch-config-menu";
            let from = if branch { "branch-menu" } else { "remote-menu" };
            let current = if branch { self.git.current_branch() } else { self.current_remote() };
            let direct = self.prefix_arg.is_none() && self.invoking_menu.as_deref() != Some(from);
            let scope = match current.filter(|_| direct) {
                Some(s) => s,
                None => {
                    let read = if branch {
                        self.read_local_branch("Configure branch")?
                    } else {
                        self.read_remote("Configure remote")?
                    };
                    let Some(s) = read else {
                        return Ok(());
                    };
                    s
                }
            };
            self.var_scopes.insert(cmd.to_string(), Some(scope));
            self.open_menu(cmd);
            return Ok(());
        }
        // the git variable infixes (from M-x: the scope read)
        if let Some(v) = gitvar::lookup(cmd) {
            let menu = self.invoking_menu.clone();
            return self.git_var_command(v, menu.as_deref());
        }
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
            "blame-echo" => self.blame_echo(args)?,
            "blame-quit" => self.blame_quit(),
            // magit-file-dispatch
            "file-stage" | "file-unstage" | "file-untrack" | "file-rename" | "file-delete" | "file-checkout"
            | "diff-buffer-file" | "log-buffer-file" | "log-trace-definition" | "blob-previous" | "blob-next"
            | "find-file" | "blob-visit-file" | "status-here" | "edit-line-commit" => self.file_command(cmd)?,
            "stage-files" => self.stage_files()?,
            "unstage-files" => self.unstage_files()?,
            "display-repository-buffer" | "display-buffer" => self.display_repository_buffer()?,
            "execute-extended-command" => self.execute_extended_command()?,
            "quit" => self.mode_bury_buffer(),
            "toggle-section" => self.toggle_section(),
            "visit" => self.visit_thing()?,
            "jump" => self.jump_command(),
            "jump-to-stashes" | "jump-to-untracked" | "jump-to-unstaged" | "jump-to-staged" | "jump-to-unpulled-from-upstream"
            | "jump-to-unpulled-from-pushremote" | "jump-to-unpushed-to-upstream" | "jump-to-unpushed-to-pushremote" => {
                self.section_jumper(cmd)
            }
            "imenu" => self.imenu()?,
            // magit-status-quick: the status buffer (shown again)
            "status-quick" => self.switch_to_mode(BufferMode::Status),
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
            // `K` is magit-file-untrack: the full command (region,
            // confirmation, "Already untracked" and prefix --force) is
            // implemented by file-untrack, also bound in file-dispatch.
            "untrack" => self.file_command("file-untrack")?,
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
            // magit-checkout: a local branch becomes the current branch,
            // anything else (a remote branch, a tag, a commit) detaches HEAD
            "checkout" => {
                let Some(commit) = self.read_other_branch_or_commit("Checkout")? else {
                    return Ok(());
                };
                self.checkout_rev(&commit, &args);
            }
            "branch-checkout" => self.branch_checkout(&args)?,
            // magit-branch-and-checkout
            "branch-and-checkout" => {
                let Some((name, start)) = self.branch_read_args("Create and checkout branch")? else {
                    return Ok(());
                };
                let res = if is_stash_ref(&start) {
                    g!().git_interactive(&["stash", "branch", &name, &start])
                } else {
                    let mut a: Vec<&str> = vec!["checkout"];
                    a.extend(args.iter().map(String::as_str));
                    a.extend(["-b", &name, &start]);
                    let res = g!().git_interactive(&a);
                    self.branch_maybe_adjust_upstream(&name, &start);
                    res
                };
                self.done("", res.map(|_| String::new()));
            }
            // magit-branch-create
            "branch-create" => {
                let Some((name, start)) = self.branch_read_args("Create branch")? else {
                    return Ok(());
                };
                let res = g!().git_interactive(&["branch", &name, &start]);
                self.branch_maybe_adjust_upstream(&name, &start);
                self.done("", res.map(|_| String::new()));
            }
            // magit-branch-spinoff / -spinout: FROM the last (oldest)
            // commit selected
            // magit-branch-orphan
            "branch-orphan" => {
                let Some((name, start)) = self.branch_read_args("Create and checkout orphan branch")? else {
                    return Ok(());
                };
                let res = g!().git_interactive(&["checkout", "--orphan", &name, &start]);
                self.done_keep_msg(res);
            }
            "branch-shelve" => self.branch_shelve()?,
            "branch-unshelve" => self.branch_unshelve()?,
            "delete-shelved-branch" => {
                if let Some(b) = self.read_shelved_branch("Delete shelved branch")? {
                    let res = g!().git_interactive(&["update-ref", "-d", &format!("refs/shelved/{b}")]);
                    self.done_keep_msg(res);
                }
            }
            "branch-spinoff" | "branch-spinout" => {
                let checkout = cmd == "branch-spinoff";
                let from = self.region_commit_values().and_then(|v| v.last().cloned());
                let prompt = if checkout { "Spin off branch" } else { "Spin out branch" };
                let Some(name) = self.read_string_ns(prompt)? else {
                    return Ok(());
                };
                self.branch_spinoff(&name, from.as_deref(), checkout);
            }
            // magit-branch-rename
            "branch-rename" => {
                let bs = g!().branches();
                // magit-read-local-branch
                let default = self.local_branch_at_point().or_else(|| g!().current_branch());
                let Some(old) = self.pick_rev("Rename branch", &bs, default.as_deref())?.cloned() else {
                    return Ok(());
                };
                let Some(new) = self.read_line_prompt(&format!("Rename branch '{old}' to"), None)? else {
                    return Ok(());
                };
                self.branch_rename(&old, &new)?;
            }
            "branch-reset" => self.branch_reset()?,
            "branch-delete" => self.branch_delete()?,

            // -------- remote menu
            "remote-add" | "remote-rename" | "remote-remove" | "remote-prune" | "remote-prune-refspecs"
            | "remote-unshallow" => self.remote_command(cmd, &args)?,

            // -------- fetch/pull/push (upstream = branch.<b>.remote / branch.<b>.merge)
            "fetch-from-pushremote" | "fetch-from-upstream" | "fetch-other" | "fetch-branch" | "fetch-refspec"
            | "fetch-all" | "fetch-modules" | "fetch-modules-run" | "pull-from-pushremote" | "pull-from-upstream"
            | "pull-into-upstream" | "pull-branch" => self.fetch_pull_command(cmd, &args)?,
            "push-current-to-pushremote" | "push-current-to-upstream" | "push-current" | "push-other" | "push-refspecs"
            | "push-matching" | "push-tags" | "push-tag" | "push-notes-ref" => self.push_command(cmd, &args)?,

            // -------- merge / rebase
            "merge-plain" | "merge-nocommit" | "merge-squash" | "merge-editmsg" | "merge-preview" => {
                let mut bs = g!().branches();
                bs.extend(g!().remote_branches());
                let Some(rev) = self.pick_rev("Merge revision", &bs, None)?.cloned() else {
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
                            Err(e) => self.report_error(&e),
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
                    Err(e) => self.report_error(&e),
                }
                self.refresh();
            }
            // magit-merge-abort
            "merge-abort" => {
                if !g!().merge_in_progress() {
                    self.set_msg("No merge in progress");
                    return Ok(());
                }
                if !self.confirm("abort-merge", "Abort merge")? {
                    return Ok(());
                }
                g!().merge_abort()?;
                self.set_msg("merge aborted");
                self.refresh();
            }
            // magit-rebase-onto-pushremote
            "rebase-onto-pushremote" => {
                let Some((branch, remote, _)) = self.select_push_remote("rebase onto that")? else {
                    return Ok(());
                };
                let res = self.git.rebase(&format!("{remote}/{branch}"), &args);
                self.after_rebase(res);
            }
            // magit-rebase-onto-upstream
            "rebase-onto-upstream" => {
                let Some(branch) = self.git.current_branch() else {
                    self.set_msg("No branch is checked out");
                    return Ok(());
                };
                let mut upstream = self.upstream_branch(&branch);
                if self.prefix_arg.is_some() || upstream.is_none() {
                    let prompt = format!("Set upstream of {branch} and rebase onto that");
                    let Some(up) = self.read_upstream_branch(&branch, &prompt)? else {
                        return Ok(());
                    };
                    self.git.git_interactive(&["branch", "--set-upstream-to", &up, &branch])?;
                    upstream = Some(up);
                }
                let res = self.git.rebase(&upstream.unwrap_or_default(), &args);
                self.after_rebase(res);
            }
            // magit-rebase-branch
            "rebase-branch" => {
                let Some(target) = self.read_other_branch_or_commit("Rebase onto")? else {
                    return Ok(());
                };
                let res = self.git.rebase(&target, &args);
                self.after_rebase(res);
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
            "commit-reshelve" => self.commit_reshelve(&args)?,
            "commit-autofixup" | "commit-autofixup-run" | "commit-absorb-modules" => self.spread_command(cmd, &args)?,
            // magit-reshelve-since: the key of --gpg-sign= of the rebase
            // arguments
            "reshelve-since" => {
                let keyid = args.iter().find_map(|a| a.strip_prefix("--gpg-sign=")).map(str::to_string);
                self.reshelve_since_select(keyid)?;
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
            // magit-rebase-abort
            "rebase-abort" => {
                if !self.confirm("abort-rebase", "Abort this rebase")? {
                    return Ok(());
                }
                g!().rebase_abort()?;
                self.set_msg("rebase aborted");
                self.refresh();
            }

            // -------- reset
            "reset-mixed" | "reset-soft" | "reset-hard" | "reset-keep" | "reset-index"
            | "reset-worktree" | "reset-quickly" => {
                // magit-reset-quickly: hard with a prefix argument
                let hard = cmd == "reset-hard" || (cmd == "reset-quickly" && self.prefix_arg.is_some());
                let current = self.git.current_branch().unwrap_or_else(|| "detached head".into());
                let prompt = match cmd {
                    "reset-index" => "Reset index to".to_string(),
                    "reset-worktree" => "Reset worktree to".to_string(),
                    "reset-soft" => format!("Soft reset {current} to"),
                    _ if hard => format!("Hard reset {current} to"),
                    _ => format!("Reset {current} to"),
                };
                let Some(rev) = self.read_branch_or_commit(&prompt)? else {
                    return Ok(());
                };
                if cmd == "reset-worktree" {
                    self.wip_hook(&[], "before", "reset")?;
                    let res = self.git.reset_worktree(&rev);
                    if res.is_ok() {
                        self.wip_hook(&[], "after", "reset")?;
                    }
                    self.done("", res.map(|_| String::new()));
                    return Ok(());
                }
                let arg = match cmd {
                    "reset-index" => None,
                    "reset-soft" => Some("--soft"),
                    "reset-keep" => Some("--keep"),
                    _ if hard => Some("--hard"),
                    _ => Some("--mixed"),
                };
                self.reset_internal(arg, &rev, (cmd == "reset-index").then_some("."))?;
            }

            // -------- tag
            "tag-create" | "tag-release" | "tag-prune" => self.tag_command(cmd, &args)?,
            // magit-tag-delete: the selected tags, else one read
            "tag-delete" => {
                let tags = match self.region_refs(&[RefKind::Tag], false) {
                    Some(tags) => {
                        if !self.confirm_items("t", "Delete %s", "Delete %d tags", &tags)? {
                            return Ok(());
                        }
                        tags
                    }
                    None => {
                        let tags = g!().tags();
                        let default = self.tag_at_point();
                        match self.pick_rev("Delete tag", &tags, default.as_deref())? {
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
                        let Some(r) = self.read_rev(prompt, default.as_deref())? else {
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
                // magit-sequencer-abort
                if action == "abort" {
                    let (a, p) = match g!().sequencer_command() {
                        Ok("revert") => ("abort-revert", "Really abort revert"),
                        Ok(_) => ("abort-cherry-pick", "Really abort cherry-pick"),
                        Err(_) => {
                            self.set_msg("No cherry-pick or revert in progress");
                            return Ok(());
                        }
                    };
                    if !self.confirm(a, p)? {
                        return Ok(());
                    }
                }
                match g!().sequencer(action) {
                    Ok(()) => self.set_msg(&format!("sequencer {action}")),
                    Err(e) => self.report_error(&e),
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
                        let Some(r) = self.read_rev(prompt, default.as_deref())? else {
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
            "stash-both" | "stash-index" | "stash-worktree" | "stash-keep-index" | "snapshot-both" | "snapshot-index"
            | "snapshot-worktree" | "stash-push" | "stash-apply" | "stash-pop" | "stash-branch" | "stash-branch-here"
            | "stash-format-patch" => self.stash_command(cmd, &args)?,
            "stash-show" => self.diff_view("stash")?,
            "stash-drop" => self.stash_drop()?,
            "stash-list" => self.setup_buffer(BufferMode::Stashes { refname: "refs/stash".into() }, Vec::new()),

            // -------- ignore / misc
            "ignore" | "ignore-subdir" | "ignore-gitdir" | "ignore-system" | "skip-worktree" | "no-skip-worktree"
            | "assume-unchanged" | "no-assume-unchanged" => self.gitignore_command(cmd)?,
            // magit-run (!), magit-ediff (E) and magit-git-mergetool
            "git-command" | "git-command-topdir" | "shell-command" | "shell-command-topdir" | "run-gitk"
            | "run-gitk-all" | "run-gitk-branches" | "run-git-gui" | "git-mergetool" | "git-mergetool-invoke"
            | "ediff-mergetool" | "ediff-resolve-rest" | "ediff-resolve-all" | "ediff-stage" => {
                self.run_menu_command(cmd, &args)?
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
                if let Some(r) = self.pick_rev("Compare with", &choices, default.as_deref())?.cloned() {
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
            | "log-all" | "log-change" | "wip-log-index" | "wip-log-worktree" | "wip-log" | "wip-log-current" => {
                self.log_view(cmd)?
            }
            // magit-wip-commit: with a prefix argument the message is read
            "wip-commit" => {
                let msg = if self.prefix_arg.is_some() {
                    match self.read_line_prompt("Wip commit message", None)? {
                        Some(m) => m,
                        None => return Ok(()),
                    }
                } else {
                    "wip-save tracked files".to_string()
                };
                match self.options.wip.commit(&self.git, &[], Some(&msg)) {
                    Ok(m) => {
                        self.set_msg(m.as_deref().unwrap_or(""));
                        self.refresh();
                    }
                    Err(e) => self.set_msg(&e.to_string()),
                }
            }
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
                        let Some(r) = self.read_rev("Show reflog for", default.as_deref())? else {
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
                    let Some(c) = self.read_rev("Shortlog since", default.as_deref())? else {
                        return Ok(());
                    };
                    format!("{c}..")
                } else {
                    let default = match self.dwim() {
                        Dwim::Commit(c) => Some(self.git.rev_short(&c)),
                        Dwim::Range(r) => Some(r),
                        _ => g!().current_branch(),
                    };
                    let Some(r) = self.read_rev("Shortlog for revision or range", default.as_deref())? else {
                        return Ok(());
                    };
                    r
                };
                let mut a: Vec<&str> = vec!["shortlog"];
                a.extend(args.iter().map(String::as_str));
                a.push(&rev);
                match g!().git_string(&a) {
                    Ok(o) => self.view_text = Some(("*magit-shortlog*".into(), o)),
                    Err(e) => self.report_error(&e),
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
                let Some(rev) = self.read_rev(prompt, default.as_deref())? else {
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
                let Some(r) = self.read_rev("Diff for range", default.as_deref())? else {
                    return Ok(());
                };
                self.show_diff(Some(r), None, "committed");
            }
            "dwim" => self.diff_dwim()?,
            "commit" => {
                // magit-thing-at-point 'git-revision, else
                // magit-branch-or-commit-at-point
                let atpoint = match self.current_line().map(|l| l.kind.clone()) {
                    Some(LineKind::Commit { hash, .. }) => Some(hash),
                    Some(LineKind::Ref { name, .. }) => Some(name),
                    _ => match &self.mode {
                        BufferMode::Revision { rev, .. } => Some(rev.clone()),
                        _ => None,
                    },
                };
                // with a prefix argument always read the commit
                let rev = match atpoint {
                    Some(r) if self.prefix_arg.is_none() => Some(r),
                    Some(r) => {
                        let full = r.len() >= 40 && r.chars().all(|c| c.is_ascii_hexdigit());
                        let default = if full { self.git.rev_short(&r) } else { r };
                        self.read_rev("Show commit", Some(&default))?
                    }
                    None => {
                        let current = self.git.current_branch();
                        self.read_rev("Show commit", current.as_deref())?
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
                    _ => self.read_stash("Show stash")?,
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
            BufferMode::File { .. } | BufferMode::RebaseTodo => Dwim::None,
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
                self.clear_region();
                self.show_diff(Some(format!("{v}^..{v}")), None, "committed")
            }
            Dwim::Range(r) => {
                self.clear_region();
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
                let Some(r) = self.read_rev("Log rev,s", default.as_deref())? else {
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
            // magit-wip-log-index / -worktree
            "wip-log-index" => vec![self.options.wip.wip_ref(&self.git, "index/", None)],
            "wip-log-worktree" => vec![self.options.wip.wip_ref(&self.git, "wtree/", None)],
            // magit-wip-log / -current: the branch and its wip refs, with
            // the tips of as many previous "branches" of them as the
            // prefix argument (negative: just the worktree wip ref)
            "wip-log" | "wip-log-current" => {
                let branch = if cmd == "wip-log" {
                    let mut choices = self.git.branches();
                    choices.push("HEAD".into());
                    let default = self.branch_at_point().or(current).unwrap_or_else(|| "HEAD".into());
                    let table = Completion { candidates: choices, require_match: true, multiple: false };
                    let Some(b) = self
                        .completing_read("Log branch and its wip refs", Some(&default), Hist::Revision, Some(table))?
                        .filter(|b| !b.is_empty())
                    else {
                        return Ok(());
                    };
                    b
                } else {
                    current.unwrap_or_else(|| "HEAD".into())
                };
                let count = self.prefix_numeric();
                let n = count.unsigned_abs() as usize;
                let wip = &self.options.wip;
                let mut v = vec![branch.clone()];
                v.extend(crate::git::wip::Wip::log_tips(&self.git, &wip.wip_ref(&self.git, "wtree/", Some(&branch)), n));
                if count >= 0 {
                    v.extend(crate::git::wip::Wip::log_tips(&self.git, &wip.wip_ref(&self.git, "index/", Some(&branch)), n));
                }
                v
            }
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
        self.settle();
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

    /// magit-region-values 'commit: the commits of the selected sibling
    /// sections (one if the region is within a commit's heading), top to
    /// bottom, abbreviated.
    fn region_commit_values(&self) -> Option<Vec<String>> {
        let secs = self.region_sections(false)?;
        secs.iter()
            .map(|&s| match &self.buf.all[self.sec(s).start].kind {
                LineKind::Commit { hash, .. } => Some(self.git.rev_short(hash)),
                _ => None,
            })
            .collect()
    }

    /// magit-read-string-ns: a string read with minibuffer-history,
    /// trimmed; "Need non-empty input" and "Input contains whitespace"
    /// refuse it.
    fn read_string_ns(&mut self, prompt: &str) -> Result<Option<String>> {
        self.read_string_ns_default(prompt, None)
    }

    /// magit-read-string-ns with DEFAULT (the value of an empty input).
    fn read_string_ns_default(&mut self, prompt: &str, default: Option<&str>) -> Result<Option<String>> {
        let Some(v) = self.read_line_raw(prompt, default)? else {
            return Ok(None);
        };
        let v = v.trim_matches([' ', '\t', '\n', '\r']);
        if v.is_empty() {
            self.set_msg("Need non-empty input");
            return Ok(None);
        }
        if v.contains([' ', '\t', '\n']) {
            self.set_msg("Input contains whitespace");
            return Ok(None);
        }
        Ok(Some(v.to_string()))
    }

    /// magit--branch-spinoff: create BRANCH at the current branch (and
    /// with CHECKOUT, or uncommitted changes, check it out), its upstream
    /// the current branch's indirect upstream if opted in; the current
    /// branch is reset to where it meets its upstream (FROM^ with FROM),
    /// with update-ref when checking out, else with a hard reset.
    fn branch_spinoff(&mut self, branch: &str, from: Option<&str>, mut checkout: bool) {
        if self.is_branch(branch) {
            self.set_msg(&format!("Cannot spin off {branch}.  It already exists"));
            return;
        }
        if !checkout && (!self.git.staged_files().is_empty() || !self.git.unstaged_files().is_empty()) {
            self.set_msg("Staying on HEAD due to uncommitted changes");
            checkout = true;
        }
        let Some(current) = self.git.current_branch() else {
            let res = if checkout {
                self.git.git_interactive(&["checkout", "-b", branch])
            } else {
                self.git.git_interactive(&["branch", branch])
            };
            self.done_keep_msg(res);
            return;
        };
        let tracked = self.upstream_branch(&current);
        if let Some(from) = from {
            if !self.git.is_ancestor(from, &current) {
                self.set_msg(&format!("Cannot spin off {branch}.  {from} is not reachable from {current}"));
                return;
            }
            if let Some(t) = tracked.as_ref().filter(|t| self.git.is_ancestor(from, t)) {
                self.set_msg(&format!("Cannot spin off {branch}.  {from} is ancestor of upstream {t}"));
                return;
            }
        }
        let res = if checkout {
            self.git.git_interactive(&["checkout", "-b", branch, &current])
        } else {
            self.git.git_interactive(&["branch", branch, &current])
        };
        if let Err(e) = res {
            self.refresh();
            self.set_msg(&format!("{e:#}"));
            return;
        }
        if let Some(u) = self.indirect_upstream_branch(&current, false) {
            let _ = self.git.git_interactive(&["branch", "--set-upstream-to", &u, branch]);
        }
        let base = tracked.and_then(|t| match from {
            Some(f) => Some(format!("{f}^")),
            None => self.git.git_string(&["merge-base", &current, &t]).ok().map(|b| b.trim().to_string()),
        });
        let rev = |r: &str| self.git.git_string(&["rev-parse", "--verify", "-q", &format!("{r}^{{commit}}")]).ok();
        if let Some(base) = base.filter(|b| !b.is_empty() && rev(b) != rev(&current)) {
            let res = if checkout {
                let msg = format!("reset: moving to {base}");
                self.git.git_interactive(&["update-ref", "-m", &msg, &format!("refs/heads/{current}"), &base])
            } else {
                self.git.git_interactive(&["reset", "--hard", &base])
            };
            if let Err(e) = res {
                self.set_msg(&format!("{e:#}"));
            }
        }
        self.refresh();
    }

    /// Refresh after RES, its error the message (else the message kept).
    fn done_keep_msg(&mut self, res: Result<()>) {
        self.refresh();
        if let Err(e) = res {
            self.set_msg(&format!("{e:#}"));
        }
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
            Err(e) => self.report_error(&e),
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
        self.settle();
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
            SelectAction::ReshelveSince { keyid } => self.reshelve_since(&commit, keyid.as_deref()),
            SelectAction::Autofixup { args } => self.autofixup(&commit, &args),
            SelectAction::AbsorbModules => self.absorb_modules(&commit),
        }
    }

    /// magit-rebase-interactive-1 for the interactive rebase CMD of COMMIT
    /// and the commits above it (magit-rebase-remove-commit passes NOASSERT).
    fn rebase_commit(&mut self, cmd: &str, commit: &str, args: &[String]) -> Result<()> {
        if cmd != "rebase-remove-commit" && !self.rebase_assert(commit, cmd != "rebase-reword-commit", args)? {
            return Ok(());
        }
        let res = match cmd {
            // the todo is edited in xagit's git-rebase-mode buffer
            "rebase-interactive" => return self.rebase_todo_start(commit, args),
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
            self.set_msg(&format!("{e:#}"));
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
        let published = self.publishing_branches(commit);
        if published.is_empty() {
            return Ok(true);
        }
        let (m1, m2) = ("This commit has already been published to ", ".\nDo you really want to modify it");
        self.confirm_items("amend-published", &format!("{m1}%s{m2}"), &format!("{m1}%d public branches{m2}"), &published)
    }

    /// magit-list-publishing-branches: the members of
    /// magit-published-branches COMMIT is an ancestor of.
    fn publishing_branches(&self, commit: &str) -> Vec<String> {
        self.options
            .published_branches
            .iter()
            .filter(|b| self.git.rev_verify(b) && self.git.is_ancestor(commit, b))
            .cloned()
            .collect()
    }

    /// `j` in log buffers (magit-log-move-to-revision).
    fn log_move_to_revision(&mut self) -> Result<()> {
        let default = self.git.current_branch();
        let Some(rev) = self.read_rev("In log, jump to", default.as_deref())? else {
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
        let Some(head) = self.pick_rev("Cherry head", &branches, current.as_deref())?.cloned() else {
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
        let Some(upstream) = self.pick_rev("Cherry upstream", &others, default.as_deref())?.cloned() else {
            return Ok(());
        };
        self.setup_buffer(BufferMode::Cherry { head, upstream }, Vec::new());
        Ok(())
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

    /// magit-confirm: ask PROMPT (with "?") unless ACTION is in
    /// magit-no-confirm ("t": always ask); declining shows "Abort".
    fn confirm(&mut self, action: &str, prompt: &str) -> Result<bool> {
        let ok = self.confirm_noabort(action, prompt)?;
        if !ok {
            self.set_msg("Abort");
        }
        Ok(ok)
    }

    /// magit-confirm with NOABORT: declining isn't an error.
    fn confirm_noabort(&mut self, action: &str, prompt: &str) -> Result<bool> {
        if self.options.no_confirm(action) {
            return Ok(true);
        }
        self.magit_y_or_n_p(&format!("{prompt}?"), Some(action))
    }

    /// magit-read-stash: one of the stashes ("stash@{N} subject",
    /// require-match), the one at point the default, else the first.
    fn read_stash(&mut self, prompt: &str) -> Result<Option<String>> {
        let choices: Vec<String> = self.git.stashes().into_iter().map(|(n, m)| format!("{n} {m}")).collect();
        if choices.is_empty() {
            self.set_msg("No stashes");
            return Ok(None);
        }
        let atpoint = match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Stash { name, .. }) => Some(name),
            _ => None,
        };
        let default = atpoint
            .and_then(|a| choices.iter().find(|c| c.split(' ').next() == Some(a.as_str())).cloned())
            .unwrap_or_else(|| choices[0].clone());
        let table = Completion { candidates: choices.clone(), require_match: true, multiple: false };
        let Some(v) = self.completing_read(prompt, Some(&default), Hist::Minibuffer, Some(table))?.filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        // a choice, or just the name of one
        let name = choices.iter().filter_map(|c| c.split(' ').next()).find(|n| v == *n || v.starts_with(&format!("{n} ")));
        if name.is_none() {
            self.set_msg("[No match]");
        }
        Ok(name.map(str::to_string))
    }

    /// magit-stash-drop: the stashes selected by the region (confirmed,
    /// drop-stashes), else one read; dropped from the last.
    fn stash_drop(&mut self) -> Result<()> {
        let region: Option<Vec<String>> = self.region_sections(false).and_then(|secs| {
            secs.iter()
                .map(|&s| match &self.buf.all[self.sec(s).start].kind {
                    LineKind::Stash { name, .. } => Some(name.clone()),
                    _ => None,
                })
                .collect()
        });
        let stashes = match region {
            Some(v) => {
                if !self.confirm_items("drop-stashes", "Drop stashes", "Drop %d stashes", &v)? {
                    return Ok(());
                }
                v
            }
            None => match self.read_stash("Drop stash")? {
                Some(s) => vec![s],
                None => return Ok(()),
            },
        };
        self.clear_region();
        let mut msg = String::new();
        for s in stashes.iter().rev() {
            msg = format!("Deleted refs/{s} (was {})", self.git.rev_short(s));
            if let Err(e) = self.git.stash_drop(s) {
                msg = format!("{e:#}");
                break;
            }
        }
        self.refresh();
        self.set_msg(&msg);
        Ok(())
    }

    /// magit-confirm with ITEMS: ONE with `%s` the item, else the items
    /// and MANY with `%d` their count; "Abort" when declined.
    fn confirm_items(&mut self, action: &str, one: &str, many: &str, items: &[String]) -> Result<bool> {
        match items {
            [] => Ok(true),
            [i] => self.confirm(action, &one.replacen("%s", i, 1)),
            is => self.confirm(action, &format!("{}\n\n{}", is.join("\n"), many.replacen("%d", &is.len().to_string(), 1))),
        }
    }

    /// magit-confirm-files: `PROMPT "file"` or the file list and `PROMPT N files`.
    fn confirm_files(&mut self, action: &str, prompt: &str, files: &[String]) -> Result<bool> {
        match files {
            [] => Ok(true),
            [f] => self.confirm(action, &format!("{prompt} \"{f}\"")),
            fs => self.confirm(action, &format!("{}\n\n{prompt} {} files", fs.join("\n"), fs.len())),
        }
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
    /// (or the heading) of hunk HUNK, on the old side when GOTO_FROM. The
    /// lines before it are counted unless their prefix (a column per
    /// parent in combined diffs) has a `-` (`+` for GOTO_FROM). On the
    /// heading Magit moves past the first `-` or `+` of the body, so that
    /// line is counted too.
    fn hunk_line(fd: &FileDiff, hunk: usize, line: Option<usize>, goto_from: bool) -> usize {
        let Some(h) = fd.hunks.get(hunk) else { return 1 };
        let start = h.range().map_or(1, |r| if goto_from { r.old_start } else { r.new_start }).max(1);
        let prefix = h.header.chars().take_while(|c| *c == '@').count().saturating_sub(1).max(1);
        let skip = if goto_from { '+' } else { '-' };
        let target = match line {
            Some(li) => li,
            None => h.lines.iter().position(|l| l.starts_with(['-', '+'])).map_or(0, |i| i + 1),
        };
        start + h.lines.iter().take(target).filter(|l| !l.chars().take(prefix).any(|c| c == skip)).count()
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
            (Some((h, Some(l))), Some(fd)) => {
                !fd.combined && fd.hunks.get(h).and_then(|x| x.lines.get(l)).is_some_and(|t| t.starts_with('-'))
            }
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
            self.set_msg(&format!("{e:#}"));
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
        self.settle();
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
                // magit-blame-visit-other-file, keeping the style; a blob
                // buffer is read-only
                let blame = Blame { typ, style: b.style, recursive: true, args, read_only: true };
                self.setup_buffer(BufferMode::File { rev: prev_rev, file: prev_file, blame: Some(blame) }, Vec::new());
                self.goto_file_line(chunk.orig_line);
            }
            b => {
                let style = b.map_or(0, |b| b.style);
                let recursive = matches!(&self.mode, BufferMode::File { blame: Some(b), .. } if b.recursive);
                let blame = Blame { typ, style, recursive, args, read_only: true };
                self.mode = BufferMode::File { rev, file, blame: Some(blame) };
                self.refresh();
                self.goto_file_line(line);
            }
        }
        if let Some(e) = self.buf.blame_error.clone() {
            self.set_msg(&e);
        }
        Ok(())
    }

    /// magit-blame-echo: blame the additions in the lines style
    /// (magit-blame-echo-style), the summary of the chunk at point echoed,
    /// without magit-blame-read-only-mode; while blaming already, turn
    /// that mode off and use that style.
    fn blame_echo(&mut self, args: Vec<String>) -> Result<()> {
        let BufferMode::File { rev, file, blame } = self.mode.clone() else {
            self.set_msg("Buffer isn't visiting a file");
            return Ok(());
        };
        if rev != "{worktree}" {
            self.set_msg("Blob buffers aren't supported");
            return Ok(());
        }
        let (line, _) = self.file_line_at_point().unwrap_or((1, None));
        let blame = match blame {
            Some(b) => Blame { style: ECHO_STYLE, read_only: false, ..b },
            None => {
                if !self.git.git_ok(&["ls-files", "--error-unmatch", "--", &file]) {
                    self.set_msg("Buffer isn't visiting a tracked file");
                    return Ok(());
                }
                Blame { typ: BlameType::Addition, style: ECHO_STYLE, recursive: false, args, read_only: false }
            }
        };
        self.mode = BufferMode::File { rev, file, blame: Some(blame) };
        self.refresh();
        self.goto_file_line(line);
        if let Some(e) = self.buf.blame_error.clone() {
            self.set_msg(&e);
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
        self.blame_previous_chunk = None;
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

    /// magit-blame-goto-chunk-hook (in post-command-hook while blaming):
    /// when point entered another chunk, magit-blame-maybe-show-message
    /// echoes its summary if the style has show-message.
    fn blame_post_command(&mut self) {
        let BufferMode::File { blame: Some(b), .. } = &self.mode else { return };
        let style = b.style;
        let Some(c) = self.current_blame_chunk() else { return };
        let id = (self.buffer_name_of(&self.mode), c.orig_rev.clone(), c.final_line);
        if self.blame_previous_chunk.as_ref() == Some(&id) {
            return;
        }
        self.blame_previous_chunk = Some(id);
        if style_shows_message(style) {
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
    fn read_file_from_rev(&mut self, rev: &str, prompt: &str, include_dirs: bool) -> Result<Option<String>> {
        let mut choices = self.git.git_lines(&["ls-tree", "-r", "--full-tree", "--name-only", "-z", rev]);
        // magit-revision-directories
        if include_dirs {
            let dirs = self.git.git_lines(&["ls-tree", "-z", "--full-tree", "-r", "-d", "--name-only", rev]);
            choices.extend(dirs.into_iter().map(|d| format!("{d}/")));
            choices.sort();
        }
        let default = self.current_file_name().filter(|f| choices.contains(f));
        self.read_choice(prompt, &choices, default.as_deref())
    }

    /// magit-completing-read with require-match.
    fn read_choice(&mut self, prompt: &str, choices: &[String], default: Option<&str>) -> Result<Option<String>> {
        let table = Completion { candidates: choices.to_vec(), require_match: true, multiple: false };
        let Some(v) = self.completing_read(prompt, default, Hist::Minibuffer, Some(table))?.filter(|s| !s.is_empty())
        else {
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
                        if !self.confirm_files("untrack", "Untrack", &files)? {
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
                        if !self.confirm_files("delete", "Delete", &files)? {
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
                    if !self.yes_or_no_p(&format!("Recursively delete {what} \"{names}\":"))? {
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
                let Some(rev) = self.read_rev("Checkout from revision", default.as_deref())? else {
                    return Ok(());
                };
                let Some(file) = self.read_file_from_rev(&rev, "Checkout file", true)? else {
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
                let Some(rev) = self.read_rev("Find file from revision", default.as_deref())? else {
                    return Ok(());
                };
                let from = if rev == "{worktree}" || rev == "{index}" { "HEAD" } else { rev.as_str() };
                let Some(file) = self.read_file_from_rev(from, "Find file", false)? else {
                    return Ok(());
                };
                let line = match self.visited_file() {
                    Some((_, f)) if f == file => self.file_line().unwrap_or(1),
                    _ => 1,
                };
                if let Err(e) = super::blame::file_contents(&self.git, &rev, &file) {
                    self.set_msg(&format!("{e:#}"));
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
                && self.magit_y_or_n_p(&format!("Also rename \"{old}\" to \"{new}\" on \"{remote}\"?"), None)?
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

    /// magit-local-branch-at-point: the local branch at point, or at the
    /// commit at point.
    fn local_branch_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Local, name }) => Some(name.clone()),
            Some(LineKind::Commit { hash, .. }) => self.name_local_branch(hash),
            _ => None,
        }
    }

    /// magit-read-other-local-branch: a local branch but the current
    /// one, the default the one at point, else the previous branch.
    fn read_other_local_branch(&mut self, prompt: &str) -> Result<Option<String>> {
        let current = self.git.current_branch();
        let default = self.local_branch_at_point().filter(|b| Some(b) != current.as_ref()).or_else(|| self.previous_branch());
        let mut candidates = self.git.branches();
        candidates.retain(|b| Some(b) != current.as_ref());
        let table = Completion { candidates, require_match: true, multiple: false };
        Ok(self.completing_read(prompt, default.as_deref(), Hist::Revision, Some(table))?.filter(|b| !b.is_empty()))
    }

    /// magit-read-shelved-branch: a shelved branch (the newest last).
    fn read_shelved_branch(&mut self, prompt: &str) -> Result<Option<String>> {
        let mut candidates: Vec<String> = self
            .git
            .git_lines(&["for-each-ref", "--format=%(refname)", "refs/shelved"])
            .into_iter()
            .filter_map(|r| r.strip_prefix("refs/shelved/").map(str::to_string))
            .collect();
        candidates.reverse();
        let table = Completion { candidates, require_match: true, multiple: false };
        Ok(self.completing_read(prompt, None, Hist::Minibuffer, Some(table))?.filter(|b| !b.is_empty()))
    }

    /// magit--rename-reflog-file OLD NEW (full refnames).
    fn rename_reflog_file(&self, old: &str, new: &str) {
        let old = self.git.gitdir.join("logs").join(old);
        let new = self.git.gitdir.join("logs").join(new);
        if old.exists() {
            if let Some(dir) = new.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::rename(old, new);
        }
    }

    /// magit-branch-shelve: refs/heads/BRANCH becomes
    /// refs/shelved/YYYY-MM-DD-BRANCH (its committer date), with its
    /// reflog; its push-remote is unset.
    fn branch_shelve(&mut self) -> Result<()> {
        let Some(branch) = self.read_other_local_branch("Shelve branch")? else {
            return Ok(());
        };
        let old = format!("refs/heads/{branch}");
        let date = self.git.git_string(&["log", "-1", "--format=%cs", &old]).unwrap_or_default();
        let new = format!("refs/shelved/{}-{branch}", date.trim());
        if let Err(e) = self.git.git_interactive(&["update-ref", &new, &old, ""]) {
            self.set_msg(&format!("{e:#}"));
            return Ok(());
        }
        self.rename_reflog_file(&old, &new);
        let _ = self.git.git_interactive(&["config", "--unset", &format!("branch.{branch}.pushRemote")]);
        let res = self.git.git_interactive(&["branch", "-D", &branch]);
        self.done_keep_msg(res);
        Ok(())
    }

    /// magit-branch-unshelve: refs/shelved/BRANCH becomes
    /// refs/heads/BRANCH (a leading YYYY-MM-DD- dropped), with its
    /// reflog.
    fn branch_unshelve(&mut self) -> Result<()> {
        let Some(branch) = self.read_shelved_branch("Unshelve branch")? else {
            return Ok(());
        };
        let old = format!("refs/shelved/{branch}");
        let b = branch.as_bytes();
        let dated = b.len() > 11
            && b[..10].iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
            && b[10] == b'-';
        let new = format!("refs/heads/{}", if dated { &branch[11..] } else { &branch[..] });
        if let Err(e) = self.git.git_interactive(&["update-ref", &new, &old, ""]) {
            self.set_msg(&format!("{e:#}"));
            return Ok(());
        }
        self.rename_reflog_file(&old, &new);
        let res = self.git.git_interactive(&["update-ref", "-d", &old]);
        self.done_keep_msg(res);
        Ok(())
    }

    /// magit-tag-at-point
    fn tag_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Tag, name }) => Some(name.clone()),
            _ => None,
        }
    }

    /// The secret OpenPGP keys that can sign (`gpg --list-secret-keys`):
    /// the fingerprints of their primary keys.
    fn gpg_signing_keys(&self) -> Vec<String> {
        let Ok(out) = std::process::Command::new("gpg").args(["--list-secret-keys", "--with-colons"]).output() else {
            return Vec::new();
        };
        let mut keys: Vec<(String, bool)> = Vec::new();
        let mut want_fpr = false;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let f: Vec<&str> = line.split(':').collect();
            match f[0] {
                "sec" => {
                    keys.push((String::new(), false));
                    want_fpr = true;
                }
                "ssb" => want_fpr = false,
                "fpr" if want_fpr => {
                    if let Some(k) = keys.last_mut() {
                        k.0 = f.get(9).unwrap_or(&"").to_string();
                    }
                    want_fpr = false;
                }
                _ => {}
            }
            if matches!(f[0], "sec" | "ssb") && f.get(11).is_some_and(|c| c.contains('s')) {
                if let Some(k) = keys.last_mut() {
                    k.1 = true;
                }
            }
        }
        keys.into_iter().filter(|(f, sign)| *sign && !f.is_empty()).map(|(f, _)| f).collect()
    }

    /// magit-read-gpg-signing-key: "" unless gpg.format is openpgp; the
    /// only key, or magit-openpgp-default-signing-key, without asking
    /// unless with a prefix argument; else one read (any input).
    fn read_gpg_signing_key(&mut self, prompt: &str) -> Result<Option<String>> {
        if !matches!(self.config("gpg.format").as_deref(), None | Some("openpgp")) {
            return Ok(Some(String::new()));
        }
        let keys = self.gpg_signing_keys();
        if self.prefix_arg.is_none() {
            if keys.len() == 1 {
                return Ok(keys.into_iter().next());
            }
            if let Some(d) = self.options.openpgp_default_signing_key.clone().filter(|d| keys.contains(d)) {
                return Ok(Some(d));
            }
        }
        let table = Completion { candidates: keys, require_match: false, multiple: false };
        Ok(self.read_line_hist(prompt, None, Hist::Infix("--gpg-sign="), Some(table))?.map(|s| s.trim().to_string()))
    }

    /// magit-remote-at-point: the remote section at point, or the remote
    /// of the remote branch section at point.
    fn remote_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Remote { name }) => Some(name.clone()),
            Some(LineKind::Ref { kind: RefKind::Remote, .. }) => self.sec(self.cur_sec()).parent.and_then(|parent| {
                match &self.buf.all[self.sec(parent).start].kind {
                    LineKind::Remote { name } => Some(name.clone()),
                    _ => None,
                }
            }),
            _ => None,
        }
    }

    /// The value of the git variable VAR, if set and not empty.
    fn config(&self, var: &str) -> Option<String> {
        self.git.git_string(&["config", var]).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }

    /// magit-get-remote: the remote of the current branch, unless ".".
    fn get_remote(&self) -> Option<String> {
        let branch = self.git.current_branch()?;
        self.config(&format!("branch.{branch}.remote")).filter(|r| r != ".")
    }

    /// magit-get-current-remote with ALLOW-UNNAMED: the remote of the
    /// upstream (an url too), else the only remote, else
    /// magit-primary-remote.
    fn current_remote(&self) -> Option<String> {
        let remotes = self.git.remotes();
        let upstream = self.git.current_branch().and_then(|b| self.config(&format!("branch.{b}.remote"))).filter(|r| {
            r != "." && (remotes.contains(r) || unnamed_remote(r))
        });
        upstream.or_else(|| if remotes.len() == 1 { remotes.first().cloned() } else { self.git.primary_remote() })
    }

    /// magit-get-push-remote of BRANCH: branch.BRANCH.pushRemote, else
    /// remote.pushDefault.
    fn get_push_remote(&self, branch: Option<&str>) -> Option<String> {
        branch.and_then(|b| self.config(&format!("branch.{b}.pushRemote"))).or_else(|| self.config("remote.pushDefault"))
    }

    /// magit--push-remote-variable: branch.BRANCH.pushRemote, without a
    /// branch remote.pushDefault (SHORT: pushRemote, pushDefault).
    fn push_remote_variable(branch: Option<&str>, short: bool) -> String {
        match (branch, short) {
            (Some(_), true) => "pushRemote".into(),
            (Some(b), false) => format!("branch.{b}.pushRemote"),
            (None, true) => "pushDefault".into(),
            (None, false) => "remote.pushDefault".into(),
        }
    }

    /// magit-read-remote: one of the remotes, the default the remote at
    /// point, else that of the current branch.
    fn read_remote(&mut self, prompt: &str) -> Result<Option<String>> {
        let default = self.remote_at_point().or_else(|| self.get_remote());
        let table = Completion { candidates: self.git.remotes(), require_match: true, multiple: false };
        Ok(self.completing_read(prompt, default.as_deref(), Hist::Minibuffer, Some(table))?.filter(|r| !r.is_empty()))
    }

    /// magit-read-remote-or-url: a remote or any url (the remotes and
    /// "https://", "git://", "git@" completed).
    fn read_remote_or_url(&mut self, prompt: &str) -> Result<Option<String>> {
        let default = self.remote_at_point().or_else(|| self.get_remote());
        let mut candidates = self.git.remotes();
        candidates.extend(["https://", "git://", "git@"].map(String::from));
        let table = Completion { candidates, ..Default::default() };
        self.read_any(prompt, default.as_deref(), Hist::Minibuffer, table)
    }

    /// magit-read-refspec: one of the refs of magit-remote-list-refs of
    /// REMOTE (any input).
    fn read_refspec(&mut self, prompt: &str, remote: &str) -> Result<Option<String>> {
        self.set_msg("Determining available refs...");
        let candidates = self
            .git
            .git_lines(&["ls-remote", remote])
            .into_iter()
            .filter_map(|l| l.get(41..).map(str::to_string))
            .filter(|r| !r.ends_with("^{}"))
            .collect();
        self.set_msg("Determining available refs...done");
        let table = Completion { candidates, ..Default::default() };
        self.read_any(prompt, None, Hist::Minibuffer, table)
    }

    /// magit-remote-branch-at-point: the remote branch at point, or one
    /// at the commit at point.
    fn remote_branch_at_point(&self) -> Option<String> {
        let remote = self.git.remote_branches();
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Remote, name }) => remote.contains(name).then(|| name.clone()),
            Some(LineKind::Commit { hash, .. }) => self
                .git
                .git_lines(&["for-each-ref", "--points-at", hash, "--format=%(refname:short)", "refs/remotes"])
                .into_iter()
                .find(|r| remote.contains(r)),
            _ => None,
        }
    }

    /// magit-read-upstream-branch: an existing branch but BRANCH, the
    /// default the branch at point (the local one first), else the main
    /// branch (or origin/MAIN), else the previous branch.
    fn read_upstream_branch(&mut self, branch: &str, prompt: &str) -> Result<Option<String>> {
        let mut candidates = self.git.branches();
        candidates.extend(self.git.remote_branches());
        candidates.retain(|b| b != branch);
        let member = |b: Option<String>| b.filter(|b| candidates.contains(b));
        let default = member(self.local_branch_at_point())
            .or_else(|| member(self.remote_branch_at_point()))
            .or_else(|| {
                let main = self.git.main_branch()?;
                member(Some(main.clone())).or_else(|| member(Some(format!("origin/{main}"))))
            })
            .or_else(|| member(self.previous_branch()));
        let table = Completion { candidates, require_match: true, multiple: false };
        Ok(self.completing_read(prompt, default.as_deref(), Hist::Revision, Some(table))?.filter(|b| !b.is_empty()))
    }

    /// magit--select-push-remote: the current branch and its push-remote,
    /// which is read (and set) with a prefix argument or when it isn't
    /// an existing remote ("Set VAR and PROMPT-SUFFIX").
    /// CHANGED: whether it was read.
    fn select_push_remote(&mut self, suffix: &str) -> Result<Option<(String, String, bool)>> {
        let Some(branch) = self.git.current_branch() else {
            self.set_msg("No branch is checked out");
            return Ok(None);
        };
        let remote = self.get_push_remote(Some(&branch)).filter(|r| self.git.remotes().contains(r));
        let (remote, changed) = match remote {
            Some(r) if self.prefix_arg.is_none() => (r, false),
            _ => {
                let var = Self::push_remote_variable(Some(&branch), false);
                let Some(r) = self.read_remote(&format!("Set {var} and {suffix}"))? else {
                    return Ok(None);
                };
                self.git.git_interactive(&["config", &var, &r])?;
                (r, true)
            }
        };
        Ok(Some((branch, remote, changed)))
    }

    /// magit-pull--upstreams: the upstream of the current branch if it is
    /// a local branch, and its upstream if that is a remote branch.
    fn pull_upstreams(&self) -> Option<(String, String)> {
        let up1 = self.upstream_branch(&self.git.current_branch()?)?;
        let is = |prefix: &str, b: &str| self.git.git_ok(&["show-ref", "--verify", "--quiet", &format!("{prefix}{b}")]);
        if !is("refs/heads/", &up1) {
            return None;
        }
        let up2 = self.upstream_branch(&up1)?;
        is("refs/remotes/", &up2).then_some((up1, up2))
    }

    /// magit-pull--pushbranch-description of BRANCH.
    fn pull_pushbranch_description(&self, b: &str, remotes: &[String]) -> String {
        let v = Self::push_remote_variable(Some(b), true);
        match self.get_push_remote(Some(b)) {
            Some(r) if self.git.rev_verify(&format!("{r}/{b}")) => format!("{r}/{b}"),
            Some(r) if remotes.contains(&r) => format!("{v}, replacing non-existent"),
            Some(_) => format!("{v}, replacing invalid"),
            None => format!("{v}, setting that"),
        }
    }

    /// The descriptions and `:if` of the suffixes of the menu NAME that
    /// depend on the repository, the git variable infixes included.
    fn menu_states(&self, name: &str) -> Vec<(String, String, bool)> {
        let mut states: Vec<(String, String, bool)> =
            self.menu_suffix_states(name).into_iter().map(|(k, d, s)| (k.to_string(), d, s)).collect();
        states.extend(self.var_suffix_states(name));
        states
    }

    /// Update the descriptions of the menu NAME shown (transient
    /// :refresh-suffixes after an infix set a git variable).
    fn refresh_menu_states(&mut self, name: &str) {
        let states = self.menu_states(name);
        if let Some(m) = self.menus.get_mut(name) {
            for e in m.groups.iter_mut().flatten() {
                if let Some((_, desc, _)) = states.iter().find(|(k, _, _)| *k == e.key) {
                    e.desc.clone_from(desc);
                }
            }
        }
    }

    /// The descriptions and `:if` of the suffixes of NAME that depend on
    /// the repository: (key, description, shown).
    fn menu_suffix_states(&self, name: &str) -> Vec<(&'static str, String, bool)> {
        let branch = self.git.current_branch();
        let remotes = self.git.remotes();
        match name {
            // magit-gitignore-on-system's description
            "gitignore-menu" => {
                let file = self.config("core.excludesfile");
                let file = file.as_deref().unwrap_or("core.excludesfile is not set");
                vec![("g", format!("privately for all repositories ({file})"), true)]
            }
            "fetch-menu" => {
                // magit-fetch--pushremote-description
                let remote = self.get_push_remote(branch.as_deref());
                let v = Self::push_remote_variable(branch.as_deref(), true);
                let p = match remote {
                    Some(r) if remotes.contains(&r) => r,
                    Some(_) => format!("{v}, replacing invalid"),
                    None => format!("{v}, setting that"),
                };
                let u = self.current_remote();
                vec![("p", p, true), ("u", u.clone().unwrap_or_default(), u.is_some())]
            }
            "pull-menu" => {
                let Some(b) = branch else {
                    return ["p", "u", "U"].map(|k| (k, String::new(), false)).into();
                };
                let p = self.pull_pushbranch_description(&b, &remotes);
                // magit-pull--upstream-description
                let remote = self.config(&format!("branch.{b}.remote"));
                let merge = self.config(&format!("branch.{b}.merge"));
                let u = match self.upstream_branch(&b) {
                    Some(up) => up,
                    None if unnamed_upstream(remote.as_deref(), merge.as_deref()) => {
                        format!("{} of {}", merge.unwrap_or_default(), remote.unwrap_or_default())
                    }
                    None if valid_upstream(remote.as_deref(), merge.as_deref(), &remotes) => {
                        "@{upstream}, replacing non-existent".into()
                    }
                    None if remote.is_some() || merge.is_some() => "@{upstream}, replacing invalid".into(),
                    None => "@{upstream}, setting that".into(),
                };
                let ups = self.pull_upstreams();
                vec![
                    ("p", p, true),
                    ("u", u, true),
                    ("U", ups.clone().map(|(_, u)| u).unwrap_or_default(), ups.is_some()),
                ]
            }
            "rebase-menu" => {
                let Some(b) = branch else {
                    return ["p", "u"].map(|k| (k, String::new(), false)).into();
                };
                let p = self.pull_pushbranch_description(&b, &remotes);
                // magit-rebase--upstream-description
                let remote = self.config(&format!("branch.{b}.remote"));
                let merge = self.config(&format!("branch.{b}.merge"));
                let u = match self.upstream_branch(&b) {
                    Some(up) => up,
                    None if unnamed_upstream(remote.as_deref(), merge.as_deref()) => "@{upstream}, replacing unnamed".into(),
                    None if valid_upstream(remote.as_deref(), merge.as_deref(), &remotes) => {
                        "@{upstream}, replacing non-existent".into()
                    }
                    None if remote.is_some() || merge.is_some() => "@{upstream}, replacing invalid".into(),
                    None => "@{upstream}, setting that".into(),
                };
                vec![("p", p, true), ("u", u, true)]
            }
            "push-menu" => {
                let Some(b) = branch else {
                    return ["p", "u", "e"].map(|k| (k, String::new(), false)).into();
                };
                // magit-push--pushbranch-description
                let remote = self.get_push_remote(Some(&b));
                let v = Self::push_remote_variable(Some(&b), true);
                let p = match remote {
                    Some(r) if self.git.rev_verify(&format!("{r}/{b}")) => format!("{r}/{b}"),
                    Some(r) if remotes.contains(&r) => format!("{r}/{b}, creating it"),
                    Some(_) => format!("{v}, replacing invalid"),
                    None => format!("{v}, setting that"),
                };
                // magit-push--upstream-description
                let remote = self.config(&format!("branch.{b}.remote"));
                let merge = self.config(&format!("branch.{b}.merge"));
                let u = match self.upstream_branch(&b) {
                    Some(up) => up,
                    None if unnamed_upstream(remote.as_deref(), merge.as_deref()) => {
                        format!("{} as {}", remote.unwrap_or_default(), merge.unwrap_or_default())
                    }
                    None if valid_upstream(remote.as_deref(), merge.as_deref(), &remotes) => {
                        format!("{} creating {}", remote.unwrap_or_default(), merge.unwrap_or_default())
                    }
                    None if remote.is_some() || merge.is_some() => "@{upstream}, creating it and replacing invalid".into(),
                    None => "@{upstream}, creating it".into(),
                };
                vec![("p", p, true), ("u", u, true), ("e", "elsewhere".into(), true)]
            }
            _ => Vec::new(),
        }
    }

    /// magit-get-tracked: the remote and the ref on it of the
    /// remote-tracking branch REFNAME (per the remote.*.fetch refspecs).
    fn get_tracked(&self, refname: &str) -> Option<(String, String)> {
        let full = self
            .git
            .git_string(&["rev-parse", "--verify", "-q", "--symbolic-full-name", refname])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        self.git.git_lines(&["config", "--local", "--list"]).iter().find_map(|line| {
            let rest = line.strip_prefix("remote.")?;
            let (remote, rest) = rest.split_once(".fetch=")?;
            if remote.contains('.') {
                return None;
            }
            let (src, dst) = rest.trim_start_matches('+').split_once(':')?;
            let star = match dst.split_once('*') {
                Some((pre, post)) => full.strip_prefix(pre)?.strip_suffix(post).filter(|m| !m.is_empty())?,
                None if dst == full => "",
                None => return None,
            };
            Some((remote.to_string(), src.replacen('*', star, 1)))
        })
    }

    /// magit-split-branch-name: (".", BRANCH) for a local branch, else
    /// the remote and the branch on it.
    fn split_branch_name(&self, branch: &str) -> std::result::Result<(String, String), String> {
        if self.git.branches().iter().any(|b| b == branch) {
            return Ok((".".into(), branch.into()));
        }
        self.git
            .remotes()
            .into_iter()
            .find_map(|r| {
                let b = branch.strip_prefix(&format!("{r}/")).filter(|b| !b.is_empty())?.to_string();
                Some((r, b))
            })
            .ok_or_else(|| format!("Invalid branch name {branch}"))
    }

    /// magit-read-remote with USE-ONLY: the only remote without asking.
    fn read_remote_use_only(&mut self, prompt: &str) -> Result<Option<String>> {
        let remotes = self.git.remotes();
        if remotes.len() == 1 {
            return Ok(remotes.into_iter().next());
        }
        self.read_remote(prompt)
    }

    /// magit-read-remote-branch without REMOTE, with LOCAL-BRANCH and
    /// REQUIRE-MATCH confirm: REMOTE/LOCAL-BRANCH for every remote and
    /// the remote branches offered; the choice must have the form
    /// REMOTE/BRANCH.
    fn read_remote_branch(&mut self, prompt: &str, default: Option<&str>, local: &str) -> Result<Option<String>> {
        let mut candidates: Vec<String> = self.git.remotes().iter().map(|r| format!("{r}/{local}")).collect();
        for b in self.git.remote_branches() {
            if !candidates.contains(&b) {
                candidates.push(b);
            }
        }
        let table = Completion { candidates, ..Default::default() };
        let Some(choice) = self.read_any(prompt, default, Hist::Revision, table)? else {
            return Ok(None);
        };
        if !choice.split_once('/').is_some_and(|(r, b)| !r.is_empty() && !b.is_empty()) {
            self.set_msg(&format!("`{choice}' doesn't have the form REMOTE/BRANCH"));
            return Ok(None);
        }
        Ok(Some(choice))
    }

    /// magit-read-local-branch-or-commit: a local branch, a special ref
    /// (HEAD, ORIG_HEAD, ... that exist) or the commit at point (the
    /// default after the local branch at point).
    fn read_local_branch_or_commit(&mut self, prompt: &str) -> Result<Option<String>> {
        let commit = self.commit_at_point();
        let mut candidates: Vec<String> = commit.iter().cloned().collect();
        candidates.extend(self.git.branches());
        candidates.extend(
            ["HEAD", "ORIG_HEAD", "FETCH_HEAD", "MERGE_HEAD", "CHERRY_PICK_HEAD"]
                .iter()
                .filter(|r| self.git.gitdir.join(r).exists())
                .map(|r| r.to_string()),
        );
        let default = self.local_branch_at_point().or(commit);
        let table = Completion { candidates, ..Default::default() };
        self.read_any(prompt, default.as_deref(), Hist::Revision, table)
    }

    /// magit-read-tag: a tag, the one at point the default.
    fn read_tag(&mut self, prompt: &str) -> Result<Option<String>> {
        let default = self.tag_at_point();
        let table = Completion { candidates: self.git.tags(), require_match: true, multiple: false };
        Ok(self.completing_read(prompt, default.as_deref(), Hist::Revision, Some(table))?.filter(|t| !t.is_empty()))
    }

    /// magit-git-push: push BRANCH to the remote branch TARGET
    /// ("REMOTE/NAME"), qualified with refs/heads/ unless it exists.
    fn git_push(&mut self, branch: &str, target: &str, args: &[String]) {
        let namespace = if self.get_tracked(target).is_some() { "" } else { "refs/heads/" };
        let (remote, name) = match self.split_branch_name(target) {
            Ok(x) => x,
            Err(e) => {
                self.set_msg(&e);
                return;
            }
        };
        let mut a: Vec<String> = vec!["push".into(), "-v".into()];
        a.extend(args.iter().cloned());
        a.extend([remote, format!("{branch}:{namespace}{name}")]);
        self.run_remote_git(&a, &format!("pushed {branch} to {target}"));
    }

    /// The suffixes of magit-push.
    fn push_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let push = |head: &[&str], tail: &[String]| -> Vec<String> {
            head.iter().map(|s| s.to_string()).chain(tail.iter().cloned()).collect()
        };
        match cmd {
            // magit-push-current-to-pushremote
            "push-current-to-pushremote" => {
                let Some((branch, remote, changed)) = self.select_push_remote("push there")? else {
                    return Ok(());
                };
                let prompt = format!("Really use \"{remote}\" as push-remote and push \"{branch}\" there");
                if changed && !self.confirm("set-and-push", &prompt)? {
                    return Ok(());
                }
                let mut a = push(&["push", "-v"], args);
                a.extend([remote.clone(), format!("refs/heads/{branch}:refs/heads/{branch}")]);
                self.run_remote_git(&a, &format!("pushed {branch} to {remote}"));
            }
            // magit-push-current-to-upstream
            "push-current-to-upstream" => self.push_current_to_upstream(args)?,
            // magit-push-current
            "push-current" => {
                let Some(branch) = self.git.current_branch() else {
                    self.set_msg("No branch is checked out");
                    return Ok(());
                };
                let Some(target) = self.read_remote_branch(&format!("Push {branch} to"), None, &branch)? else {
                    return Ok(());
                };
                self.git_push(&branch, &target, args);
            }
            // magit-push-other
            "push-other" => {
                let Some(source) = self.read_local_branch_or_commit("Push")? else {
                    return Ok(());
                };
                let push_branch = |s: &Self, b: &str| s.get_push_remote(Some(b)).map(|r| format!("{r}/{b}"));
                let default = if self.git.branches().contains(&source) {
                    push_branch(self, &source).or_else(|| self.upstream_branch(&source))
                } else if self.git.is_ancestor(&source, "HEAD") {
                    self.git.current_branch().and_then(|c| push_branch(self, &c).or_else(|| self.upstream_branch(&c)))
                } else {
                    None
                };
                let prompt = format!("Push {source} to");
                let Some(target) = self.read_remote_branch(&prompt, default.as_deref(), &source)? else {
                    return Ok(());
                };
                self.git_push(&source, &target, args);
            }
            // magit-push-refspecs
            "push-refspecs" => {
                let Some(remote) = self.read_remote("Push to remote")? else {
                    return Ok(());
                };
                let mut candidates = vec!["HEAD".to_string()];
                candidates.extend(self.git.branches());
                let table = Completion { candidates, require_match: false, multiple: true };
                let Some(input) = self.completing_read("Push refspec,s", None, Hist::Minibuffer, Some(table))? else {
                    return Ok(());
                };
                let mut a = push(&["push", "-v"], args);
                a.push(remote.clone());
                a.extend(input.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
                self.run_remote_git(&a, &format!("pushed to {remote}"));
            }
            // magit-push-matching
            "push-matching" => {
                let Some(remote) = self.read_remote_use_only("Push matching branches to")? else {
                    return Ok(());
                };
                let mut a = push(&["push", "-v"], args);
                a.extend([remote.clone(), ":".into()]);
                self.run_remote_git(&a, &format!("pushed matching branches to {remote}"));
            }
            // magit-push-tags: git push REMOTE --tags ARGS
            "push-tags" => {
                let Some(remote) = self.read_remote_use_only("Push tags to remote")? else {
                    return Ok(());
                };
                let a = push(&["push", &remote, "--tags"], args);
                self.run_remote_git(&a, &format!("pushed tags to {remote}"));
            }
            // magit-push-tag / magit-push-notes-ref: git push REMOTE REF ARGS
            "push-tag" | "push-notes-ref" => {
                let r = if cmd == "push-tag" { self.read_tag("Push tag")? } else { self.notes_read_ref("Push notes", "push-notes-ref")? };
                let Some(r) = r else {
                    return Ok(());
                };
                let Some(remote) = self.read_remote_use_only(&format!("Push {r} to remote"))? else {
                    return Ok(());
                };
                let a = push(&["push", &remote, &r], args);
                self.run_remote_git(&a, &format!("pushed {r} to {remote}"));
            }
            _ => {}
        }
        Ok(())
    }

    /// magit-push-current-to-upstream: with a prefix argument, or when
    /// the upstream isn't usable, it is read ("Set upstream of B and push
    /// there", any REMOTE/BRANCH) and set by pushing with --set-upstream.
    fn push_current_to_upstream(&mut self, args: &[String]) -> Result<()> {
        let Some(branch) = self.git.current_branch() else {
            self.set_msg("No branch is checked out");
            return Ok(());
        };
        let mut remote = self.config(&format!("branch.{branch}.remote"));
        let mut merge = self.config(&format!("branch.{branch}.merge"));
        let mut args = args.to_vec();
        let remotes = self.git.remotes();
        let usable = self.upstream_branch(&branch).is_some()
            || unnamed_upstream(remote.as_deref(), merge.as_deref())
            || valid_upstream(remote.as_deref(), merge.as_deref(), &remotes);
        if self.prefix_arg.is_some() || !usable {
            let mut candidates: Vec<String> = remotes.iter().map(|r| format!("{r}/{branch}")).collect();
            for b in self.git.remote_branches() {
                if !candidates.contains(&b) {
                    candidates.push(b);
                }
            }
            let default = self
                .remote_branch_at_point()
                .or_else(|| self.git.primary_remote().and_then(|r| self.git.remote_main_branch(&r)));
            let table = Completion { candidates, ..Default::default() };
            let prompt = format!("Set upstream of {branch} and push there");
            let Some(upstream) = self.read_any(&prompt, default.as_deref(), Hist::Revision, table)? else {
                return Ok(());
            };
            let (r, m) = match self.get_tracked(&upstream).map_or_else(|| self.split_branch_name(&upstream), Ok) {
                Ok(x) => x,
                Err(e) => {
                    self.set_msg(&e);
                    return Ok(());
                }
            };
            let m = if m.starts_with("refs/") { m } else { format!("refs/heads/{m}") };
            let prompt = format!("Really use \"{upstream}\" as upstream and push \"{branch}\" there");
            if !self.confirm("set-and-push", &prompt)? {
                return Ok(());
            }
            (remote, merge) = (Some(r), Some(m));
            if !args.iter().any(|a| a == "--set-upstream") {
                args.insert(0, "--set-upstream".into());
            }
        }
        let mut a: Vec<String> = vec!["push".into(), "-v".into()];
        a.extend(args);
        a.extend(remote.clone());
        a.push(format!("{branch}:{}", merge.unwrap_or_default()));
        self.run_remote_git(&a, &format!("pushed {branch} to {}", remote.unwrap_or_default()));
        Ok(())
    }

    /// Run the git command A (fetching, pulling, pushing) in the
    /// background, then refresh.
    fn run_remote_git(&mut self, a: &[String], ok: &str) {
        self.run_git_async(a, ok);
    }

    /// The suffixes of magit-fetch and magit-pull.
    fn fetch_pull_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let v = |head: &[&str], tail: &[String]| -> Vec<String> {
            head.iter().map(|s| s.to_string()).chain(tail.iter().cloned()).collect()
        };
        match cmd {
            // magit-fetch-from-pushremote
            "fetch-from-pushremote" => {
                let branch = self.git.current_branch();
                let mut remote = self.get_push_remote(branch.as_deref());
                if self.prefix_arg.is_some() || !remote.as_ref().is_some_and(|r| self.git.remotes().contains(r)) {
                    let var = Self::push_remote_variable(branch.as_deref(), false);
                    let Some(r) = self.read_remote(&format!("Set {var} and fetch from there"))? else {
                        return Ok(());
                    };
                    self.git.git_interactive(&["config", &var, &r])?;
                    remote = Some(r);
                }
                let r = remote.unwrap_or_default();
                self.run_remote_git(&v(&["fetch", &r], args), &format!("fetched {r}"));
            }
            // magit-fetch-from-upstream
            "fetch-from-upstream" => {
                let Some(r) = self.current_remote() else {
                    self.set_msg("The \"current\" remote could not be determined");
                    return Ok(());
                };
                self.run_remote_git(&v(&["fetch", &r], args), &format!("fetched {r}"));
            }
            "fetch-other" => {
                let Some(r) = self.read_remote("Fetch remote")? else {
                    return Ok(());
                };
                self.run_remote_git(&v(&["fetch", &r], args), &format!("fetched {r}"));
            }
            // magit-fetch-branch / magit-fetch-refspec
            "fetch-branch" | "fetch-refspec" => {
                let Some(r) = self.read_remote_or_url("Fetch from remote or url")? else {
                    return Ok(());
                };
                let what = if cmd == "fetch-branch" {
                    // magit-read-remote-branch with REMOTE: its branches
                    let prefix = format!("{r}/");
                    let candidates =
                        self.git.remote_branches().iter().filter_map(|b| b.strip_prefix(&prefix)).map(str::to_string).collect();
                    let table = Completion { candidates, ..Default::default() };
                    self.read_any("Fetch branch", None, Hist::Revision, table)?
                } else {
                    self.read_refspec("Fetch using refspec", &r)?
                };
                let Some(what) = what else {
                    return Ok(());
                };
                self.run_remote_git(&v(&["fetch", &r, &what], args), &format!("fetched {r} {what}"));
            }
            // magit-fetch-all
            "fetch-all" => self.run_remote_git(&v(&["fetch", "--all"], args), "fetched all remotes"),
            // magit-fetch-modules: with a prefix argument its transient,
            // else with its value
            "fetch-modules" | "fetch-modules-run" => {
                if cmd == "fetch-modules" && self.prefix_arg.is_some() {
                    self.open_menu("fetch-modules-menu");
                    return Ok(());
                }
                let args = if cmd == "fetch-modules" { self.prefix_args("fetch-modules-menu") } else { args.to_vec() };
                let res = self.git.fetch_modules(&args);
                if self.git.remotes().is_empty() {
                    self.set_msg("Cannot fetch modules if super-repository has no remotes");
                    self.refresh();
                } else {
                    self.done("fetched modules", res);
                }
            }
            // magit-pull-from-pushremote
            "pull-from-pushremote" => {
                let Some((branch, remote, _)) = self.select_push_remote("pull from there")? else {
                    return Ok(());
                };
                let mut a = v(&["pull"], args);
                a.extend([remote.clone(), branch.clone()]);
                self.run_remote_git(&a, &format!("pulled {remote}/{branch}"));
            }
            // magit-pull-from-upstream
            "pull-from-upstream" => {
                let Some(branch) = self.git.current_branch() else {
                    self.set_msg("No branch is checked out");
                    return Ok(());
                };
                let get = |s: &Self, k: &str| s.config(&format!("branch.{branch}.{k}"));
                let (mut remote, mut merge) = (get(self, "remote"), get(self, "merge"));
                if self.prefix_arg.is_some()
                    || !(self.upstream_branch(&branch).is_some() || unnamed_upstream(remote.as_deref(), merge.as_deref()))
                {
                    let prompt = format!("Set upstream of {branch} and pull from there");
                    let Some(up) = self.read_upstream_branch(&branch, &prompt)? else {
                        return Ok(());
                    };
                    self.git.git_interactive(&["branch", "--set-upstream-to", &up, &branch])?;
                    remote = get(self, "remote");
                    merge = get(self, "merge");
                }
                let mut a = v(&["pull"], args);
                a.extend(remote.into_iter().chain(merge));
                self.run_remote_git(&a, "pulled");
            }
            // magit-pull-into-upstream: fetch the upstream of the
            // upstream, then update the upstream to it
            "pull-into-upstream" => {
                let Some((up1, up2)) = self.pull_upstreams() else {
                    self.set_msg("Cannot perform background update of upstream branch");
                    return Ok(());
                };
                let (remote, branch) = self.git.tracked_of(&up2);
                let res = self
                    .git
                    .git_interactive(&["fetch", &remote, &branch])
                    .and_then(|()| self.git.git_interactive(&["update-ref", &format!("refs/heads/{up1}"), "FETCH_HEAD"]));
                match res {
                    Ok(()) => self.set_msg(&format!("updated {up1} from {up2}")),
                    Err(e) => self.report_error(&e),
                }
                self.refresh();
            }
            // magit-pull-branch: a remote branch
            "pull-branch" => {
                let table = Completion { candidates: self.git.remote_branches(), require_match: true, multiple: false };
                let Some(source) = self.completing_read("Pull", None, Hist::Revision, Some(table))? else {
                    return Ok(());
                };
                let Some((remote, branch)) = source.split_once('/').filter(|(r, b)| !r.is_empty() && !b.is_empty()) else {
                    self.set_msg(&format!("`{source}' doesn't have the form REMOTE/BRANCH"));
                    return Ok(());
                };
                let (remote, branch) = match self.git.tracked_of(&source) {
                    (r, b) if r != "." => (r, b),
                    _ => (remote.to_string(), branch.to_string()),
                };
                let mut a = v(&["pull"], args);
                a.extend([remote, branch]);
                self.run_remote_git(&a, &format!("pulled {source}"));
            }
            _ => {}
        }
        Ok(())
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

    /// magit-get-indirect-upstream-branch: the upstream of BRANCH if it
    /// is the remote branch of the same name and can be fast-forwarded to
    /// BRANCH, unless FORCE only if BRANCH matches
    /// magit-branch-prefer-remote-upstream (a member that is a valid
    /// branch name exactly, else as a regexp).
    fn indirect_upstream_branch(&self, branch: &str, force: bool) -> Option<String> {
        let remote = self.git.git_string(&["config", &format!("branch.{branch}.remote")]).ok()?;
        if remote.is_empty() || remote == "." {
            return None;
        }
        let opted_in = |p: &String| {
            if self.git.git_ok(&["check-ref-format", "--branch", p]) {
                p == branch
            } else {
                super::regexp::Regexp::new(p).is_ok_and(|re| re.is_match(branch))
            }
        };
        if !force && !self.options.branch_prefer_remote_upstream.iter().any(opted_in) {
            return None;
        }
        let upstream = self.upstream_branch(branch)?;
        (upstream.strip_prefix(&format!("{remote}/")) == Some(branch) && self.git.is_ancestor(&upstream, branch))
            .then_some(upstream)
    }

    /// magit-branch-p: whether NAME is a local or remote branch.
    fn is_branch(&self, name: &str) -> bool {
        ["refs/heads/", "refs/remotes/"]
            .iter()
            .any(|p| self.git.git_ok(&["show-ref", "--verify", "--quiet", &format!("{p}{name}")]))
    }

    /// magit-branch-maybe-adjust-upstream: when BRANCH has an upstream,
    /// the indirect upstream of START-POINT (magit-branch-prefer-remote-
    /// upstream) becomes its upstream; else for the remote branch
    /// START-POINT the first existing upstream of
    /// magit-branch-adjust-remote-upstream-alist whose rule matches its
    /// name (the remote stripped).
    fn branch_maybe_adjust_upstream(&self, branch: &str, start: &str) {
        let upstream = self
            .upstream_branch(branch)
            .and_then(|_| self.indirect_upstream_branch(start, false))
            .or_else(|| {
                let (_, name) = self.split_remote_branch(start)?;
                self.options
                    .branch_adjust_remote_upstream
                    .iter()
                    .find(|(up, rule)| self.is_branch(up) && rule.applies(&name))
                    .map(|(up, _)| up.clone())
            });
        if let Some(u) = upstream {
            let _ = self.git.git_interactive(&["branch", &format!("--set-upstream-to={u}"), branch]);
        }
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

    /// magit-worktree-delete: a worktree other than the primary one,
    /// confirmed (`danger` with uncommitted changes, else `trash` or
    /// `delete`), moved to the trash (magit-delete-by-moving-to-trash)
    /// or removed with `git worktree remove --force`, then pruned.
    fn worktree_delete(&mut self) -> Result<()> {
        // magit-list-worktrees: (path, locked reason)
        let mut worktrees: Vec<(String, Option<String>)> = Vec::new();
        for line in self.git.git_lines(&["worktree", "list", "--porcelain"]) {
            if let Some(p) = line.strip_prefix("worktree ") {
                worktrees.push((p.to_string(), None));
            } else if let (Some(l), Some(w)) = (line.strip_prefix("locked"), worktrees.last_mut()) {
                w.1 = Some(l.trim().to_string());
            }
        }
        let choices: Vec<String> = worktrees.iter().skip(1).map(|w| w.0.clone()).collect();
        let at_point = self.worktree_at_point().filter(|p| choices.contains(p));
        let Some(path) = self.read_choice("Delete worktree", &choices, at_point.as_deref())? else {
            return Ok(());
        };
        let dir = std::path::PathBuf::from(&path);
        if dir.join(".git").is_dir() {
            self.set_msg(&format!("Deleting {path} would delete the shared .git directory"));
            return Ok(());
        }
        if let Some(Some(reason)) = worktrees.iter().find(|w| w.0 == path).map(|w| w.1.clone()) {
            let reason = if reason.is_empty() { String::new() } else { format!(" [reason: {reason}]") };
            self.set_msg(&format!("Cannot delete locked {path}{reason}"));
            return Ok(());
        }
        let trash = self.options.delete_by_moving_to_trash;
        if dir.exists() {
            let uncommitted = !self.git.git_lines(&["-C", &path, "status", "--porcelain"]).is_empty();
            let action = match (uncommitted, trash) {
                (true, _) => "danger",
                (false, true) => "trash",
                (false, false) => "delete",
            };
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let prompt = format!(
                "{} worktree \"{name}\"{}",
                if trash { "Trash" } else { "Delete" },
                if uncommitted { " despite uncommitted changes" } else { "" }
            );
            if !self.confirm(action, &prompt)? {
                return Ok(());
            }
            let res = if trash {
                super::trash::move_to_trash(&dir, self.options.trash_directory.as_deref())
            } else {
                self.git.git_interactive(&["worktree", "remove", "--force", &path]).map_err(|e| e.to_string())
            };
            if let Err(e) = res {
                self.set_msg(&e);
                return Ok(());
            }
        }
        if self.git.toplevel.exists() {
            let res = self.git.git_interactive(&["worktree", "prune"]);
            self.done("", res.map(|_| String::new()));
            return Ok(());
        }
        // the worktree of this buffer was deleted: pruned from the primary
        // one, whose status replaces a status buffer
        let primary = std::path::PathBuf::from(&worktrees[0].0);
        let git = match Git::discover(&primary) {
            Ok(g) => g,
            Err(e) => {
                self.set_msg(&e.to_string());
                return Ok(());
            }
        };
        let res = git.git_interactive(&["worktree", "prune"]);
        self.switch_repository(git);
        self.done("", res.map(|_| String::new()));
        Ok(())
    }

    /// Show the repository of GIT: the buffers of the current one are
    /// killed, a status buffer becomes that of GIT.
    fn switch_repository(&mut self, git: Git) {
        self.cwd = git.toplevel.clone();
        self.git = git;
        self.buried.clear();
        self.background.clear();
        self.clear_region();
        self.view_text = None;
        if self.mode == BufferMode::Status {
            self.buf = StatusBuffer::default();
            self.vis.clear();
            self.lines.clear();
            self.cursor = 0;
            self.scroll = 0;
        }
    }

    /// magit-branch-reset: reset BRANCH (read, the local branch at point
    /// or the current one the default) to a revision (the branch or
    /// commit at point, else its upstream, the default). The current
    /// branch is reset hard, uncommitted changes have to be confirmed
    /// (yes-or-no-p); another one with update-ref. With a prefix
    /// argument a branch reset to becomes the upstream.
    fn branch_reset(&mut self) -> Result<()> {
        let current = self.git.current_branch();
        let default = self.local_branch_at_point().or_else(|| current.clone());
        let branches = self.git.branches();
        let Some(branch) = self.pick_rev("Reset branch", &branches, default.as_deref())?.cloned() else {
            return Ok(());
        };
        // magit-read-branch-or-commit with BRANCH excluded
        let at_point = self.branch_at_point().filter(|b| *b != branch);
        let commit = self.commit_at_point().map(|c| self.git.rev_short(&c));
        let to_default = at_point
            .or(commit)
            .or_else(|| self.upstream_branch(&branch))
            .or_else(|| current.clone().filter(|c| *c != branch));
        let Some(to) = self.read_rev(&format!("Reset {branch} to"), to_default.as_deref())? else {
            return Ok(());
        };
        if current.as_deref() == Some(branch.as_str()) {
            let modified = !self.git.staged_files().is_empty() || !self.git.unstaged_files().is_empty();
            if modified && !self.yes_or_no_p("Uncommitted changes will be lost.  Proceed?")? {
                self.set_msg("Abort");
                return Ok(());
            }
            if let Err(e) = self.git.reset_hard(&to) {
                self.refresh();
                self.set_msg(&format!("{e:#}"));
                return Ok(());
            }
        } else {
            let msg = format!("reset: moving to {to}");
            if let Err(e) = self.git.git_interactive(&["update-ref", "-m", &msg, &format!("refs/heads/{branch}"), &to]) {
                self.set_msg(&format!("{e:#}"));
                return Ok(());
            }
        }
        if self.prefix_arg.is_some() && self.is_branch(&to) {
            self.git.git_interactive(&["branch", &format!("--set-upstream-to={to}"), &branch])?;
            self.branch_maybe_adjust_upstream(&branch, &to);
        }
        self.refresh();
        Ok(())
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
                if !self.confirm("t", &format!("Delete {} branches", bs.len()))? {
                    return Ok(());
                }
                bs
            }
            _ => {
                let mut choices = self.git.branches();
                choices.extend(self.git.remote_branches());
                let default = self.branch_prefer_other_default();
                let prompt = if force { "Force delete branch" } else { "Delete branch" };
                match self.pick_rev(prompt, &choices, default.as_deref())? {
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
                if self.confirm_noabort("delete-unmerged-branch", &prompt)? {
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
                self.branch_maybe_delete_pr_remote(b)?;
            }
            for b in &branches {
                self.branch_unset_push_remote(b);
            }
            let mut a = vec!["branch", if force { "-D" } else { "-d" }];
            a.extend(branches.iter().map(String::as_str));
            let res = self.git.git_interactive(&a);
            self.done(&format!("deleted {}", branches.join(", ")), res.map(|()| String::new()));
            return Ok(());
        }
        let branch = branches.remove(0);
        if self.git.current_branch().as_deref() == Some(branch.as_str()) {
            let mut target = self.indirect_upstream_branch(&branch, true);
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
                && !self.confirm("delete-unmerged-branch", &format!("Delete unmerged branch {branch}"))?
            {
                return Ok(());
            }
            match &checkout {
                Some(t) => self.git.git_interactive(&["checkout", "-q", t])?,
                None => self.git.git_interactive(&["checkout", "-q", "--detach"])?,
            }
            force = true;
        }
        self.branch_maybe_delete_pr_remote(&branch)?;
        self.branch_unset_push_remote(&branch);
        let res = self.git.git_interactive(&["branch", if force { "-D" } else { "-d" }, &branch]);
        self.done(&format!("deleted {branch}"), res.map(|()| String::new()));
        Ok(())
    }

    /// magit-visit-ref on the ref NAME of KIND in the refs buffer, as
    /// magit-visit-ref-behavior says, else like magit-show-commit.
    fn visit_ref(&mut self, kind: RefKind, name: &str) -> Result<()> {
        let o = &self.options;
        let (focus, create, any, branch) =
            (o.visit_ref("focus-on-ref"), o.visit_ref("create-branch"), o.visit_ref("checkout-any"), o.visit_ref("checkout-branch"));
        let behavior = !o.visit_ref_behavior.is_empty();
        let prefix = self.prefix_arg.is_some();
        let checkout = |app: &mut Self, args: &[&str], focus: &str| -> Result<()> {
            let res = app.git.git_interactive(args);
            if let BufferMode::Refs { upstream } = &mut app.mode {
                *upstream = focus.to_string();
            }
            app.refresh();
            if let Err(e) = res {
                app.set_msg(&format!("{e:#}"));
            }
            Ok(())
        };
        if prefix && focus {
            // magit-refs-setup-buffer with the ref at point
            if let BufferMode::Refs { upstream } = &mut self.mode {
                *upstream = name.to_string();
            }
            self.prefix_arg = None;
            self.refresh();
            return Ok(());
        }
        if prefix && behavior {
            // magit-show-commit without prompting
            self.prefix_arg = None;
            return self.diff_view("commit");
        }
        if create && kind == RefKind::Remote {
            let branch = name.split_once('/').map_or(name, |(_, b)| b).to_string();
            if !self.git.git_ok(&["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]) {
                return checkout(self, &["checkout", "-b", &branch, name], &branch);
            }
            if self.git.rev_parse(&branch).ok() == self.git.rev_parse(name).ok() {
                return checkout(self, &["checkout", &branch], &branch);
            }
            let prompt = format!(
                "Branch {branch} already exists.\n  [c]heckout {branch} as-is\n  [r]reset {branch} to {name} and checkout {branch}\n  [a]bort "
            );
            return match self.read_char_choice(&prompt, &['c', 'r', 'a'])? {
                Some('c') => checkout(self, &["checkout", &branch], &branch),
                Some('r') => checkout(self, &["checkout", "-B", &branch, name], &branch),
                _ => {
                    self.set_msg("Abort");
                    Ok(())
                }
            };
        }
        if any || (branch && kind == RefKind::Local) {
            return checkout(self, &["checkout", name], name);
        }
        self.diff_view("commit")
    }

    /// read-char-choice: one of CHOICES (none: C-g).
    fn read_char_choice(&mut self, prompt: &str, choices: &[char]) -> Result<Option<char>> {
        loop {
            let keys = choices.iter().map(|c| (c.to_string(), c.to_string())).collect();
            let Some(ans) = self.read_instant(prompt, keys)? else {
                return Ok(None);
            };
            let mut it = ans.trim().chars();
            match (it.next(), it.next()) {
                (Some(c), None) if choices.contains(&c) => return Ok(Some(c)),
                _ => continue,
            }
        }
    }

    /// magit-branch-unset-pushRemote
    fn branch_unset_push_remote(&self, branch: &str) {
        let _ = self.git.git_ok(&["config", "--unset", &format!("branch.{branch}.pushRemote")]);
    }

    /// magit-branch-maybe-delete-pr-remote: the remote of a pull-request
    /// branch (branch.BRANCH.pullRequestRemote) that only fetches single
    /// branches loses the refspec of BRANCH (that of its push branch, else
    /// its upstream); a remote left with nothing to fetch is removed if
    /// confirmed.
    fn branch_maybe_delete_pr_remote(&mut self, branch: &str) -> Result<()> {
        let get = |key: &str| self.git.git_string(&["config", "--get", key]).ok().filter(|v| !v.is_empty());
        let Some(remote) = get(&format!("branch.{branch}.pullRequestRemote")) else {
            return Ok(());
        };
        let variable = format!("remote.{remote}.fetch");
        let refspecs: Vec<String> = self
            .git
            .git_string(&["config", "--get-all", &variable])
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default();
        if refspecs.contains(&format!("+refs/heads/*:refs/remotes/{remote}/*")) {
            return Ok(());
        }
        let refspec = if get(&format!("branch.{branch}.pushRemote")).as_deref() == Some(remote.as_str()) {
            Some(format!("+refs/heads/{branch}:refs/remotes/{remote}/{branch}"))
        } else {
            get(&format!("branch.{branch}.merge"))
                .and_then(|m| m.strip_prefix("refs/heads/").map(str::to_string))
                .map(|m| format!("+refs/heads/{m}:refs/remotes/{remote}/{m}"))
        };
        let Some(refspec) = refspec.filter(|r| refspecs.contains(r)) else {
            return Ok(());
        };
        if refspecs.len() == 1
            && self.confirm_noabort("delete-pr-remote", &format!("Also delete remote {remote} (no pull-request branch remains)"))?
        {
            self.git.git_interactive(&["remote", "rm", &remote])?;
        } else {
            let pattern = format!("^{}$", regex_quote(&refspec));
            self.git.git_interactive(&["config", "--unset-all", &variable, &pattern])?;
        }
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
            [f] => format!("Deleting local {f}.  Also delete on {remote}"),
            _ => format!("Deleting {} local refs.  Also delete on {remote}", fulls.len()),
        };
        if !self.confirm_noabort("delete-branch-on-remote", &prompt)? {
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
                self.set_msg(&format!("{e:#}"));
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
            BufferMode::RebaseTodo => return "git-rebase-todo".into(),
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
        self.settle();
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
    /// while blaming (but not after magit-blame-echo), else
    /// magit-blob-mode-map. Returns whether KEY is bound there.
    fn file_key(&mut self, key: &Key) -> Result<bool> {
        let BufferMode::File { blame, .. } = &self.mode else { return Ok(false) };
        let blaming = blame.as_ref().is_some_and(|b| b.read_only);
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
                self.forward_line(if matches!(label.as_str(), "C-n" | "down") { 1 } else { -1 });
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
            // magit-worktree-section-map: magit-visit-thing is
            // magit-worktree-status
            LineKind::Worktree { .. } => self.worktree_command("worktree-status")?,
            // magit-stash-show / magit-show-commit, reading the stash or
            // revision with a prefix argument; magit-visit-ref
            // (magit-visit-ref-behavior nil) like magit-show-commit
            LineKind::Stash { .. } => self.diff_view("stash")?,
            LineKind::Ref { kind, name } if matches!(self.mode, BufferMode::Refs { .. }) => self.visit_ref(kind, &name)?,
            LineKind::Commit { .. } | LineKind::Ref { .. } => self.diff_view("commit")?,
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

    /// magit-jump-to-diffstat-or-diff: from a file in the diffstat to its
    /// diff, from a file or hunk to its diffstat line, else to the diffstat.
    pub(super) fn jump_to_diffstat_or_diff(&mut self) {
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

/// Magit extras: bisect / worktree / submodule / notes / ediff.
/// Kept out of `run_command`'s main dispatch so each feature stays together.
impl App {
    fn done(&mut self, ok_msg: &str, res: Result<String>) {
        match res {
            Ok(out) => {
                let first = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                self.set_msg(if first.is_empty() { ok_msg } else { first });
                self.refresh();
            }
            Err(e) => self.report_error(&e),
        }
    }

    fn run_extra(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        match cmd {
            // ---- bisect
            "bisect-start" | "bisect-bad" | "bisect-good" | "bisect-mark" | "bisect-skip" | "bisect-reset" | "bisect-run" => {
                self.bisect_command(cmd, args)?
            }

            // ---- worktree
            "worktree-checkout" | "worktree-branch" | "worktree-move" | "worktree-delete" | "worktree-status" => {
                self.worktree_command(cmd)?
            }

            // ---- submodule
            "submodule-list" | "submodule-add" | "submodule-register" | "submodule-populate" | "submodule-update"
            | "submodule-sync" | "submodule-unpopulate" | "submodule-remove" => self.submodule_command(cmd, args)?,

            // ---- notes
            "notes-edit" | "notes-remove" | "notes-merge" | "notes-merge-commit" | "notes-merge-abort" | "notes-prune" => {
                self.notes_command(cmd, args)?
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
            "subtree-add" | "subtree-add-commit" | "subtree-merge" | "subtree-pull" | "subtree-push" | "subtree-split" => {
                self.subtree_command(cmd, args)?
            }

            // ---- patches
            "patch-create" | "patch-apply" | "patches-apply" | "patches-apply-maildir" | "patches-continue" | "patches-skip"
            | "patches-abort" | "patch-save" | "request-pull" => self.patch_command(cmd, args)?,

            // ---- branch configuration
            "update-default-branch" => self.update_default_branch()?,
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
                let Some(newbase) = self.read_rev("Rebase subset onto", default.as_deref())? else {
                    return Ok(());
                };
                let msg = format!("Type %p on a commit to rebase it and commits above it onto {newbase},");
                self.log_select(SelectAction::RebaseSubset { newbase, args: args.to_vec() }, Some(&msg), None);
            }
            "rebase-edit-todo" => self.rebase_todo_edit_in_progress()?,

            _ => self.set_msg(&format!("unknown command: {cmd}")),
        }
        Ok(())
    }

    /// Report the outcome of a rebase that may stop half-way.
    fn after_rebase(&mut self, res: Result<()>) {
        match res {
            Ok(()) if self.git.rebase_in_progress() => {
                self.set_msg("rebase stopped (r r continue, r s skip, r a abort)")
            }
            Ok(()) => self.set_msg("rebased"),
            Err(e) => self.report_error(&e),
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
                self.set_msg(&format!("{e:#}"));
                return Ok(false);
            }
        };
        let target = self.git.rev_parse(commit)?;
        let published = self.publishing_branches(commit);
        let all_at_commit = published
            .iter()
            .all(|b| self.git.rev_parse(b).ok().as_deref() == Some(target.as_str()));
        if !(delay_edit_confirm && all_at_commit) {
            let (m1, m2) = ("Some of these commits have already been published to ", ".\nDo you really want to modify them");
            if !self.confirm_items("rebase-published", &format!("{m1}%s{m2}"), &format!("{m1}%d public branches{m2}"), &published)? {
                return Ok(false);
            }
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
            let Some(branch) = self.pick_rev("Absorb branch", &others, None)?.cloned() else {
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
            let Some(into) = self.pick_rev(&format!("Merge `{label}' into"), &others, default.as_deref())?.cloned()
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
            Err(e) => self.report_error(&e),
        }
        self.refresh();
        Ok(())
    }

    /// magit--merge-absorb asks before merging the main branch into another one.
    fn confirm_absorb(&mut self, branch: &str) -> Result<bool> {
        if self.git.main_branch().as_deref() == Some(branch)
            && !self.yes_or_no_p(&format!("Do you really want to merge `{branch}' into another branch?"))?
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
        let Some(spec) = self.read_rev(&format!("{verb} cherry (commit or A..B)"), None)? else {
            return Ok(());
        };
        let commits = match self.git.resolve_cherries(&spec) {
            Ok(c) => c,
            Err(e) => {
                self.set_msg(&format!("{e:#}"));
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
                    _ => match self.pick_rev(&format!("Remove {cherries} from branch"), &branches, None)? {
                        Some(b) => Some(b.clone()),
                        None => return Ok(()),
                    },
                };
                self.git.cherry_move(&commits, src.as_deref(), &current, args, None, true)
            }
            "cherry-donate" => {
                let others: Vec<String> =
                    self.git.branches().into_iter().filter(|b| *b != current).collect();
                let Some(dst) = self.pick_rev(&format!("Move {cherries} to branch"), &others, None)?.cloned() else {
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
                let Some(start) = self.read_rev(
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
                self.set_msg(&format!("{e:#}"));
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
                self.set_msg(&format!("{e:#}"));
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
            Err(e) => self.report_error(&e),
        }
        self.refresh();
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
