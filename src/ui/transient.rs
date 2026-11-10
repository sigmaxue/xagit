//! ui/transient.rs — transient prefix menus mirroring Magit's dispatchers.
//!
//! Menus are plain data so they can be unit-tested against the Magit
//! definitions extracted from lisp sources.

use super::keys::{key_from_label, Key};
use std::collections::BTreeMap;

/// One entry in a transient menu: either an argument switch or an action.
#[derive(Debug, Clone)]
pub struct MenuEntry {
    pub key: String,          // "z" / "C-c" style label
    pub desc: String,
    pub action: Action,
    /// the level of the suffix (transient--default-child-level 1)
    pub level: u8,
    /// not used at the level of the prefix (set when it is shown)
    pub hidden: bool,
}

impl MenuEntry {
    /// The suffix at LEVEL, like `(LEVEL "key" ...)` in a prefix.
    pub fn level(mut self, level: u8) -> Self {
        self.level = level;
        self
    }

    /// Whether the entry's infix takes the argument A.
    fn takes(&self, a: &str) -> bool {
        match &self.action {
            Action::Switch(sw) => sw == a,
            Action::Option { arg, .. } => option_matches(arg, a),
            Action::Switches(args) => args.iter().any(|x| x == a),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    /// toggle an infix argument for the *next* action (e.g. "-u" for stash)
    Switch(String),
    /// transient-option: ARG followed by a value read by READER when it is
    /// set; typing the key again unsets it (choices are always read, the
    /// empty choice "unspecified" unsets it).
    Option { arg: String, reader: Reader },
    /// transient-switches: at most one of these arguments, the key cycles
    /// through them and then back to none.
    Switches(Vec<String>),
    /// run a named command (dispatch to App)
    Command(String),
    /// open a sub-menu
    Submenu(String),
}

/// How the value of an option is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reader {
    /// any non-empty string
    String,
    /// transient-read-number-N0 / -N+
    Number { zero: bool },
    /// transient-read-number-N+ of an option with :allow-empty (the
    /// argument alone when empty)
    OptionalNumber,
    /// magit-read-similarity-index: digits with an optional "%"
    Similarity,
    /// magit-read-char-case: one of these ("unspecified" unsets)
    Choices(Vec<String>),
    /// transient-option :choices: completing-read of one of these
    /// (require-match); typing the key again unsets it
    Pick(Vec<String>),
    /// magit-read-file-trace: "TRACE:FILE"
    FileTrace,
    /// magit-read-gpg-signing-key: a secret OpenPGP key that can sign
    /// (with only one, or magit-openpgp-default-signing-key, that one
    /// unless with a prefix argument); "" unless gpg.format is openpgp
    GpgKey,
    /// :multi-value repeat with :choices: completing-read-multiple of
    /// values separated by commas (any value), the argument repeated for
    /// each ("--push-option=a --push-option=b")
    Multi(Vec<String>),
    /// transient-files with magit-read-files of magit-modified-files
    /// (magit-stash-push's magit:--): files separated by commas, the
    /// value "-- FILE..."
    Files,
}

impl Action {
    /// The argument of this infix that is set in ACTIVE, if any.
    pub fn value_in(&self, active: &BTreeMap<String, bool>) -> Option<String> {
        let on = active.iter().filter(|(_, on)| **on).map(|(k, _)| k);
        match self {
            Action::Switch(sw) => on.into_iter().find(|k| *k == sw).cloned(),
            Action::Option { arg, reader: Reader::Multi(_) } => {
                let v: Vec<&str> = on.into_iter().filter(|k| option_matches(arg, k)).map(String::as_str).collect();
                (!v.is_empty()).then(|| v.join(" "))
            }
            Action::Option { arg, .. } => on.into_iter().find(|k| option_matches(arg, k)).cloned(),
            Action::Switches(args) => on.into_iter().find(|k| args.contains(k)).cloned(),
            _ => None,
        }
    }
}

/// Whether K is a value of the option ARG ("-n" matches "-n256", "-M" also
/// matches "-M" alone). Switches use long arguments ("--patch"), so a
/// short option's prefix never matches one of them.
pub fn option_matches(arg: &str, k: &str) -> bool {
    k.starts_with(arg) && (k.len() > arg.len() || !arg.ends_with('='))
}

#[derive(Debug, Clone, Default)]
pub struct Menu {
    pub name: String,
    pub title: String,
    /// groups of entries rendered as columns (like Magit's vector layout)
    pub groups: Vec<Vec<MenuEntry>>,
}

/// Result of looking up the keys typed so far in a menu.
#[derive(Debug)]
pub enum Lookup<'a> {
    Found(&'a MenuEntry),
    /// the keys start a longer sequence ("-" of "-a", "a" of "a m")
    Prefix,
    None,
}

/// The keys to type for a transient label, as in Emacs `kbd`:
/// "-a" is "-" then "a", "a m" is "a" then "m", "C-c" and "<tab>" are one key.
pub fn key_seq(label: &str) -> Vec<Key> {
    if label.len() > 1 && label.contains(' ') {
        label.split(' ').filter(|s| !s.is_empty()).map(key_from_label).collect()
    } else if label.len() == 2 && (label.starts_with('-') || label.starts_with('=') || label.starts_with('+')) {
        vec![key_from_label(&label[..1]), key_from_label(&label[1..])]
    } else {
        vec![key_from_label(label)]
    }
}

impl Menu {
    /// Entry bound to exactly this one key.
    pub fn find(&self, key: &Key) -> Option<&MenuEntry> {
        match self.lookup(std::slice::from_ref(key)) {
            Lookup::Found(e) => Some(e),
            _ => None,
        }
    }

    /// Match a (possibly partial) key sequence.
    pub fn lookup(&self, typed: &[Key]) -> Lookup<'_> {
        let mut prefix = false;
        for e in self.groups.iter().flatten().filter(|e| !e.hidden) {
            let seq = key_seq(&e.key);
            if seq == typed {
                return Lookup::Found(e);
            }
            if seq.len() > typed.len() && seq.starts_with(typed) {
                prefix = true;
            }
        }
        if prefix {
            Lookup::Prefix
        } else {
            Lookup::None
        }
    }

    /// All keys in order — used by tests to compare with Magit's maps.
    pub fn keys(&self) -> Vec<String> {
        let mut v = Vec::new();
        for g in &self.groups {
            for e in g {
                v.push(e.key.clone());
            }
        }
        v
    }

    /// transient--use-level-p: hide the suffixes not at a level from 1
    /// to LEVEL (the level of the prefix); LEVELS overrides the level of
    /// the suffixes by key.
    pub fn set_level(&mut self, level: u8, levels: &BTreeMap<String, u8>) {
        for e in self.groups.iter_mut().flatten() {
            let l = levels.get(&e.key).copied().unwrap_or(e.level);
            e.hidden = l == 0 || l > level;
        }
    }

    /// Whether the argument A belongs to a hidden infix only (its value
    /// isn't used then).
    pub fn hidden_arg(&self, a: &str) -> bool {
        let mut es = self.groups.iter().flatten().filter(|e| e.takes(a));
        es.clone().next().is_some() && es.all(|e| e.hidden)
    }

