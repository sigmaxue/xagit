//! User options (the defcustoms of Magit), read from
//! `~/.config/xagit/options`. Each line is an option name followed by its
//! value, `#` or `;` starting a comment:
//!
//! ```text
//! magit-no-confirm discard drop-stashes set-and-push
//! magit-no-confirm t
//! magit-slow-confirm discard drop-stashes
//! magit-refs-filter-alist ^refs/pullreqs/ nil
//! magit-refs-filter-alist ^origin/pr- nil
//! magit-visit-ref-behavior create-branch checkout-branch
//! magit-published-branches origin/master origin/main
//! transient-show-common-commands t
//! magit-delete-by-moving-to-trash nil
//! trash-directory ~/.trash
//! magit-wip-mode t
//! magit-wip-merge-branch nil
//! magit-wip-namespace refs/wip/
//! enable-command magit-clean
//! disabled-command-function nil
//! ```
//!
//! magit-no-confirm, magit-slow-confirm and magit-visit-ref-behavior take a list of symbols
//! (or `t` / `nil`), magit-published-branches a list of branches (or
//! `nil`), a line setting the whole list. Each
//! magit-refs-filter-alist line adds an entry: a regexp (Emacs syntax,
//! without whitespace) and `t` (show) or `nil` (omit). enable-command
//! and disable-command (Emacs' `(put COMMAND 'disabled nil|t)`) enable
//! or disable a command (magit-clean is disabled); with
//! disabled-command-function nil disabled commands just run.

use super::regexp::Regexp;

/// The actions of magit-no-confirm (magit--confirm-actions).
pub const CONFIRM_ACTIONS: &[&str] = &[
    "discard",
    "reverse",
    "stage-all-changes",
    "unstage-all-changes",
    "delete",
    "trash",
    "resurrect",
    "untrack",
    "rename",
    "reset-bisect",
    "abort-cherry-pick",
    "abort-revert",
    "abort-rebase",
    "abort-merge",
    "merge-dirty",
    "delete-unmerged-branch",
    "delete-branch-on-remote",
    "delete-pr-remote",
    "drop-stashes",
    "set-and-push",
    "amend-published",
    "rebase-published",
    "edit-published",
    "remove-modules",
    "remove-dirty-modules",
    "trash-module-gitdirs",
    "stash-apply-3way",
    "kill-process",
    "safe-with-wip",
];

/// The symbols of magit-visit-ref-behavior.
pub const VISIT_REF_BEHAVIORS: &[&str] = &["focus-on-ref", "create-branch", "checkout-any", "checkout-branch"];

#[derive(Debug, Clone)]
pub struct Options {
    /// magit-no-confirm: None is t (no action is confirmed)
    pub no_confirm: Option<Vec<String>>,
    /// magit-slow-confirm: None is t (all actions ask "yes or no")
    pub slow_confirm: Option<Vec<String>>,
    /// magit-refs-filter-alist
    pub refs_filter: Vec<(Regexp, bool)>,
    /// magit-visit-ref-behavior
    pub visit_ref_behavior: Vec<String>,
    /// magit-published-branches
    pub published_branches: Vec<String>,
    /// transient-show-common-commands
    pub show_common_commands: bool,
    /// magit-delete-by-moving-to-trash
    pub delete_by_moving_to_trash: bool,
    /// trash-directory: None is the system's trash
    pub trash_directory: Option<std::path::PathBuf>,
    /// magit-wip-mode
    pub wip_mode: bool,
    /// magit-wip-namespace and magit-wip-merge-branch
    pub wip: crate::git::wip::Wip,
    /// disabled-command-function: nil (false) runs disabled commands
    pub disabled_command_function: bool,
    /// the commands whose `disabled` property is set
    pub disabled_commands: Vec<String>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            no_confirm: Some(vec!["set-and-push".into()]),
            slow_confirm: Some(vec!["drop-stashes".into()]),
            refs_filter: Vec::new(),
            visit_ref_behavior: Vec::new(),
            published_branches: vec!["origin/master".into()],
            show_common_commands: false,
            delete_by_moving_to_trash: true,
            trash_directory: None,
            wip_mode: false,
            wip: crate::git::wip::Wip::default(),
            disabled_command_function: true,
            disabled_commands: vec!["magit-clean".into()],
        }
    }
}

impl Options {
    /// Whether ACTION isn't confirmed (magit-confirm). The action `t`
    /// is always confirmed; safe-with-wip covers discard, reverse,
    /// stage-all-changes and unstage-all-changes when magit-wip-mode is
    /// on (magit--no-confirm-alist).
    pub fn no_confirm(&self, action: &str) -> bool {
        let member = |a: &str| self.no_confirm.as_ref().is_none_or(|l| l.iter().any(|x| x == a));
        action != "t"
            && (member(action)
                || (self.wip_mode
                    && member("safe-with-wip")
                    && ["discard", "reverse", "stage-all-changes", "unstage-all-changes"].contains(&action)))
    }

