//! Magit's git variable infixes (magit--git-variable and its subclasses
//! in magit-transient.el, magit--git-branch:upstream in magit-branch.el):
//! suffixes that show the value of a git variable of the scope of their
//! transient (a branch or a remote) and set it without leaving it.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum VarScope {
    None,
    Branch,
    Remote,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum VarChoices {
    List(&'static [&'static str]),
    /// magit-list-remotes
    Remotes,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum VarKind {
    /// magit--git-variable: one value, read when unset, unset when set
    Text,
    /// magit--git-variable with :multi-value: the values read (separated
    /// by commas, the current ones the initial input)
    Multi,
    /// magit--git-variable:urls: like Multi, set with `git remote set-url`
    Urls { push: bool },
    /// magit--git-variable:choices: cycled through, read with a prefix
    /// argument; FALLBACK and DEFAULT shown after them
    Choices { choices: VarChoices, fallback: Option<&'static str>, default: Option<&'static str> },
    /// magit--git-variable:boolean with :global t (magit-git-mergetool):
    /// the choices "true"/"false", shown with DEFAULT when unset
    Boolean { default: &'static str },
    /// magit--git-variable with :reader magit--read-mergetool and :global t:
    /// a value read from the tools of `git mergetool --tool-help', unset
    /// when set
    Mergetool,
    /// magit-branch.<branch>.description: a suffix editing it (exits)
    Description,
    /// magit--git-branch:upstream: branch.B.merge and branch.B.remote
    Upstream,
    /// magit-core.notesRef (GLOBAL: magit-global-core.notesRef): one
    /// value read with magit-notes-read-ref, unset when set
    NotesRef { global: bool },
    /// magit-notes.displayRef (GLOBAL: magit-global-notes.displayRef):
    /// the values read with magit-notes-read-refs
    NotesRefs { global: bool },
}

#[derive(Debug)]
pub(super) struct GitVar {
    /// the command (the Magit infix without "magit-")
    pub command: &'static str,
    /// the variable, "%s" standing for the scope
    pub variable: &'static str,
    pub scope: VarScope,
    pub kind: VarKind,
}

const REBASE: &[&str] = &["true", "merges", "interactive", "false"];

const VARS: &[GitVar] = &[
    GitVar {
        command: "branch.<branch>.description",
        variable: "branch.%s.description",
        scope: VarScope::Branch,
        kind: VarKind::Description,
    },
    GitVar {
        command: "branch.<branch>.merge/remote",
        variable: "branch.%s.merge",
        scope: VarScope::Branch,
        kind: VarKind::Upstream,
    },
    GitVar {
        command: "branch.<branch>.rebase",
        variable: "branch.%s.rebase",
        scope: VarScope::Branch,
        kind: VarKind::Choices { choices: VarChoices::List(REBASE), fallback: Some("pull.rebase"), default: Some("false") },
    },
    GitVar {
        command: "branch.<branch>.pushRemote",
        variable: "branch.%s.pushRemote",
        scope: VarScope::Branch,
        kind: VarKind::Choices { choices: VarChoices::Remotes, fallback: Some("remote.pushDefault"), default: None },
    },
    GitVar {
        command: "pull.rebase",
        variable: "pull.rebase",
        scope: VarScope::None,
        kind: VarKind::Choices { choices: VarChoices::List(REBASE), fallback: None, default: Some("false") },
    },
    GitVar {
        command: "remote.pushDefault",
        variable: "remote.pushDefault",
        scope: VarScope::None,
        kind: VarKind::Choices { choices: VarChoices::Remotes, fallback: None, default: None },
    },
    GitVar {
        command: "branch.autoSetupMerge",
        variable: "branch.autoSetupMerge",
        scope: VarScope::None,
        kind: VarKind::Choices {
            choices: VarChoices::List(&["always", "true", "false"]),
            fallback: None,
            default: Some("true"),
        },
    },
    GitVar {
        command: "branch.autoSetupRebase",
        variable: "branch.autoSetupRebase",
        scope: VarScope::None,
        kind: VarKind::Choices {
            choices: VarChoices::List(&["always", "local", "remote", "never"]),
            fallback: None,
            default: Some("never"),
        },
    },
    GitVar {
        command: "remote.<remote>.url",
        variable: "remote.%s.url",
        scope: VarScope::Remote,
        kind: VarKind::Urls { push: false },
    },
    GitVar { command: "remote.<remote>.fetch", variable: "remote.%s.fetch", scope: VarScope::Remote, kind: VarKind::Multi },
    GitVar {
        command: "remote.<remote>.pushurl",
        variable: "remote.%s.pushurl",
        scope: VarScope::Remote,
        kind: VarKind::Urls { push: true },
    },
    GitVar { command: "remote.<remote>.push", variable: "remote.%s.push", scope: VarScope::Remote, kind: VarKind::Text },
    GitVar {
        command: "remote.<remote>.tagopt",
        variable: "remote.%s.tagOpt",
        scope: VarScope::Remote,
        kind: VarKind::Choices { choices: VarChoices::List(&["--no-tags", "--tags"]), fallback: None, default: None },
    },
    GitVar {
        command: "remote.<remote>.followremotehead",
        variable: "remote.%s.followRemoteHEAD",
        scope: VarScope::Remote,
        kind: VarKind::Choices {
            choices: VarChoices::List(&["create", "always", "warn"]),
            fallback: None,
            default: Some("create"),
        },
    },
    GitVar { command: "core.notesRef", variable: "core.notesRef", scope: VarScope::None, kind: VarKind::NotesRef { global: false } },
    GitVar {
        command: "notes.displayRef",
        variable: "notes.displayRef",
        scope: VarScope::None,
        kind: VarKind::NotesRefs { global: false },
    },
    GitVar {
        command: "global-core.notesRef",
        variable: "core.notesRef",
        scope: VarScope::None,
        kind: VarKind::NotesRef { global: true },
    },
    GitVar {
        command: "global-notes.displayRef",
        variable: "notes.displayRef",
        scope: VarScope::None,
        kind: VarKind::NotesRefs { global: true },
    },
    // magit-git-mergetool's global git variables (magit-extras.el)
    GitVar { command: "merge.guitool", variable: "merge.guitool", scope: VarScope::None, kind: VarKind::Mergetool },
    GitVar { command: "merge.tool", variable: "merge.tool", scope: VarScope::None, kind: VarKind::Mergetool },
    GitVar {
        command: "mergetool.hideResolved",
        variable: "mergetool.hideResolved",
        scope: VarScope::None,
        kind: VarKind::Boolean { default: "false" },
    },
    GitVar {
        command: "mergetool.keepBackup",
        variable: "mergetool.keepBackup",
        scope: VarScope::None,
        kind: VarKind::Boolean { default: "true" },
    },
    GitVar {
        command: "mergetool.keepTemporaries",
        variable: "mergetool.keepTemporaries",
        scope: VarScope::None,
        kind: VarKind::Boolean { default: "false" },
    },
    GitVar {
        command: "mergetool.writeToTemp",
        variable: "mergetool.writeToTemp",
        scope: VarScope::None,
        kind: VarKind::Boolean { default: "false" },
    },
];

/// The location argument of V (--global or --local).
fn location(v: &GitVar) -> &'static str {
    match v.kind {
        VarKind::NotesRef { global: true } | VarKind::NotesRefs { global: true } | VarKind::Boolean { .. }
        | VarKind::Mergetool => "--global",
        _ => "--local",
    }
}