    /// Render like Magit's transient popup, in columns (the groups
    /// without a suffix at the prefix's level omitted); an infix that is
    /// set is followed by its value, as in "-n Limit number of commits (-n256)".
    pub fn render(&self, active: &BTreeMap<String, bool>) -> String {
        let mut out = String::new();
        let cells: Vec<Vec<String>> = self
            .groups
            .iter()
            .map(|g| {
                g.iter()
                    .filter(|e| !e.hidden)
                    .flat_map(|e| display(e, active).split('\n').map(str::to_string).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .filter(|g| !g.is_empty())
            .collect();
        let ncols = cells.len();
        let widths: Vec<usize> = cells.iter().map(|g| g.iter().map(|d| d.chars().count()).max().unwrap_or(0)).collect();
        let nrows = cells.iter().map(Vec::len).max().unwrap_or(0);
        for row in 0..nrows {
            let mut line = String::new();
            for (c, w) in widths.iter().enumerate().take(ncols) {
                let d = cells[c].get(row).map_or("", String::as_str);
                line.push_str(&format!("{d:<w$}", w = w + 3));
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }
}

fn display(e: &MenuEntry, active: &BTreeMap<String, bool>) -> String {
    match e.action.value_in(active) {
        Some(v) => format!("{} {} ({v})", e.key, e.desc),
        None => format!("{} {}", e.key, e.desc),
    }
}

fn opt(key: &str, desc: &str, arg: &str, reader: Reader) -> MenuEntry {
    MenuEntry {
        key: key.into(),
        desc: desc.into(),
        action: Action::Option { arg: arg.into(), reader },
        level: 1,
        hidden: false,
    }
}
fn choices(c: &[&str]) -> Reader {
    Reader::Choices(c.iter().map(|s| s.to_string()).collect())
}
fn pick(c: &[&str]) -> Reader {
    Reader::Pick(c.iter().map(|s| s.to_string()).collect())
}
fn switches(key: &str, desc: &str, args: &[&str]) -> MenuEntry {
    MenuEntry { key: key.into(), desc: desc.into(), action: Action::Switches(args.iter().map(|s| s.to_string()).collect()), level: 1, hidden: false }
}
/// magit-merge:--strategy ("-s" for merge, "=s" for cherry-pick and revert)
fn merge_strategy(key: &str) -> MenuEntry {
    opt(key, "Strategy", "--strategy=", pick(&["resolve", "recursive", "octopus", "ours", "subtree"]))
}
/// magit-merge:--strategy-option
fn strategy_option() -> MenuEntry {
    opt("-X", "Strategy Option", "--strategy-option=", pick(&["ours", "theirs", "patience"]))
}
/// magit-diff:--diff-algorithm with the argument "-Xdiff-algorithm="
fn x_diff_algorithm(key: &str) -> MenuEntry {
    opt(key, "Diff algorithm", "-Xdiff-algorithm=", choices(&["default", "minimal", "patience", "histogram"]))
}
/// magit:--gpg-sign (level 5; the description follows gpg.format)
fn gpg_sign() -> MenuEntry {
    opt("-S", "Sign using openpgp", "--gpg-sign=", Reader::GpgKey).level(5)
}
/// magit:--signoff (level 6)
fn signoff() -> MenuEntry {
    sw("+s", "Add Signed-off-by trailer", "--signoff").level(6)
}
/// The git variables of a remote (magit-remote.<remote>.*), shown by
/// magit-remote and magit-remote-configure.
fn remote_variables() -> Vec<MenuEntry> {
    vec![
        cmd("u", "remote.<remote>.url", "remote.<remote>.url"),
        cmd("U", "remote.<remote>.fetch", "remote.<remote>.fetch"),
        cmd("s", "remote.<remote>.pushurl", "remote.<remote>.pushurl"),
        cmd("S", "remote.<remote>.push", "remote.<remote>.push"),
        cmd("O", "remote.<remote>.tagOpt", "remote.<remote>.tagopt"),
        cmd("h", "remote.<remote>.followRemoteHEAD", "remote.<remote>.followremotehead"),
    ]
}
/// magit-cherry-pick:--mainline
fn mainline() -> MenuEntry {
    opt("-m", "Replay merge relative to parent", "--mainline=", Reader::Number { zero: false })
}
/// magit-toggle-margin, magit-cycle-margin-style, magit-toggle-margin-details
fn margin_toggle() -> MenuEntry {
    cmd("L", "Toggle visibility", "margin-toggle")
}
fn margin_cycle() -> MenuEntry {
    cmd("l", "Cycle style", "margin-cycle-style")
}
fn margin_details() -> MenuEntry {
    cmd("d", "Toggle details", "margin-toggle-details")
}

fn cmd(key: &str, desc: &str, command: &str) -> MenuEntry {
    MenuEntry { key: key.into(), desc: desc.into(), action: Action::Command(command.into()), level: 1, hidden: false }
}
fn sw(key: &str, desc: &str, switch: &str) -> MenuEntry {
    MenuEntry { key: key.into(), desc: desc.into(), action: Action::Switch(switch.into()), level: 1, hidden: false }
}
fn sub(key: &str, desc: &str, menu: &str) -> MenuEntry {
    MenuEntry { key: key.into(), desc: desc.into(), action: Action::Submenu(menu.into()), level: 1, hidden: false }
}

/// magit-diff argument switches shared by `d` and `D` (default level).
/// "--stat" has `:if magit-diff-argument-predicate`: it is offered by `d`
/// and by `D` in diff, revision and stash buffers, but not in the status
/// buffer.
fn diff_switches(stat: bool) -> Vec<MenuEntry> {
    let mut v = vec![
        // Limit arguments
        opt("-i", "Ignore submodules", "--ignore-submodules=", choices(&["none", "untracked", "dirty", "all"])),
        sw("-b", "Ignore whitespace changes", "--ignore-space-change"),
        sw("-w", "Ignore all whitespace", "--ignore-all-space"),
        // Context arguments
        opt("-U", "Context lines", "-U", Reader::Number { zero: true }),
        sw("-W", "Show surrounding functions", "--function-context"),
        // Tune arguments
        opt("-A", "Diff algorithm", "--diff-algorithm=", choices(&["default", "minimal", "patience", "histogram"])),
        opt("-X", "Diff merges", "--diff-merges=", choices(&["off", "first-parent", "combined", "dense-combined"])),
        opt("-M", "Detect renames", "-M", Reader::Similarity),
        sw("-x", "Disallow external diff drivers", "--no-ext-diff"),
    ];
    if stat {
        v.push(sw("-s", "Show stats", "--stat"));
    }
    v
}

/// magit-diff-refresh (D); STAT when the buffer is a diff buffer.
pub fn diff_refresh_menu(stat: bool) -> Menu {
    Menu {
        name: "diff-refresh-menu".into(),
        title: "Change arguments of diff buffers".into(),
        groups: vec![
            diff_switches(stat),
            vec![
                cmd("g", "buffer", "diff-refresh"),
                cmd("s", "buffer and set defaults", "transient-set-and-exit"),
                cmd("w", "buffer and save defaults", "transient-save-and-exit"),
            ],
        ],
    }
}

/// magit-log-infix-arguments (default level).
fn log_switches() -> Vec<MenuEntry> {
    vec![
        // Commit limiting
        opt("-n", "Limit number of commits", "-n", Reader::Number { zero: false }),
        opt("-A", "Limit to author", "--author=", Reader::String),
        opt("-F", "Search messages", "--grep=", Reader::String),
        opt("-G", "Search changes", "-G", Reader::String),
        opt("-S", "Search occurrences", "-S", Reader::String),
        opt("-L", "Trace line evolution", "-L", Reader::FileTrace),
        // History simplification
        sw("-D", "Simplify by decoration", "--simplify-by-decoration"),
        sw("-f", "Follow renames when showing single-file log", "--follow"),
        // Commit ordering
        switches("-o", "Order commits by", &["--topo-order", "--author-date-order", "--date-order"]),
        sw("-r", "Reverse order", "--reverse"),
        // Formatting
        sw("-g", "Show graph", "--graph"),
        sw("-c", "Show graph in color", "--color"),
        sw("-d", "Show refnames", "--decorate"),
        sw("-p", "Show diffs", "--patch"),
        sw("-s", "Show diffstats", "--stat"),
    ]
}

/// magit-log-refresh (L): with FULL (in magit-log-mode) the arguments of
/// magit-log-infix-arguments, elsewhere only the "Arguments" group.
pub fn log_refresh_menu(full: bool) -> Menu {
    let args = if full {
        log_switches()
    } else {
        vec![
            opt("-n", "Limit number of commits", "-n", Reader::Number { zero: false }),
            switches("-o", "Order commits by", &["--topo-order", "--author-date-order", "--date-order"]),
            sw("-g", "Show graph", "--graph"),
            sw("-c", "Show graph in color", "--color"),
            sw("-d", "Show refnames", "--decorate"),
        ]
    };
    Menu {
        name: "log-refresh-menu".into(),
        title: "Change arguments of log buffers".into(),
        groups: vec![
            args,
            vec![
                cmd("g", "buffer", "log-refresh"),
                cmd("s", "buffer and set defaults", "transient-set-and-exit"),
                cmd("w", "buffer and save defaults", "transient-save-and-exit"),
            ],
            vec![margin_toggle(), margin_cycle(), margin_details(), cmd("x", "Toggle shortstat", "margin-toggle-shortstat")],
        ],
    }
}

/// Repository state behind Magit's `:if` predicates that swap menu suffixes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RepoState {
    pub merge: bool,
    pub rebase: bool,
    /// cherry-pick or revert stopped (magit-sequencer-in-progress-p)
    pub sequencer: bool,
    pub am: bool,
    pub bisect: bool,
}

/// Pick the layout Magit would show: `<name>@progress` while the matching
/// operation is in progress, otherwise the normal menu.
pub fn resolve_menu(name: &str, st: RepoState) -> String {
    let busy = match name {
        "merge-menu" => st.merge,
        "rebase-menu" => st.rebase,
        "cherry-pick-menu" | "revert-menu" => st.sequencer,
        "patches-menu" => st.am,
        "bisect-menu" => st.bisect,
        _ => false,
    };
    if busy {
        format!("{name}@progress")
    } else {
        name.to_string()
    }
}

/// Build the menu registry — every menu mirrors a transient-define-prefix in Magit.
/// The `:value` of Magit's transient prefixes: the arguments enabled when
/// the menu is opened.
pub fn prefix_value(name: &str) -> &'static [&'static str] {
    match name {
        "commit-menu" => &["--verbose"],
        "remote-menu" => &["-f"],
        "shortlog-menu" => &["--numbered", "--summary"],
        "cherry-pick-menu" => &["--ff"],
        "revert-menu" => &["--edit"],
        "patches-menu" => &["--3way"],
        "rebase-menu" => &["--autostash"],
        "blame-menu" => &["-w"],
        "fetch-modules-menu" => &["--verbose", "--jobs=4"],
        "commit-autofixup-menu" => &["-vv"],
        _ => &[],
    }
}

/// The `:incompatible` arguments of Magit's transient prefixes: setting
/// one of a group unsets the others.
pub fn incompatible(name: &str) -> &'static [&'static [&'static str]] {
    match name.split('@').next().unwrap_or(name) {
        "pull-menu" => &[&["--ff-only", "--rebase=true", "--rebase=merges", "--rebase=interactive"]],
        "merge-menu" => &[&["--ff-only", "--no-ff"]],
        "log-menu" | "log-refresh-menu" => &[&["--merges", "--no-merges"]],
        "cherry-pick-menu" => &[&["--ff", "-x"]],
        "patch-menu" => &[&["--subject-prefix=", "--rfc"]],
        _ => &[],
    }
}

/// transient-infix-set of transient-argument: after the infix ENTRY of
/// MENU is set to VALUE (its argument with the value of an option), the
/// arguments incompatible with its argument or VALUE are unset in ACTIVE.
pub fn unset_incompatible(menu: &Menu, entry: &MenuEntry, value: &str, active: &mut BTreeMap<String, bool>) {
    let arg = match &entry.action {
        Action::Switch(a) | Action::Option { arg: a, .. } => a.as_str(),
        _ => value,
    };
    let mut incomp: Vec<&str> = Vec::new();
    let xs: &[&str] = if arg == value { &[arg] } else { &[arg, value] };
    for g in incompatible(&menu.name) {
        for x in xs {
            if g.contains(x) {
                incomp.extend(g.iter().filter(|y| *y != x));
            }
        }
    }
    if incomp.is_empty() {
        return;
    }
    let set: Vec<String> = active.iter().filter(|(_, on)| **on).map(|(k, _)| k.clone()).collect();
    for k in set {
        if k == value {
            continue;
        }
        let karg = menu.groups.iter().flatten().find_map(|e| match &e.action {
            Action::Switch(a) if *a == k => Some(a.clone()),
            Action::Option { arg, .. } if option_matches(arg, &k) => Some(arg.clone()),
            Action::Switches(args) if args.contains(&k) => Some(k.clone()),
            _ => None,
        });
        let karg = karg.unwrap_or_else(|| k.clone());
        if incomp.contains(&k.as_str()) || incomp.contains(&karg.as_str()) {
            active.remove(&k);
        }
    }
}

/// transient-common-commands "Value commands", shown after `C-x`.
pub const VALUE_COMMANDS: [(&str, &str); 5] = [
    ("C-x s", "Set"),
    ("C-x C-s", "Save"),
    ("C-x C-k", "Reset"),
    ("C-x p", "Previous value"),
    ("C-x n", "Next value"),
];

/// transient-common-commands "Sticky commands".
pub const STICKY_COMMANDS: [(&str, &str); 3] =
    [("C-g", "Quit prefix or transient"), ("C-q", "Quit transient stack"), ("C-z", "Suspend transient stack")];

/// transient-values-file: `~/.config/xagit/transient-values` (or under
/// $XDG_CONFIG_HOME).
pub fn default_values_file() -> Option<std::path::PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(x) => std::path::PathBuf::from(x),
        None => std::path::PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("xagit").join("transient-values"))
}

/// Read transient-values: one prefix per line, its key followed by its
/// arguments, separated by tabs. A missing file has no values.
pub fn read_values(path: &std::path::Path) -> BTreeMap<String, Vec<String>> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut it = l.split('\t');
            let key = it.next().unwrap_or_default().to_string();
            (key, it.filter(|a| !a.is_empty()).map(String::from).collect())
        })
        .collect()
}