    /// Whether running COMMAND calls disabled-command-function.
    pub fn disabled(&self, command: &str) -> bool {
        self.disabled_command_function && self.disabled_commands.iter().any(|c| c == command)
    }

    /// (put COMMAND 'disabled DISABLE)
    pub fn put_disabled(&mut self, command: &str, disable: bool) {
        self.disabled_commands.retain(|c| c != command);
        if disable {
            self.disabled_commands.push(command.to_string());
        }
    }

    /// Whether ACTION asks a "yes or no" question (magit-y-or-n-p).
    pub fn slow_confirm(&self, action: Option<&str>) -> bool {
        match &self.slow_confirm {
            None => true,
            Some(l) => action.is_some_and(|a| l.iter().any(|x| x == a)),
        }
    }

    /// magit-refs--insert-refname-p: the value of the first entry whose
    /// regexp matches REFNAME, else shown.
    pub fn show_ref(&self, refname: &str) -> bool {
        self.refs_filter.iter().find(|(re, _)| re.is_match(refname)).is_none_or(|(_, show)| *show)
    }

    /// Whether SYMBOL is a member of magit-visit-ref-behavior.
    pub fn visit_ref(&self, symbol: &str) -> bool {
        self.visit_ref_behavior.iter().any(|s| s == symbol)
    }

    /// Parse the options file TEXT over the defaults; the errors are
    /// "LINE: message" (the line is skipped).
    pub fn parse(text: &str) -> (Options, Vec<String>) {
        let mut o = Options::default();
        let mut errors = Vec::new();
        let mut filter_set = false;
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let mut words = line.split_whitespace();
            let name = words.next().unwrap_or_default();
            let values: Vec<&str> = words.collect();
            let err = match name {
                "magit-no-confirm" => actions(&values).map(|a| o.no_confirm = a).err(),
                "magit-slow-confirm" => actions(&values).map(|a| o.slow_confirm = a).err(),
                "magit-visit-ref-behavior" => {
                    let vs: &[&str] = if values == ["nil"] { &[] } else { &values };
                    match vs.iter().find(|v| !VISIT_REF_BEHAVIORS.contains(v)) {
                        Some(v) => Some(format!("unknown symbol {v}")),
                        None => {
                            o.visit_ref_behavior = vs.iter().map(|v| v.to_string()).collect();
                            None
                        }
                    }
                }
                "magit-published-branches" => {
                    let vs: &[&str] = if values == ["nil"] { &[] } else { &values };
                    o.published_branches = vs.iter().map(|v| v.to_string()).collect();
                    None
                }
                "transient-show-common-commands" => boolean(&values).map(|b| o.show_common_commands = b).err(),
                "magit-delete-by-moving-to-trash" => boolean(&values).map(|b| o.delete_by_moving_to_trash = b).err(),
                "trash-directory" => match values.as_slice() {
                    ["nil"] => {
                        o.trash_directory = None;
                        None
                    }
                    [d] => {
                        o.trash_directory = Some(expand_home(d));
                        None
                    }
                    _ => Some("expected a directory or nil".into()),
                },
                "magit-wip-mode" => boolean(&values).map(|b| o.wip_mode = b).err(),
                "magit-wip-merge-branch" => boolean(&values).map(|b| o.wip.merge_branch = b).err(),
                "magit-wip-namespace" => match values.as_slice() {
                    [ns] => {
                        o.wip.namespace = ns.to_string();
                        None
                    }
                    _ => Some("expected a string".into()),
                },
                "disabled-command-function" => boolean(&values).map(|b| o.disabled_command_function = b).err(),
                "enable-command" | "disable-command" => match values.as_slice() {
                    [c] => {
                        o.put_disabled(c, name == "disable-command");
                        None
                    }
                    _ => Some("expected a command".into()),
                },
                "magit-refs-filter-alist" => match values.as_slice() {
                    ["nil"] => {
                        o.refs_filter.clear();
                        filter_set = true;
                        None
                    }
                    [re, v @ ("t" | "nil")] => match Regexp::new(re) {
                        Ok(r) => {
                            if !filter_set {
                                o.refs_filter.clear();
                                filter_set = true;
                            }
                            o.refs_filter.push((r, *v == "t"));
                            None
                        }
                        Err(e) => Some(format!("invalid regexp {re}: {e}")),
                    },
                    _ => Some("expected a regexp and t or nil".into()),
                },
                _ => Some(format!("unknown option {name}")),
            };
            if let Some(e) = err {
                errors.push(format!("{}: {e}", n + 1));
            }
        }
        (o, errors)
    }
}

/// `t` or `nil`.
fn boolean(values: &[&str]) -> Result<bool, String> {
    match values {
        [v @ ("t" | "nil")] => Ok(*v == "t"),
        _ => Err("expected t or nil".into()),
    }
}

/// D with a leading `~/` expanded.
fn expand_home(d: &str) -> std::path::PathBuf {
    match (d.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => std::path::PathBuf::from(home).join(rest),
        _ => std::path::PathBuf::from(d),
    }
}