/// The git variable infix COMMAND.
pub(super) fn lookup(command: &str) -> Option<&'static GitVar> {
    VARS.iter().find(|v| v.command == command)
}

/// The scope of the transient NAME (transient-scope): the branch of
/// magit-branch, magit-pull and magit-branch-configure, the remote of
/// magit-remote and magit-remote-configure.
pub(super) fn menu_scope_kind(name: &str) -> VarScope {
    match name {
        "branch-menu" | "pull-menu" | "branch-config-menu" => VarScope::Branch,
        "remote-menu" | "remote-configure-menu" => VarScope::Remote,
        _ => VarScope::None,
    }
}

impl App {
    /// `git config ARG --get-all VAR` (ARG --local or --global).
    fn config_values(&self, arg: &str, var: &str) -> Vec<String> {
        self.git.git_lines(&["config", arg, "--get-all", var]).into_iter().filter(|v| !v.is_empty()).collect()
    }

    fn config_value(&self, arg: &str, var: &str) -> Option<String> {
        self.git.git_string(&["config", arg, var]).ok().map(|v| v.trim_end_matches('\n').to_string()).filter(|v| !v.is_empty())
    }

    /// The variable of V for SCOPE.
    fn var_name(v: &GitVar, scope: &str) -> String {
        v.variable.replace("%s", scope)
    }