/// transient-save-values: write VALUES, sorted by key (nothing is written
/// when there are no values and no file yet).
pub fn write_values(path: &std::path::Path, values: &BTreeMap<String, Vec<String>>) -> std::io::Result<()> {
    if values.is_empty() && !path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text: String = values
        .iter()
        .map(|(k, v)| std::iter::once(k.as_str()).chain(v.iter().map(String::as_str)).collect::<Vec<_>>().join("\t") + "\n")
        .collect();
    std::fs::write(path, text)
}

/// transient-history-limit: the values kept per key when saving.
pub const HISTORY_LIMIT: usize = 10;

/// transient-history: the values used by each prefix (latest first) and
/// the values read by each infix (latest first, keyed by its argument).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    pub prefixes: BTreeMap<String, Vec<Vec<String>>>,
    pub infixes: BTreeMap<String, Vec<String>>,
}

/// transient-history-file: next to the values file.
pub fn default_history_file() -> Option<std::path::PathBuf> {
    Some(default_values_file()?.with_file_name("transient-history"))
}

/// Read the history file: a line per value, "P", the prefix's key and the
/// arguments, or "I", the infix's key and the value, separated by tabs,
/// latest first. A missing or unreadable file has no history.
pub fn read_history(path: &std::path::Path) -> History {
    let mut h = History::default();
    let Ok(text) = std::fs::read_to_string(path) else {
        return h;
    };
    for line in text.lines() {
        let mut it = line.split('\t');
        match (it.next(), it.next()) {
            (Some("P"), Some(key)) => {
                let args = it.filter(|a| !a.is_empty()).map(String::from).collect();
                h.prefixes.entry(key.to_string()).or_default().push(args);
            }
            (Some("I"), Some(key)) => {
                if let Some(v) = it.next() {
                    h.infixes.entry(key.to_string()).or_default().push(v.to_string());
                }
            }
            _ => {}
        }
    }
    h
}

/// transient-save-history: write HISTORY sorted by key, each list without
/// duplicates and cut to HISTORY_LIMIT (nothing is written when there is
/// no history and no file yet). Values containing tabs or newlines are
/// skipped.
pub fn write_history(path: &std::path::Path, history: &History) -> std::io::Result<()> {
    if history.prefixes.is_empty() && history.infixes.is_empty() && !path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let ok = |s: &str| !s.contains(['\t', '\n']);
    let mut text = String::new();
    for (key, values) in &history.prefixes {
        let mut seen: Vec<&Vec<String>> = Vec::new();
        for v in values.iter().filter(|v| v.iter().all(|a| ok(a))) {
            if seen.len() == HISTORY_LIMIT {
                break;
            }
            if !seen.contains(&v) {
                seen.push(v);
                text.push_str(&std::iter::once("P").chain([key.as_str()]).chain(v.iter().map(String::as_str)).collect::<Vec<_>>().join("\t"));
                text.push('\n');
            }
        }
    }
    for (key, values) in &history.infixes {
        let mut seen: Vec<&String> = Vec::new();
        for v in values.iter().filter(|v| ok(v)) {
            if seen.len() == HISTORY_LIMIT {
                break;
            }
            if !seen.contains(&v) {
                seen.push(v);
                text.push_str(&format!("I\t{key}\t{v}\n"));
            }
        }
    }
    std::fs::write(path, text)
}