/// A list of magit--confirm-actions, `t` (None) or `nil`.
fn actions(values: &[&str]) -> Result<Option<Vec<String>>, String> {
    match values {
        ["t"] => Ok(None),
        ["nil"] => Ok(Some(Vec::new())),
        vs => match vs.iter().find(|v| !CONFIRM_ACTIONS.contains(v)) {
            Some(v) => Err(format!("unknown action {v}")),
            None => Ok(Some(vs.iter().map(|v| v.to_string()).collect())),
        },
    }
}

/// `~/.config/xagit/options`
pub fn default_options_file() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join(".config/xagit/options"))
}

#[cfg(test)]
mod tests {
    use super::Options;

    #[test]
    fn parse() {
        let (o, e) = Options::parse("");
        assert!(e.is_empty());
        assert!(o.no_confirm("set-and-push") && !o.no_confirm("discard") && o.show_ref("x"));
        let text = "# comment\nmagit-no-confirm discard drop-stashes\n\nmagit-refs-filter-alist ^origin/pr- nil\n\
                    magit-refs-filter-alist ^origin/ t\nmagit-refs-filter-alist /tmp nil\nmagit-visit-ref-behavior create-branch\n\
                    magit-no-confirm frobnicate\nmagit-foo 1\nmagit-refs-filter-alist \\( nil\n";
        let (o, e) = Options::parse(text);
        assert!(o.no_confirm("discard") && !o.no_confirm("set-and-push") && !o.no_confirm("t"));
        assert!(!o.show_ref("origin/pr-1") && o.show_ref("origin/tmp") && !o.show_ref("wip/tmp") && o.show_ref("master"));
        assert!(o.visit_ref("create-branch") && !o.visit_ref("checkout-any"));
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(e[0].starts_with("8: unknown action frobnicate"), "{e:?}");
        let (o, _) = Options::parse("magit-no-confirm t\nmagit-visit-ref-behavior nil\nmagit-published-branches nil");
        assert!(o.no_confirm("discard") && !o.no_confirm("t") && o.visit_ref_behavior.is_empty());
        assert!(o.published_branches.is_empty() && Options::default().published_branches == ["origin/master"]);
        let (o, _) = Options::parse("magit-published-branches origin/main upstream/main");
        assert_eq!(o.published_branches, ["origin/main", "upstream/main"]);
        assert!(!o.show_common_commands);
        let d = Options::default();
        assert!(d.slow_confirm(Some("drop-stashes")) && !d.slow_confirm(Some("discard")) && !d.slow_confirm(None));
        let (o, e) = Options::parse("magit-slow-confirm t");
        assert!(e.is_empty() && o.slow_confirm(None) && o.slow_confirm(Some("discard")));
        let (o, e) = Options::parse("magit-slow-confirm discard\nmagit-slow-confirm nope");
        assert!(o.slow_confirm(Some("discard")) && !o.slow_confirm(Some("drop-stashes")));
        assert_eq!(e, ["2: unknown action nope"]);
        assert!(d.delete_by_moving_to_trash && d.trash_directory.is_none());
        let (o, e) = Options::parse("magit-delete-by-moving-to-trash nil\ntrash-directory /tmp/t\ntrash-directory a b");
        assert!(!o.delete_by_moving_to_trash);
        assert_eq!(o.trash_directory.as_deref(), Some(std::path::Path::new("/tmp/t")));
        assert_eq!(e, ["3: expected a directory or nil"]);
        let (o, _) = Options::parse("magit-slow-confirm nil");
        assert!(!o.slow_confirm(Some("drop-stashes")));
        let (o, e) = Options::parse("transient-show-common-commands t\ntransient-show-common-commands 1");
        assert!(o.show_common_commands);
        assert_eq!(e, ["2: expected t or nil"]);
        // magit-wip
        assert!(!d.wip_mode && d.wip.namespace == "refs/wip/" && !d.wip.merge_branch);
        let (o, e) = Options::parse("magit-wip-mode t\nmagit-wip-merge-branch t\nmagit-wip-namespace refs/w/\nmagit-wip-namespace");
        assert!(o.wip_mode && o.wip.merge_branch && o.wip.namespace == "refs/w/");
        assert_eq!(e, ["4: expected a string"]);
        // safe-with-wip needs magit-wip-mode
        let (o, _) = Options::parse("magit-no-confirm safe-with-wip");
        assert!(!o.no_confirm("discard"));
        let (o, _) = Options::parse("magit-no-confirm safe-with-wip\nmagit-wip-mode t");
        assert!(o.no_confirm("discard") && o.no_confirm("unstage-all-changes") && !o.no_confirm("trash"));
        // disabled commands
        assert!(d.disabled("magit-clean") && !d.disabled("magit-status"));
        let (o, e) = Options::parse("enable-command magit-clean\ndisable-command\ndisable-command magit-status");
        assert!(!o.disabled("magit-clean") && o.disabled("magit-status"));
        assert_eq!(e, ["2: expected a command"]);
        let (o, _) = Options::parse("disabled-command-function nil");
        assert!(!o.disabled("magit-clean"));
    }
}
