//! User options (the defcustoms of Magit), read from
//! `~/.config/ngit/options`. Each line is an option name followed by its
//! value, `#` or `;` starting a comment:
//!
//! ```text
//! magit-no-confirm discard drop-stashes set-and-push
//! magit-no-confirm t
//! magit-refs-filter-alist ^refs/pullreqs/ nil
//! magit-refs-filter-alist ^origin/pr- nil
//! magit-visit-ref-behavior create-branch checkout-branch
//! magit-published-branches origin/master origin/main
//! ```
//!
//! magit-no-confirm and magit-visit-ref-behavior take a list of symbols
//! (or `t` / `nil`), magit-published-branches a list of branches (or
//! `nil`), a line setting the whole list. Each
//! magit-refs-filter-alist line adds an entry: a regexp (Emacs syntax,
//! without whitespace) and `t` (show) or `nil` (omit).

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
    /// magit-refs-filter-alist
    pub refs_filter: Vec<(Regexp, bool)>,
    /// magit-visit-ref-behavior
    pub visit_ref_behavior: Vec<String>,
    /// magit-published-branches
    pub published_branches: Vec<String>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            no_confirm: Some(vec!["set-and-push".into()]),
            refs_filter: Vec::new(),
            visit_ref_behavior: Vec::new(),
            published_branches: vec!["origin/master".into()],
        }
    }
}

impl Options {
    /// Whether ACTION isn't confirmed (magit-confirm). The action `t`
    /// is always confirmed; safe-with-wip needs magit-wip-mode, which
    /// ngit doesn't have.
    pub fn no_confirm(&self, action: &str) -> bool {
        action != "t" && self.no_confirm.as_ref().is_none_or(|l| l.iter().any(|a| a == action))
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
                "magit-no-confirm" => match values.as_slice() {
                    ["t"] => {
                        o.no_confirm = None;
                        None
                    }
                    ["nil"] => {
                        o.no_confirm = Some(Vec::new());
                        None
                    }
                    vs => match vs.iter().find(|v| !CONFIRM_ACTIONS.contains(v)) {
                        Some(v) => Some(format!("unknown action {v}")),
                        None => {
                            o.no_confirm = Some(vs.iter().map(|v| v.to_string()).collect());
                            None
                        }
                    },
                },
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

/// `~/.config/ngit/options`
pub fn default_options_file() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join(".config/ngit/options"))
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
    }
}