/// The Wiplog group of magit-log (:if-non-nil magit-wip-mode).
pub fn wip_log_group() -> Vec<MenuEntry> {
    vec![cmd("i", "index", "wip-log-index"), cmd("w", "worktree", "wip-log-worktree")]
}

/// magit-blame: "r" and "f" only in blob buffers (:if-nil
/// buffer-file-name), "m" only in file buffers not blamed or blamed
/// READ_ONLY, "q" and the Refresh group only while BLAMING.
pub fn blame_menu(blob: bool, blaming: bool, read_only: bool) -> Menu {
    let mut actions = vec![cmd("b", "Show commits adding lines", "blame-addition")];
    if blob {
        actions.push(cmd("r", "Show commits removing lines", "blame-removal"));
        actions.push(cmd("f", "Show last commits that still have lines", "blame-reverse"));
    } else if !blaming || read_only {
        actions.push(cmd("m", "Blame echo", "blame-echo"));
    }
    if blaming {
        actions.push(cmd("q", "Quit blaming", "blame-quit"));
    }
    let mut groups = vec![
        vec![
            sw("-w", "Ignore whitespace", "-w"),
            sw("-r", "Do not treat root commits as boundaries", "--root"),
            sw("-P", "Follow only first parent", "--first-parent"),
            opt("-M", "Detect lines moved or copied within a file", "-M", Reader::OptionalNumber),
            opt("-C", "Detect lines moved or copied between files", "-C", Reader::OptionalNumber),
        ],
        actions,
    ];
    if blaming {
        groups.push(vec![cmd("c", "Cycle style", "blame-cycle-style")]);
    }
    Menu { name: "blame-menu".into(), title: "Blame".into(), groups }
}

/// magit-file-dispatch (C-c M-g): the commands for the visited file or
/// blob (IN_FILE), else the file commands and navigation ("g" not in the
/// STATUS buffer). The blame commands as in `blame_menu` ("q" only while
/// BLAMING).
pub fn file_dispatch_menu(in_file: bool, blob: bool, blaming: bool, read_only: bool, status: bool) -> Menu {
    let groups = if in_file {
        let mut blame = vec![cmd("B", "Blame...", "blame-menu"), cmd("b", "Blame", "blame-addition")];
        if blob {
            blame.push(cmd("r", "...removal", "blame-removal"));
            blame.push(cmd("f", "...reverse", "blame-reverse"));
        } else if !blaming || read_only {
            blame.push(cmd("m", "Blame echo", "blame-echo"));
        }
        if blaming {
            blame.push(cmd("q", "Quit blame", "blame-quit"));
        }
        vec![
            vec![
                cmd("s", "Stage", "file-stage"),
                cmd("u", "Unstage", "file-unstage"),
                cmd(", x", "Untrack", "file-untrack"),
                cmd(", r", "Rename", "file-rename"),
                cmd(", k", "Delete", "file-delete"),
                cmd(", c", "Checkout", "file-checkout"),
            ],
            vec![cmd("D", "Diff...", "diff-menu"), cmd("d", "Diff", "diff-buffer-file")],
            vec![
                cmd("L", "Log...", "log-menu"),
                cmd("l", "Log", "log-buffer-file"),
                cmd("t", "Trace", "log-trace-definition"),
            ],
            blame,
            vec![
                cmd("p", "Prev blob", "blob-previous"),
                cmd("n", "Next blob", "blob-next"),
                cmd("v", "Goto blob", "find-file"),
                cmd("V", "Goto file", "blob-visit-file"),
                cmd("g", "Goto status", "status-here"),
                cmd("G", "Goto magit", "display-repository-buffer"),
            ],
            vec![cmd("c", "Commit", "commit-menu"), cmd("e", "Edit line", "edit-line-commit")],
        ]
    } else {
        vec![
            vec![
                cmd("s", "Stage", "stage-files"),
                cmd("u", "Unstage", "unstage-files"),
                cmd("x", "Untrack", "file-untrack"),
                cmd("r", "Rename", "file-rename"),
                cmd("k", "Delete", "file-delete"),
                cmd("c", "Checkout", "file-checkout"),
            ],
            vec![cmd("g", "Goto status", "status-here"), cmd("G", "Goto magit", "display-repository-buffer")]
                .into_iter()
                .filter(|e| !(status && e.key == "g"))
                .collect(),
        ]
    };
    Menu { name: "file-dispatch".into(), title: "File dispatch".into(), groups }
}

/// magit-show-refs; "r" (magit-refs-set-show-commit-count) only in the
/// refs buffer (:if-derived magit-refs-mode).
pub fn show_refs_menu(in_refs: bool) -> Menu {
    let mut actions = vec![
        cmd("y", "Show refs, comparing them with HEAD", "show-refs-head"),
        cmd("c", "Show refs, comparing them with current branch", "show-refs-current"),
        cmd("o", "Show refs, comparing them with other branch", "show-refs-other"),
    ];
    if in_refs {
        actions.push(cmd("r", "Show refs, changing commit count display", "refs-set-show-commit-count"));
    }
    Menu {
        name: "show-refs-menu".into(),
        title: "Show refs".into(),
        groups: vec![
            vec![
                opt("-c", "Contains", "--contains=", Reader::String),
                opt("-M", "Merged", "--merged=", Reader::String),
                sw("-m", "Merged to HEAD", "--merged"),
                opt("-N", "Not merged", "--no-merged=", Reader::String),
                sw("-n", "Not merged to HEAD", "--no-merged"),
                opt("-s", "Sort", "--sort=", choices(&["-committerdate", "-authordate", "committerdate", "authordate"])),
            ],
            actions,
        ],
    }
}