    fn var_choices(&self, v: &GitVar) -> Vec<String> {
        match v.kind {
            VarKind::Choices { choices: VarChoices::List(l), .. } => l.iter().map(|s| s.to_string()).collect(),
            VarKind::Choices { choices: VarChoices::Remotes, .. } => self.git.remotes(),
            VarKind::Boolean { .. } => vec!["true".into(), "false".into()],
            _ => Vec::new(),
        }
    }

    /// magit--git-variable-list-choices: the value shown after the
    /// choices, the global value (global:V), the value of the fallback
    /// variable (FALLBACK:V), else the default (default:V). A variable
    /// whose location is global has no "global:" alternative.
    fn var_fallback(&self, v: &GitVar, var: &str) -> Option<String> {
        let (fallback, default) = match v.kind {
            VarKind::Choices { fallback, default, .. } => (fallback, default),
            VarKind::Boolean { default } => (None, Some(default)),
            _ => return None,
        };
        if location(v) != "--global" {
            if let Some(g) = self.config_value("--global", var) {
                return Some(format!("global:{g}"));
            }
        }
        if let Some(f) = fallback {
            if let Some(fv) = self.config(f) {
                return Some(format!("{f}:{fv}"));
            }
        }
        default.map(|d| format!("default:{d}"))
    }

    /// The description of the infix V in a transient of SCOPE
    /// (transient-format-description and transient-format-value): the
    /// variable and its value or "unset"; for choices those (the active
    /// one in parentheses) and the fallback.
    pub(super) fn var_description(&self, v: &GitVar, scope: &str) -> String {
        let var = Self::var_name(v, scope);
        match v.kind {
            VarKind::Upstream => {
                let get = |k: &str| self.config_value("--local", &format!("branch.{scope}.{k}"));
                let (remote, merge) = (get("remote"), get("merge"));
                let (remote, merge) = if remote.is_some() && merge.is_some() { (remote, merge) } else { (None, None) };
                format!(
                    "branch.{scope}.merge {}\n  branch.{scope}.remote {}",
                    merge.as_deref().unwrap_or("unset"),
                    remote.as_deref().unwrap_or("unset")
                )
            }
            VarKind::Multi | VarKind::Urls { .. } | VarKind::NotesRefs { .. } => match self.config_values(location(v), &var).as_slice() {
                [] => format!("{var} unset"),
                [one] => format!("{var} {one}"),
                many => format!("{var} {}", many.iter().map(|v| format!("\n     {v}")).collect::<String>()),
            },
            VarKind::Text | VarKind::Description | VarKind::NotesRef { .. } | VarKind::Mergetool => {
                let value = self.config_value(location(v), &var);
                let first = value.as_deref().and_then(|v| v.lines().next());
                format!("{var} {}", first.unwrap_or("unset"))
            }
            VarKind::Choices { .. } | VarKind::Boolean { .. } => {
                let value = self.config_value(location(v), &var);
                let mut choices = self.var_choices(v);
                if let Some(val) = value.as_ref().filter(|val| !choices.contains(val)) {
                    choices.insert(0, val.clone());
                }
                let mut shown: Vec<String> =
                    choices.into_iter().map(|c| if Some(&c) == value.as_ref() { format!("({c})") } else { c }).collect();
                if let Some(f) = self.var_fallback(v, &var) {
                    shown.push(if value.is_none() { format!("({f})") } else { f });
                }
                format!("{var} [{}]", shown.join("|"))
            }
        }
    }

    /// The scope of the transient NAME when it is shown: the current
    /// branch or remote, or the one read by magit-branch-configure and
    /// magit-remote-configure.
    pub(super) fn menu_scope(&self, name: &str) -> Option<String> {
        if let Some(s) = self.var_scopes.get(name) {
            return s.clone();
        }
        match menu_scope_kind(name) {
            VarScope::Branch => self.git.current_branch(),
            VarScope::Remote => self.current_remote(),
            VarScope::None => None,
        }
    }

    /// The descriptions of the git variable infixes of the menu NAME, and
    /// whether they are shown (those of a scope only with one).
    pub(super) fn var_suffix_states(&self, name: &str) -> Vec<(String, String, bool)> {
        let scope = self.menu_scope(name);
        let Some(menu) = self.menus.get(name) else {
            return Vec::new();
        };
        menu.groups
            .iter()
            .flatten()
            .filter_map(|e| match &e.action {
                Action::Command(c) => lookup(c).map(|v| (e.key.clone(), v)),
                _ => None,
            })
            .map(|(key, v)| match (v.scope, scope.as_deref()) {
                (VarScope::None, _) => (key, self.var_description(v, ""), true),
                (_, Some(s)) => (key, self.var_description(v, s), true),
                (_, None) => (key, String::new(), false),
            })
            .collect()
    }

