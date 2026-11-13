//! ui/app/buffers.rs — branch operations, buffer switching and visiting things.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

impl App {
    /// magit-region-values of the ref sections of KINDS (at least two if
    /// MULTIPLE), if the region selects such sections only.
    pub(super) fn region_refs(&self, kinds: &[RefKind], multiple: bool) -> Option<Vec<String>> {
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
    pub(super) fn branch_merged(&self, branch: &str) -> bool {
        match self.git.current_branch() {
            Some(c) => self.branch_merged_into(branch, Some(&c)),
            None => false,
        }
    }

    /// magit-branch-merged-p with TARGET; None: into any other local
    /// branch.
    pub(super) fn branch_merged_into(&self, branch: &str, target: Option<&str>) -> bool {
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
    pub(super) fn upstream_branch(&self, branch: &str) -> Option<String> {
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
    pub(super) fn indirect_upstream_branch(&self, branch: &str, force: bool) -> Option<String> {
        let remote = self.git.git_string(&["config", &format!("branch.{branch}.remote")]).ok()?;
        if remote.is_empty() || remote == "." {
            return None;
        }
        let opted_in = |p: &String| {
            if self.git.git_ok(&["check-ref-format", "--branch", p]) {
                p == branch
            } else {
                crate::ui::regexp::Regexp::new(p).is_ok_and(|re| re.is_match(branch))
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
    pub(super) fn is_branch(&self, name: &str) -> bool {
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
    pub(super) fn branch_maybe_adjust_upstream(&self, branch: &str, start: &str) {
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
    pub(super) fn branch_prefer_other_default(&self) -> Option<String> {
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
    pub(super) fn worktree_delete(&mut self) -> Result<()> {
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
                crate::ui::trash::move_to_trash(&dir, self.options.trash_directory.as_deref())
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
    pub(super) fn switch_repository(&mut self, git: Git) {
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

    /// After `init` / `clone` created a repository, show it when xagit was
    /// started outside any repository.  From inside a repository the
    /// current buffer stays in front (init/clone are just commands there).
    pub(super) fn open_created_repository(&mut self, dir: &std::path::Path) {
        if !self.started_outside_repo {
            return;
        }
        if let Ok(git) = Git::discover(dir) {
            self.started_outside_repo = false;
            self.switch_repository(git);
            self.head_label.clear();
            self.refresh();
        }
    }

    /// magit-branch-reset: reset BRANCH (read, the local branch at point
    /// or the current one the default) to a revision (the branch or
    /// commit at point, else its upstream, the default). The current
    /// branch is reset hard, uncommitted changes have to be confirmed
    /// (yes-or-no-p); another one with update-ref. With a prefix
    /// argument a branch reset to becomes the upstream.
    pub(super) fn branch_reset(&mut self) -> Result<()> {
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
    pub(super) fn branch_delete(&mut self) -> Result<()> {
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
    pub(super) fn visit_ref(&mut self, kind: RefKind, name: &str) -> Result<()> {
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
    pub(super) fn read_char_choice(&mut self, prompt: &str, choices: &[char]) -> Result<Option<char>> {
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
    pub(super) fn branch_unset_push_remote(&self, branch: &str) {
        let _ = self.git.git_ok(&["config", "--unset", &format!("branch.{branch}.pushRemote")]);
    }

    /// magit-branch-maybe-delete-pr-remote: the remote of a pull-request
    /// branch (branch.BRANCH.pullRequestRemote) that only fetches single
    /// branches loses the refspec of BRANCH (that of its push branch, else
    /// its upstream); a remote left with nothing to fetch is removed if
    /// confirmed.
    pub(super) fn branch_maybe_delete_pr_remote(&mut self, branch: &str) -> Result<()> {
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
    pub(super) fn delete_remote_tracking_branches(&mut self, branches: &[String], fulls: &[String], force: bool) -> Result<()> {
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
    pub(super) fn which_function(&self) -> Option<String> {
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
    pub(super) fn edit_line_commit(&mut self) -> Result<()> {
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
            crate::ui::blame::run_blame_parsed(&self.git, r, &file, &range, reverse, typ)?.into_iter().next()
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
    pub(super) fn buffer_name_of(&self, mode: &BufferMode) -> String {
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
            BufferMode::Commit => "xagit-commit-msg",
            BufferMode::TextEdit => "xagit-edit",
            BufferMode::Resolve => "magit-resolve",
            BufferMode::File { rev, file, .. } => return crate::ui::blame::buffer_name(rev, file),
        };
        format!("{m}: {repo}")
    }

    /// magit-display-repository-buffer (J, and G of magit-file-dispatch)
    pub(super) fn display_repository_buffer(&mut self) -> Result<()> {
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
    pub(super) fn raise_buffer(&mut self, b: SavedBuffer) {
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
    pub(super) fn switch_to_mode(&mut self, mode: BufferMode) {
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
    pub(super) fn goto_file_position(&mut self, file: &str, line: usize, groups: &[&str]) {
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
    pub(super) fn file_key(&mut self, key: &Key) -> Result<bool> {
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
    pub(super) fn visit_thing(&mut self) -> Result<()> {
        self.visit(false)
    }

    /// RET, or C-RET / C-j (magit-diff-visit-worktree-file) when WORKTREE.
    pub(super) fn visit(&mut self, worktree: bool) -> Result<()> {
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
    pub(super) fn show_commit(&mut self, rev: &str) {
        let args = match (&self.transient_diff_args, &self.mode) {
            (Some(a), _) => a.clone(),
            (None, BufferMode::Revision { .. }) => self.buf_args.clone(),
            (None, _) => self.diff_args.clone(),
        };
        self.setup_buffer(BufferMode::Revision { rev: rev.to_string(), files: Vec::new() }, args);
    }

    /// magit-stash-show: the stash buffer of STASH (without "--stat").
    pub(super) fn show_stash(&mut self, stash: &str) {
        let args = match (&self.transient_diff_args, &self.mode) {
            (Some(a), _) => a,
            (None, BufferMode::Stash { .. }) => &self.buf_args,
            (None, _) => &self.diff_args,
        };
        let args: Vec<String> = args.iter().filter(|a| *a != "--stat").cloned().collect();
        self.setup_buffer(BufferMode::Stash { stash: stash.to_string() }, args);
    }

    /// Show a diff buffer (magit-diff-setup-buffer).
    pub(super) fn show_diff(&mut self, range: Option<String>, typearg: Option<&str>, typ: &str) {
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
    pub(super) fn show_or_scroll(&mut self, up: bool) -> Result<()> {
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