pub fn registry() -> BTreeMap<String, Menu> {
    let mut m = BTreeMap::new();

    // ------------------------------------------------ magit-dispatch (h/?)
    m.insert(
        "dispatch".into(),
        Menu {
            name: "dispatch".into(),
            title: "Magit dispatch".into(),
            groups: vec![
                vec![
                    cmd("A", "Apply", "cherry-pick-menu"),
                    cmd("b", "Branch", "branch-menu"),
                    cmd("B", "Bisect", "bisect-menu"),
                    cmd("c", "Commit", "commit-menu"),
                    cmd("C", "Clone", "clone"),
                    cmd("d", "Diff", "diff-menu"),
                    cmd("D", "Diff (change)", "diff-refresh-menu"),
                    cmd("e", "Ediff (dwim)", "ediff-dwim"),
                    cmd("E", "Ediff", "ediff-menu"),
                    cmd("f", "Fetch", "fetch-menu"),
                    cmd("F", "Pull", "pull-menu"),
                    cmd("h", "Help", "help"),
                    cmd("H", "Section info", "section-info"),
                ],
                vec![
                    cmd("i", "Ignore", "gitignore-menu"),
                    cmd("I", "Init", "init"),
                    cmd("j", "Jump to section", "jump"),
                    cmd("J", "Display buffer", "display-buffer"),
                    cmd("l", "Log", "log-menu"),
                    cmd("L", "Log (change)", "log-refresh-menu"),
                    cmd("m", "Merge", "merge-menu"),
                    cmd("M", "Remote", "remote-menu"),
                    cmd("o", "Submodule", "submodule-menu"),
                    cmd("O", "Subtree", "subtree-menu"),
                    cmd("P", "Push", "push-menu"),
                    cmd("q", "Bury buffer", "quit"),
                    cmd("Q", "Command", "git-command"),
                ],
                vec![
                    cmd("r", "Rebase", "rebase-menu"),
                    cmd("t", "Tag", "tag-menu"),
                    cmd("T", "Note", "notes-menu"),
                    cmd("V", "Revert", "revert-menu"),
                    cmd("w", "Apply patches", "patches-menu"),
                    cmd("W", "Format patches", "patch-menu"),
                    cmd("X", "Reset", "reset-menu"),
                    cmd("y", "Show Refs", "show-refs"),
                    cmd("Y", "Cherries", "cherries"),
                    cmd("z", "Stash", "stash-menu"),
                    cmd("Z", "Worktree", "worktree-menu"),
                    cmd("!", "Run", "run-menu"),
                ],
                vec![
                    cmd("a", "Apply", "apply"),
                    cmd("v", "Reverse", "reverse"),
                    cmd("k", "Discard", "discard"),
                    cmd("s", "Stage", "stage"),
                    cmd("u", "Unstage", "unstage"),
                    cmd("S", "Stage all", "stage-all"),
                    cmd("U", "Unstage all", "unstage-all"),
                ],
                vec![
                    cmd("g", "Refresh current buffer", "refresh"),
                    cmd("<tab>", "Toggle section at point", "toggle-section"),
                    cmd("<return>", "Visit thing at point", "visit"),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-commit (c)
    m.insert(
        "commit-menu".into(),
        Menu {
            name: "commit-menu".into(),
            title: "Create a new commit or replace an existing commit".into(),
            groups: vec![
                vec![
                    sw("-a", "Stage all modified and deleted files", "--all"),
                    sw("-e", "Allow empty commit", "--allow-empty"),
                    sw("-v", "Show diff of changes to be committed", "--verbose"),
                    sw("-n", "Disable hooks", "--no-verify"),
                    sw("-R", "Claim authorship and reset author date", "--reset-author"),
                    opt("-A", "Override the author", "--author=", Reader::String),
                    opt("-D", "Override the author date", "--date=", Reader::String).level(7),
                    gpg_sign(),
                    signoff(),
                    opt("-C", "Reuse commit message", "--reuse-message=", Reader::String),
                    opt("-c", "Reedit commit message", "--reedit-message=", Reader::String),
                ],
                // "Create"
                vec![cmd("c", "Commit", "commit-create")],
                // "Edit HEAD"
                vec![
                    cmd("e", "Extend", "commit-extend"),
                    cmd("a", "Amend", "commit-amend"),
                    cmd("w", "Reword", "commit-reword"),
                    cmd("d", "Reshelve", "commit-reshelve").level(0),
                ],
                // "Edit"
                vec![
                    cmd("f", "Fixup", "commit-fixup"),
                    cmd("s", "Squash", "commit-squash"),
                    cmd("A", "Alter", "commit-alter"),
                    cmd("n", "Augment", "commit-augment"),
                    cmd("W", "Revise", "commit-revise"),
                ],
                // "Edit and rebase"
                vec![
                    cmd("F", "Instant fixup", "commit-instant-fixup"),
                    cmd("S", "Instant squash", "commit-instant-squash"),
                    cmd("R", "Reword past", "rebase-reword-commit").level(0),
                ],
                // "Spread across commits"
                vec![
                    cmd("x", "Modified files", "commit-autofixup").level(6),
                    cmd("X", "Updated modules", "commit-absorb-modules").level(6),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-commit-autofixup (c C-u x)
    m.insert(
        "commit-autofixup-menu".into(),
        Menu {
            name: "commit-autofixup-menu".into(),
            title: "Spread staged or unstaged changes across recent commits".into(),
            groups: vec![
                vec![
                    opt("-c", "Diff context lines", "--context=", Reader::Number { zero: true }),
                    opt("-s", "Strictness", "--strict=", Reader::Number { zero: true }),
                    sw("-v", "Increase verbosity", "-vv"),
                ],
                vec![cmd("x", "Absorb", "commit-autofixup-run")],
            ],
        },
    );

    // ------------------------------------------------ magit-branch (b)
    m.insert(
        "branch-menu".into(),
        Menu {
            name: "branch-menu".into(),
            title: "Add, configure or remove a branch".into(),
            groups: vec![
                // magit-branch-direct-configure (Magit's default): the
                // variables of the current branch (the descriptions of the
                // git variable infixes are set when the menu is shown)
                vec![
                    cmd("d", "branch.<branch>.description", "branch.<branch>.description"),
                    cmd("u", "branch.<branch>.merge/remote", "branch.<branch>.merge/remote"),
                    cmd("r", "branch.<branch>.rebase", "branch.<branch>.rebase"),
                    cmd("p", "branch.<branch>.pushRemote", "branch.<branch>.pushRemote"),
                ],
                // Configure repository defaults
                vec![
                    cmd("R", "pull.rebase", "pull.rebase"),
                    cmd("P", "remote.pushDefault", "remote.pushDefault"),
                    cmd("B", "Update default branch", "update-default-branch"),
                ],
                // Arguments
                vec![
                    sw("-m", "Automatically merge conflicting local modifications", "--merge").level(6),
                    sw("-r", "Recurse submodules when checking out an existing branch", "--recurse-submodules").level(7),
                ],
                vec![
                    cmd("b", "branch/revision", "checkout"),
                    cmd("l", "local branch", "branch-checkout"),
                    cmd("o", "new orphan", "branch-orphan").level(6),
                    cmd("c", "new branch", "branch-and-checkout"),
                    cmd("s", "new spin-off", "branch-spinoff"),
                    cmd("w", "new worktree", "worktree-checkout").level(5),
                    cmd("n", "new branch", "branch-create"),
                    cmd("S", "new spin-out", "branch-spinout"),
                    cmd("W", "new worktree", "worktree-branch").level(5),
                    cmd("m", "rename", "branch-rename"),
                    cmd("x", "reset", "branch-reset"),
                    cmd("k", "delete", "branch-delete"),
                    cmd("C", "configure...", "branch-config-menu"),
                    cmd("h", "shelve", "branch-shelve").level(7),
                    cmd("H", "unshelve", "branch-unshelve").level(7),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-remote (M)
    m.insert(
        "remote-menu".into(),
        Menu {
            name: "remote-menu".into(),
            title: "Add, configure or remove a remote".into(),
            groups: vec![
                // Variables (:if magit-remote-direct-configure and the
                // current remote)
                remote_variables(),
                // Arguments for add
                vec![sw("-f", "Fetch after add", "-f")],
                // Actions
                vec![
                    cmd("a", "Add", "remote-add"),
                    cmd("r", "Rename", "remote-rename"),
                    cmd("k", "Remove", "remote-remove"),
                ],
                vec![
                    cmd("C", "Configure...", "remote-configure-menu"),
                    cmd("p", "Prune stale branches", "remote-prune"),
                    cmd("P", "Prune stale refspecs", "remote-prune-refspecs"),
                    cmd("z", "Unshallow remote", "remote-unshallow").level(7),
                ],
                vec![cmd("d u", "Update default branch", "update-default-branch")],
            ],
        },
    );
    // magit-remote-configure (with the scope of the remote read)
    m.insert(
        "remote-configure-menu".into(),
        Menu {
            name: "remote-configure-menu".into(),
            title: "Configure a remote".into(),
            groups: vec![remote_variables()],
        },
    );

    // ------------------------------------------------ magit-fetch (f)
    m.insert(
        "fetch-menu".into(),
        Menu {
            name: "fetch-menu".into(),
            title: "Fetch from another repository".into(),
            groups: vec![
                vec![
                    sw("-p", "Prune deleted branches", "--prune"),
                    sw("-t", "Fetch all tags", "--tags"),
                    sw("-u", "Fetch full history", "--unshallow").level(7),
                    sw("-F", "Force", "--force"),
                ],
                // Fetch from (the descriptions of p and u are set when
                // the menu is shown)
                vec![
                    cmd("p", "pushRemote", "fetch-from-pushremote"),
                    cmd("u", "upstream", "fetch-from-upstream"),
                    cmd("e", "elsewhere", "fetch-other"),
                    cmd("a", "all remotes", "fetch-all"),
                ],
                vec![
                    cmd("o", "another branch", "fetch-branch"),
                    cmd("r", "explicit refspec", "fetch-refspec"),
                    cmd("m", "submodules", "fetch-modules"),
                ],
                vec![cmd("C", "variables...", "branch-config-menu")],
            ],
        },
    );
    // magit-fetch-modules (with a prefix argument)
    m.insert(
        "fetch-modules-menu".into(),
        Menu {
            name: "fetch-modules-menu".into(),
            title: "Fetch all populated submodules".into(),
            groups: vec![
                vec![
                    sw("-v", "verbose", "--verbose"),
                    opt("-j", "number of jobs", "--jobs=", Reader::Number { zero: false }),
                ],
                vec![cmd("m", "fetch modules", "fetch-modules-run")],
            ],
        },
    );

    // ------------------------------------------------ magit-pull (F)
    m.insert(
        "pull-menu".into(),
        Menu {
            name: "pull-menu".into(),
            title: "Pull from another repository".into(),
            groups: vec![
                vec![
                    sw("-f", "Fast-forward only", "--ff-only"),
                    opt("-r", "Rebase local commits", "--rebase=", pick(&["true", "merges", "interactive", "false"])),
                    sw("-A", "Autostash", "--autostash").level(7),
                    sw("-F", "Force", "--force"),
                ],
                // Pull into BRANCH from (the descriptions of p and u are
                // set when the menu is shown)
                vec![
                    cmd("p", "pushRemote", "pull-from-pushremote"),
                    cmd("u", "@{upstream}", "pull-from-upstream"),
                    cmd("e", "elsewhere", "pull-branch"),
                ],
                // Pull into UPSTREAM from
                vec![cmd("U", "upstream's upstream", "pull-into-upstream").level(7)],
                vec![
                    cmd("r", "branch.<branch>.rebase", "branch.<branch>.rebase"),
                    cmd("C", "variables...", "branch-config-menu"),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-push (P)
    m.insert(
        "push-menu".into(),
        Menu {
            name: "push-menu".into(),
            title: "Push to another repository".into(),
            groups: vec![
                vec![
                    sw("-f", "Force with lease", "--force-with-lease"),
                    sw("-F", "Force", "--force"),
                    sw("-h", "Disable hooks", "--no-verify"),
                    sw("-n", "Dry run", "--dry-run"),
                    sw("-u", "Set upstream", "--set-upstream").level(5),
                    sw("-T", "Include all tags", "--tags"),
                    sw("-t", "Include related annotated tags", "--follow-tags"),
                    opt("-o", "Set push-options", "--push-option=", Reader::Multi(vec!["skip-ci".into(), "ci.skip".into()]))
                        .level(5),
                ],
                // Push BRANCH to (:if magit-get-current-branch; the
                // descriptions of p and u are set when the menu is shown)
                vec![
                    cmd("p", "pushRemote", "push-current-to-pushremote"),
                    cmd("u", "@{upstream}", "push-current-to-upstream"),
                    cmd("e", "elsewhere", "push-current"),
                ],
                vec![
                    cmd("o", "another branch", "push-other"),
                    cmd("r", "explicit refspecs", "push-refspecs"),
                    cmd("m", "matching branches", "push-matching"),
                ],
                vec![
                    cmd("T", "a tag", "push-tag"),
                    cmd("t", "all tags", "push-tags"),
                    cmd("n", "a note ref", "push-notes-ref").level(6),
                ],
                vec![cmd("C", "Set variables...", "branch-config-menu")],
            ],
        },
    );

    // ------------------------------------------------ magit-merge (m)
    m.insert(
        "merge-menu".into(),
        Menu {
            name: "merge-menu".into(),
            title: "Merge branches".into(),
            groups: vec![
                vec![
                    sw("-f", "Fast-forward only", "--ff-only"),
                    sw("-n", "No fast-forward", "--no-ff"),
                    merge_strategy("-s"),
                    strategy_option().level(5),
                    sw("-b", "Ignore changes in amount of whitespace", "-Xignore-space-change").level(5),
                    sw("-w", "Ignore whitespace when comparing lines", "-Xignore-all-space").level(5),
                    x_diff_algorithm("-A").level(5),
                    gpg_sign(),
                    signoff(),
                ],
                vec![
                    cmd("m", "Merge", "merge-plain"),
                    cmd("e", "Merge and edit message", "merge-editmsg"),
                    cmd("n", "Merge but don't commit", "merge-nocommit"),
                    cmd("a", "Absorb", "merge-absorb"),
                ],
                vec![
                    cmd("p", "Preview merge", "merge-preview"),
                    cmd("s", "Squash merge", "merge-squash"),
                    cmd("d", "Dissolve", "merge-dissolve"),
                ],
            ],
        },
    );
    // :if magit-merge-in-progress-p
    m.insert(
        "merge-menu@progress".into(),
        Menu {
            name: "merge-menu@progress".into(),
            title: "Merge in progress".into(),
            groups: vec![vec![
                cmd("m", "Commit merge", "merge-commit"),
                cmd("a", "Abort merge", "merge-abort"),
            ]],
        },
    );

    // ------------------------------------------------ magit-rebase (r)
    m.insert(
        "rebase-menu".into(),
        Menu {
            name: "rebase-menu".into(),
            title: "Transplant commits and/or modify existing commits".into(),
            groups: vec![
                vec![
                    sw("-k", "Keep empty commits", "--keep-empty"),
                    opt("-r", "Rebase merges", "--rebase-merges=", choices(&["no-rebase-cousins", "rebase-cousins"])),
                    sw("-u", "Update branches", "--update-refs"),
                    merge_strategy("-s").level(7),
                    strategy_option().level(7),
                    x_diff_algorithm("=X").level(7),
                    sw("-f", "Force rebase", "--force-rebase").level(7),
                    sw("-d", "Use author date as committer date", "--committer-date-is-author-date"),
                    sw("-t", "Use current time as author date", "--ignore-date"),
                    sw("-a", "Autosquash", "--autosquash"),
                    sw("-A", "Autostash", "--autostash"),
                    sw("-i", "Interactive", "--interactive"),
                    sw("-h", "Disable hooks", "--no-verify"),
                    opt("-x", "Run command after commits", "--exec=", Reader::String).level(7),
                    gpg_sign(),
                    signoff(),
                ],
                // Rebase BRANCH onto (:if magit-get-current-branch for p
                // and u; their descriptions are set when shown)
                vec![
                    cmd("p", "pushRemote", "rebase-onto-pushremote"),
                    cmd("u", "@{upstream}", "rebase-onto-upstream"),
                    cmd("e", "elsewhere", "rebase-branch"),
                ],
                vec![
                    cmd("i", "interactively", "rebase-interactive"),
                    cmd("s", "a subset", "rebase-subset"),
                ],
                vec![
                    cmd("m", "to modify a commit", "rebase-edit-commit"),
                    cmd("w", "to reword a commit", "rebase-reword-commit"),
                    cmd("k", "to remove a commit", "rebase-remove-commit"),
                    cmd("f", "to autosquash", "rebase-autosquash"),
                    cmd("t", "to change dates", "reshelve-since").level(6),
                ],
            ],
        },
    );
    // :if magit-rebase-in-progress-p
    m.insert(
        "rebase-menu@progress".into(),
        Menu {
            name: "rebase-menu@progress".into(),
            title: "Rebase in progress".into(),
            groups: vec![vec![
                cmd("r", "Continue", "rebase-continue"),
                cmd("s", "Skip", "rebase-skip"),
                cmd("e", "Edit", "rebase-edit-todo"),
                cmd("a", "Abort", "rebase-abort"),
            ]],
        },
    );

    // ------------------------------------------------ magit-reset (X)
    m.insert(
        "reset-menu".into(),
        Menu {
            name: "reset-menu".into(),
            title: "Reset the HEAD, index and/or worktree to a previous state".into(),
            groups: vec![
                // "Reset"
                vec![cmd("b", "branch", "branch-reset"), cmd("f", "file", "file-checkout")],
                // "Reset this"
                vec![
                    cmd("m", "mixed    (HEAD and index)", "reset-mixed"),
                    cmd("s", "soft     (HEAD only)", "reset-soft"),
                    cmd("h", "hard     (HEAD, index and worktree)", "reset-hard"),
                    cmd("k", "keep     (HEAD and index, keeping uncommitted)", "reset-keep"),
                    cmd("i", "index    (only)", "reset-index"),
                    cmd("w", "worktree (only)", "reset-worktree"),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-tag (t)
    m.insert(
        "tag-menu".into(),
        Menu {
            name: "tag-menu".into(),
            title: "Create or delete a tag".into(),
            groups: vec![
                vec![
                    sw("-f", "Force", "--force"),
                    sw("-e", "Edit message", "--edit"),
                    sw("-a", "Annotate", "--annotate"),
                    sw("-s", "Sign", "--sign"),
                    opt("-u", "Sign as", "--local-user=", Reader::GpgKey),
                ],
                // "Create"
                vec![cmd("t", "tag", "tag-create"), cmd("r", "release", "tag-release")],
                // "Do"
                vec![cmd("k", "delete", "tag-delete"), cmd("p", "prune", "tag-prune")],
            ],
        },
    );

    // ------------------------------------------------ magit-stash (z)
    m.insert(
        "stash-menu".into(),
        Menu {
            name: "stash-menu".into(),
            title: "Stash uncommitted changes".into(),
            groups: vec![
                vec![
                    sw("-u", "Also save untracked files", "--include-untracked"),
                    sw("-a", "Also save untracked and ignored files", "--all"),
                ],
                // "Stash"
                vec![
                    cmd("z", "both", "stash-both"),
                    cmd("i", "index", "stash-index"),
                    cmd("w", "worktree", "stash-worktree"),
                    cmd("x", "keeping index", "stash-keep-index"),
                    cmd("P", "push", "stash-push-menu").level(5),
                ],
                // "Snapshot"
                vec![
                    cmd("Z", "both", "snapshot-both"),
                    cmd("I", "index", "snapshot-index"),
                    cmd("W", "worktree", "snapshot-worktree"),
                    cmd("r", "to wip ref", "wip-commit"),
                ],
                // "Use"
                vec![cmd("a", "Apply", "stash-apply"), cmd("p", "Pop", "stash-pop"), cmd("k", "Drop", "stash-drop")],
                // "Inspect"
                vec![cmd("l", "List", "stash-list"), cmd("v", "Show", "stash-show")],
                // "Transform"
                vec![
                    cmd("b", "Branch", "stash-branch"),
                    cmd("B", "Branch here", "stash-branch-here"),
                    cmd("f", "Format patch", "stash-format-patch"),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-stash-push (z P)
    m.insert(
        "stash-push-menu".into(),
        Menu {
            name: "stash-push-menu".into(),
            title: "Create stash using \"git stash push\"".into(),
            groups: vec![
                vec![
                    opt("--", "Limit to files", "-- ", Reader::Files),
                    sw("-u", "Also save untracked files", "--include-untracked"),
                    sw("-a", "Also save untracked and ignored files", "--all"),
                    sw("-k", "Keep index", "--keep-index"),
                    sw("-K", "Don't keep index", "--no-keep-index"),
                ],
                vec![cmd("P", "push", "stash-push")],
            ],
        },
    );

    // ------------------------------------------------ magit-gitignore (i)
    m.insert(
        "gitignore-menu".into(),
        Menu {
            name: "gitignore-menu".into(),
            title: "Instruct Git to ignore a file or pattern".into(),
            groups: vec![
                // "Gitignore"
                vec![
                    cmd("t", "shared at toplevel (.gitignore)", "ignore"),
                    cmd("s", "shared in subdirectory (path/to/.gitignore)", "ignore-subdir"),
                    cmd("p", "privately (.git/info/exclude)", "ignore-gitdir"),
                    cmd("g", "privately for all repositories (core.excludesfile is not set)", "ignore-system"),
                ],
                // "Skip worktree"
                vec![
                    cmd("w", "do skip worktree", "skip-worktree").level(7),
                    cmd("W", "do not skip worktree", "no-skip-worktree").level(7),
                ],
                // "Assume unchanged"
                vec![
                    cmd("u", "do assume unchanged", "assume-unchanged").level(7),
                    cmd("U", "do not assume unchanged", "no-assume-unchanged").level(7),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-diff (d)
    m.insert(
        "diff-menu".into(),
        Menu {
            name: "diff-menu".into(),
            title: "Show changes between different versions".into(),
            groups: vec![
                diff_switches(true),
                vec![
                    cmd("d", "Dwim", "diff-dwim"),
                    cmd("r", "Diff range", "diff-range"),
                    cmd("u", "Diff unstaged", "diff-unstaged"),
                    cmd("s", "Diff staged", "diff-staged"),
                    cmd("w", "Diff worktree", "diff-worktree"),
                ],
                vec![
                    cmd("c", "Show commit", "show-commit"),
                    cmd("t", "Show stash", "stash-show"),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-diff-refresh (D)
    m.insert("diff-refresh-menu".into(), diff_refresh_menu(true));

    // ------------------------------------------------ magit-log (l)
    m.insert(
        "log-menu".into(),
        Menu {
            name: "log-menu".into(),
            title: "Show a commit or reference log".into(),
            groups: vec![
                log_switches(),
                vec![
                    cmd("l", "current", "log-current"),
                    cmd("o", "other", "log-other"),
                    cmd("u", "related", "log-related"),
                    cmd("L", "local branches", "log-branches"),
                    cmd("b", "all branches", "log-all-branches"),
                    cmd("a", "all references", "log-all"),
                ],
                vec![
                    cmd("r", "reflog current", "reflog-current"),
                    cmd("O", "reflog other", "reflog-other"),
                    cmd("H", "reflog HEAD", "reflog-head"),
                    sub("s", "shortlog", "shortlog-menu"),
                    // xagit extra (Magit: magit-log-buffer-file in the file dispatch)
                    cmd("f", "file at point", "log-change"),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-shortlog (l s)
    m.insert(
        "shortlog-menu".into(),
        Menu {
            name: "shortlog-menu".into(),
            title: "Show a history summary".into(),
            groups: vec![
                vec![
                    sw("-n", "Sort by number of commits", "--numbered"),
                    sw("-s", "Show commit count summary only", "--summary"),
                    sw("-e", "Show email addresses", "--email"),
                ],
                vec![cmd("s", "since", "shortlog-since"), cmd("r", "range", "shortlog-range")],
            ],
        },
    );

    // ------------------------------------------------ magit-log-refresh (L)
    m.insert("log-refresh-menu".into(), log_refresh_menu(true));

    // ------------------------------------------------ magit-margin-settings
    // (L in reflog, stashes and cherry buffers)
    m.insert(
        "margin-settings-menu".into(),
        Menu {
            name: "margin-settings-menu".into(),
            title: "Margin".into(),
            groups: vec![vec![margin_toggle(), margin_cycle(), margin_details()]],
        },
    );

    // ------------------------------------------------ magit-run (!)
    m.insert(
        "run-menu".into(),
        Menu {
            name: "run-menu".into(),
            title: "Run git or another command".into(),
            groups: vec![vec![
                cmd("!", "in repository root", "git-command"),
                cmd("p", "in working directory", "git-command-cwd"),
            ]],
        },
    );

    // ------------------------------------------------ magit-cherry-pick (A)
    m.insert(
        "cherry-pick-menu".into(),
        Menu {
            name: "cherry-pick-menu".into(),
            title: "Apply or transplant commits".into(),
            groups: vec![
                vec![
                    mainline(),
                    merge_strategy("=s"),
                    sw("-F", "Attempt fast-forward", "--ff"),
                    sw("-x", "Reference cherry in commit message", "-x"),
                    sw("-e", "Edit commit messages", "--edit"),
                    gpg_sign(),
                    signoff(),
                ],
                vec![
                    cmd("A", "Pick", "cherry-pick"),
                    cmd("a", "Apply", "cherry-apply"),
                    cmd("h", "Harvest", "cherry-harvest"),
                    cmd("m", "Squash", "merge-squash"),
                ],
                vec![
                    cmd("d", "Donate", "cherry-donate"),
                    cmd("n", "Spinout", "cherry-spinout"),
                    cmd("s", "Spinoff", "cherry-spinoff"),
                ],
            ],
        },
    );
    // :if magit-sequencer-in-progress-p
    m.insert(
        "cherry-pick-menu@progress".into(),
        Menu {
            name: "cherry-pick-menu@progress".into(),
            title: "Cherry-pick in progress".into(),
            groups: vec![vec![
                cmd("A", "Continue", "sequencer-continue"),
                cmd("s", "Skip", "sequencer-skip"),
                cmd("a", "Abort", "sequencer-abort"),
            ]],
        },
    );

    // ------------------------------------------------ magit-revert (V)
    m.insert(
        "revert-menu".into(),
        Menu {
            name: "revert-menu".into(),
            title: "Revert existing commits, with or without creating new commits".into(),
            groups: vec![
                vec![
                    mainline(),
                    sw("-e", "Edit commit message", "--edit"),
                    sw("-E", "Don't edit commit message", "--no-edit"),
                    merge_strategy("=s"),
                    gpg_sign(),
                    signoff(),
                ],
                vec![
                    cmd("V", "Revert commit(s)", "revert"),
                    cmd("v", "Revert changes", "revert-no-commit"),
                ],
            ],
        },
    );
    // :if magit-sequencer-in-progress-p
    m.insert(
        "revert-menu@progress".into(),
        Menu {
            name: "revert-menu@progress".into(),
            title: "Revert in progress".into(),
            groups: vec![vec![
                cmd("V", "Continue", "sequencer-continue"),
                cmd("s", "Skip", "sequencer-skip"),
                cmd("a", "Abort", "sequencer-abort"),
            ]],
        },
    );

    // ------------------------------------------------ magit-bisect (B)
    m.insert(
        "bisect-menu".into(),
        Menu {
            name: "bisect-menu".into(),
            title: "Bisect".into(),
            groups: vec![vec![
                cmd("B", "Start", "bisect-start"),
                cmd("s", "Start script", "bisect-start-run"),
            ]],
        },
    );
    // :if magit-bisect-in-progress-p
    m.insert(
        "bisect-menu@progress".into(),
        Menu {
            name: "bisect-menu@progress".into(),
            title: "Bisect in progress".into(),
            groups: vec![
                vec![
                    cmd("B", "Bad", "bisect-bad"),
                    cmd("g", "Good", "bisect-good"),
                    cmd("k", "Skip", "bisect-skip"),
                    cmd("r", "Reset", "bisect-reset"),
                    cmd("s", "Run script", "bisect-run"),
                ],
                // xagit extra: Magit shows the log in the status buffer
                vec![cmd("l", "Log", "bisect-log")],
            ],
        },
    );

    // ------------------------------------------------ magit-worktree (Z)
    m.insert(
        "worktree-menu".into(),
        Menu {
            name: "worktree-menu".into(),
            title: "Worktree".into(),
            groups: vec![
                vec![
                    cmd("b", "Checkout", "worktree-checkout"),
                    cmd("c", "Branch", "worktree-branch"),
                    cmd("m", "Move", "worktree-move"),
                    cmd("k", "Delete", "worktree-delete"),
                ],
                vec![
                    cmd("p", "Prune", "worktree-prune"),
                    cmd("l", "List", "worktree-list"),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-blame (B in
    // file and blob buffers)
    m.insert("blame-menu".into(), blame_menu(true, true, true));
    m.insert("file-dispatch".into(), file_dispatch_menu(true, true, false, true, false));
    m.insert("show-refs-menu".into(), show_refs_menu(true));

    // ------------------------------------------------ magit-submodule (o)
    m.insert(
        "submodule-menu".into(),
        Menu {
            name: "submodule-menu".into(),
            title: "Submodule".into(),
            groups: vec![
                vec![
                    sw("-f", "Force", "--force"),
                    sw("-r", "Recursive", "--recursive"),
                    sw("-N", "Do not fetch", "--no-fetch"),
                    sw("-C", "Checkout tip", "--checkout"),
                    sw("-R", "Rebase onto tip", "--rebase"),
                    sw("-M", "Merge tip", "--merge"),
                    sw("-U", "Use upstream tip", "--remote"),
                ],
                vec![
                    cmd("a", "Add", "submodule-add"),
                    cmd("r", "Register", "submodule-register"),
                    cmd("p", "Populate", "submodule-populate"),
                    cmd("u", "Update", "submodule-update"),
                    cmd("s", "Synchronize", "submodule-sync"),
                    cmd("d", "Unpopulate", "submodule-unpopulate"),
                    cmd("k", "Remove", "submodule-remove"),
                ],
                vec![
                    cmd("l", "List modules", "submodule-list"),
                    cmd("f", "Fetch modules", "fetch-modules"),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-notes (T)
    m.insert(
        "notes-menu".into(),
        Menu {
            name: "notes-menu".into(),
            title: "Notes".into(),
            groups: vec![vec![
                cmd("T", "Edit", "notes-edit"),
                cmd("s", "Show", "notes-show"),
                cmd("r", "Remove", "notes-remove"),
                cmd("l", "List", "notes-list"),
                cmd("p", "Prune", "notes-prune"),
            ]],
        },
    );

    // ------------------------------------------------ magit-ediff (E)
    // No Emacs ediff session in a terminal: show the corresponding diff text
    // in the existing text buffer instead.
    m.insert(
        "ediff-menu".into(),
        Menu {
            name: "ediff-menu".into(),
            title: "Ediff (text diff view)".into(),
            groups: vec![
                vec![cmd("E", "Dwim", "ediff-dwim")],
                vec![
                    cmd("u", "Show unstaged", "ediff-unstaged"),
                    cmd("i", "Show staged", "ediff-staged"),
                    cmd("w", "Show worktree", "ediff-worktree"),
                ],
                vec![
                    cmd("c", "Show commit", "ediff-commit"),
                    cmd("r", "Show range", "ediff-range"),
                    cmd("z", "Show stash", "ediff-stash"),
                ],
            ],
        },
    );

    // ------------------------------------------------ magit-subtree (O)
    m.insert(
        "subtree-menu".into(),
        Menu {
            name: "subtree-menu".into(),
            title: "Subtree".into(),
            groups: vec![
                vec![
                    cmd("a", "Add", "subtree-add"),
                    cmd("P", "Pull", "subtree-pull"),
                    cmd("p", "Push", "subtree-push"),
                    cmd("s", "Split", "subtree-split"),
                ],
                vec![cmd("l", "List prefixes", "subtree-prefixes")],
            ],
        },
    );

    // ------------------------------------------------ magit-am (w)
    m.insert(
        "patches-menu".into(),
        Menu {
            name: "patches-menu".into(),
            title: "Apply patches".into(),
            groups: vec![
                vec![
                    sw("-3", "Fall back on 3way merge", "--3way"),
                    sw("-R", "Reject only failed hunks", "--reject"),
                    opt("-p", "Remove leading slashes from paths", "-p", Reader::Number { zero: false }),
                    sw("-c", "Remove text before scissors line", "--scissors"),
                    sw("-k", "Inhibit removal of email cruft", "--keep"),
                    sw("-b", "Limit removal of email cruft", "--keep-non-patch"),
                    sw("-d", "Use author date as committer date", "--committer-date-is-author-date"),
                    sw("-t", "Use current time as author date", "--ignore-date"),
                    gpg_sign(),
                    signoff(),
                ],
                vec![
                    cmd("m", "maildir", "patches-apply-maildir"),
                    cmd("w", "patches", "patches-apply"),
                    cmd("a", "plain patch", "patches-apply-worktree"),
                ],
            ],
        },
    );
    // :if magit-am-in-progress-p
    m.insert(
        "patches-menu@progress".into(),
        Menu {
            name: "patches-menu@progress".into(),
            title: "Applying patches".into(),
            groups: vec![
                vec![
                    cmd("w", "Continue", "patches-continue"),
                    cmd("s", "Skip", "patches-skip"),
                    cmd("a", "Abort", "patches-abort"),
                ],
                // xagit extra
                vec![cmd("l", "Show current patch", "patches-show")],
            ],
        },
    );

    // ------------------------------------------------ magit-patch (W)
    m.insert(
        "patch-menu".into(),
        Menu {
            name: "patch-menu".into(),
            title: "Patches".into(),
            groups: vec![vec![
                cmd("c", "Create patches", "patches-format"),
                cmd("w", "Apply patches", "patches-menu"),
                cmd("a", "Apply plain patch", "patches-apply-worktree"),
                cmd("s", "Save diff as patch", "patch-save"),
                cmd("r", "Request pull", "request-pull"),
            ]],
        },
    );

    // ------------------------------------------------ magit-branch-configure (b C)
    m.insert(
        "branch-config-menu".into(),
        Menu {
            name: "branch-config-menu".into(),
            title: "Configure branch".into(),
            groups: vec![
                vec![
                    cmd("d", "branch.<branch>.description", "branch.<branch>.description"),
                    cmd("u", "branch.<branch>.merge/remote", "branch.<branch>.merge/remote"),
                    cmd("r", "branch.<branch>.rebase", "branch.<branch>.rebase"),
                    cmd("p", "branch.<branch>.pushRemote", "branch.<branch>.pushRemote"),
                ],
                // Configure repository defaults
                vec![
                    cmd("R", "pull.rebase", "pull.rebase"),
                    cmd("P", "remote.pushDefault", "remote.pushDefault"),
                    cmd("B", "Update default branch", "update-default-branch"),
                ],
                // Configure branch creation
                vec![
                    cmd("a m", "branch.autoSetupMerge", "branch.autoSetupMerge"),
                    cmd("a r", "branch.autoSetupRebase", "branch.autoSetupRebase"),
                ],
            ],
        },
    );

    m
}