    /// magit--read-branch-scope / magit--read-remote-scope with OBJ:
    /// "Set VAR for branch" with <name> for the scope.
    fn read_var_scope(&mut self, v: &GitVar) -> Result<Option<String>> {
        let prompt = format!("Set {} for {}", v.variable.replace("%s", "<name>"), match v.scope {
            VarScope::Remote => "remote",
            _ => "branch",
        });
        match v.scope {
            VarScope::Remote => self.read_remote(&prompt),
            _ => self.read_local_branch(&prompt),
        }
    }

    /// magit-read-local-branch: a local branch, the one at point the
    /// default.
    pub(super) fn read_local_branch(&mut self, prompt: &str) -> Result<Option<String>> {
        let default = self.local_branch_at_point();
        let table = Completion { candidates: self.git.branches(), require_match: true, multiple: false };
        Ok(self.completing_read(prompt, default.as_deref(), Hist::Revision, Some(table))?.filter(|b| !b.is_empty()))
    }

    /// Invoke the git variable infix V, from the transient MENU (none:
    /// from M-x, the scope read); whether it was set.
    pub(super) fn git_var_command(&mut self, v: &'static GitVar, menu: Option<&str>) -> Result<()> {
        let scope = match (v.scope, menu.and_then(|m| self.menu_scope(m))) {
            (VarScope::None, _) => String::new(),
            (_, Some(s)) => s,
            (_, None) => match self.read_var_scope(v)? {
                Some(s) => s,
                None => return Ok(()),
            },
        };
        let var = Self::var_name(v, &scope);
        let loc = location(v);
        let set_all = |s: &Self, values: &[String]| -> Result<()> {
            let _ = s.git.git_ok(&["config", loc, "--unset-all", &var]);
            for val in values {
                s.git.git_interactive(&["config", loc, "--add", &var, val])?;
            }
            Ok(())
        };
        match v.kind {
            // magit-branch.<branch>.description
            VarKind::Description => {
                let res = self.with_terminal(|g| {
                    g.git_with_editors(Editor::Accept, Editor::User, &["branch", "--edit-description", &scope])
                });
                if let Err(e) = res {
                    self.set_msg(&format!("{e:#}"));
                }
            }
            VarKind::Upstream => {
                let set = self.config_value("--local", &format!("branch.{scope}.remote")).is_some()
                    && self.config_value("--local", &format!("branch.{scope}.merge")).is_some();
                if set {
                    // magit-set-upstream-branch with nil
                    self.git.git_interactive(&["branch", "--unset-upstream", &scope])?;
                } else {
                    let Some(up) = self.read_upstream_branch(&scope, "Upstream")? else {
                        return Ok(());
                    };
                    self.git.git_interactive(&["branch", "--set-upstream-to", &up, &scope])?;
                }
            }
            VarKind::Text => match self.config_value(loc, &var) {
                Some(_) => {
                    let _ = self.git.git_ok(&["config", loc, "--unset", &var]);
                }
                None => {
                    let Some(val) = self.read_line_hist(&var, None, Hist::Infix(&var), None)? else {
                        return Ok(());
                    };
                    if !val.is_empty() {
                        add_to_history(self.infix_history.borrow_mut().entry(var.clone()).or_default(), &val, true);
                        self.git.git_interactive(&["config", loc, &var, &val])?;
                    }
                }
            },
            // magit-merge.guitool / magit-merge.tool: the tool is read
            // from `git mergetool --tool-help' (magit--read-mergetool).
            VarKind::Mergetool => match self.config_value(loc, &var) {
                Some(_) => {
                    let _ = self.git.git_ok(&["config", loc, "--unset", &var]);
                }
                None => {
                    let tools = super::run::mergetool_tools(&self.git);
                    let table = Completion { candidates: tools, require_match: true, multiple: false };
                    let prompt = format!("Set `{var}' to");
                    let Some(val) = self.completing_read(&prompt, None, Hist::Infix(&var), Some(table))? else {
                        return Ok(());
                    };
                    if !val.is_empty() {
                        self.git.git_interactive(&["config", loc, &var, &val])?;
                    }
                }
            },
            VarKind::Multi | VarKind::Urls { .. } => {
                let old = self.config_values("--local", &var);
                if !old.is_empty() {
                    *self.initial_input.borrow_mut() = Some(old.join(","));
                }
                // magit-remote.<remote>.url and .pushurl share a history
                let hist = if matches!(v.kind, VarKind::Urls { .. }) { "remote.*.url".to_string() } else { var.clone() };
                let table = Completion { candidates: Vec::new(), require_match: false, multiple: true };
                let Some(input) = self.read_line_hist(&var, None, Hist::Infix(&hist), Some(table))? else {
                    return Ok(());
                };
                let mut values: Vec<String> =
                    input.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
                if !values.is_empty() {
                    add_to_history(self.infix_history.borrow_mut().entry(hist).or_default(), &values.join(","), true);
                }
                if let VarKind::Urls { push } = v.kind {
                    let home = std::env::var("HOME").unwrap_or_default();
                    for u in &mut values {
                        if let Some(rest) = u.strip_prefix('~') {
                            *u = format!("{home}{rest}");
                        }
                    }
                    let flag = if push { vec!["--push"] } else { vec![] };
                    for u in values.iter().filter(|u| !old.contains(u)) {
                        let mut a = vec!["remote", "set-url"];
                        a.extend(&flag);
                        a.extend(["--add", &scope, u]);
                        self.git.git_interactive(&a)?;
                    }
                    for u in old.iter().filter(|u| !values.contains(u)) {
                        let re = format!("^{}$", regex_quote(u));
                        let mut a = vec!["remote", "set-url"];
                        a.extend(&flag);
                        a.extend(["--delete", &scope, &re]);
                        if let Err(e) = self.git.git_interactive(&a) {
                            self.set_msg(&format!("{e:#}"));
                        }
                    }
                } else {
                    set_all(self, &values)?;
                }
            }
            VarKind::NotesRef { global } => match self.config_value(location(v), &var) {
                Some(_) => {
                    let _ = self.git.git_ok(&["config", location(v), "--unset", &var]);
                }
                None => {
                    let prompt = format!("Set {} core.notesRef", if global { "global" } else { "local" });
                    let Some(r) = self.notes_read_ref(&prompt, &var)? else {
                        return Ok(());
                    };
                    self.git.git_interactive(&["config", location(v), &var, &r])?;
                }
            },
            VarKind::NotesRefs { global } => {
                let prompt = format!("Set {} notes.displayRef", if global { "global" } else { "local" });
                let Some(refs) = self.notes_read_refs(&prompt, &var)? else {
                    return Ok(());
                };
                let _ = self.git.git_ok(&["config", location(v), "--unset-all", &var]);
                for r in &refs {
                    self.git.git_interactive(&["config", location(v), "--add", &var, r])?;
                }
            }
            VarKind::Choices { .. } | VarKind::Boolean { .. } => {
                let value = self.config_value(loc, &var);
                let choices = self.var_choices(v);
                let new = if self.prefix_arg.is_some() {
                    let unset = self.var_fallback(v, &var).unwrap_or_else(|| "(unset)".into());
                    let mut candidates = choices.clone();
                    if let Some(val) = value.as_ref().filter(|val| !choices.contains(val)) {
                        candidates.insert(0, val.clone());
                    }
                    candidates.push(unset.clone());
                    let table = Completion { candidates, require_match: true, multiple: false };
                    let Some(c) = self.completing_read(&format!("Set `{var}' to"), None, Hist::Minibuffer, Some(table))?
                    else {
                        return Ok(());
                    };
                    (c != unset && !c.is_empty()).then_some(c)
                } else {
                    match value {
                        Some(val) => choices.iter().skip_while(|c| **c != val).nth(1).cloned(),
                        None => choices.first().cloned(),
                    }
                };
                match new {
                    Some(n) => self.git.git_interactive(&["config", loc, &var, &n])?,
                    None => {
                        let _ = self.git.git_ok(&["config", loc, "--unset", &var]);
                    }
                }
            }
        }
        // transient-infix-set: "Unset VAR" outside a transient
        if menu.is_none() && !matches!(v.kind, VarKind::Description | VarKind::Upstream) {
            let unset = match v.kind {
                VarKind::Multi | VarKind::Urls { .. } | VarKind::NotesRefs { .. } => self.config_values(location(v), &var).is_empty(),
                _ => self.config_value(location(v), &var).is_none(),
            };
            if unset {
                self.set_msg(&format!("Unset {var}"));
            }
        }
        self.refresh();
        Ok(())
    }
}
