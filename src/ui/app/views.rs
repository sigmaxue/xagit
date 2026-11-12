//! ui/app/views.rs — diff / log views and the commit-related commands.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

impl App {

    /// The diff commands of `d`: magit-diff-unstaged, -staged,
    /// -working-tree, -range, -dwim, magit-show-commit, magit-stash-show.
    pub(super) fn diff_view(&mut self, kind: &str) -> Result<()> {
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
    pub(super) fn dwim_range_default(&self) -> Option<String> {
        match self.dwim() {
            Dwim::Commit(v) => Some(format!("{v}^..{v}")),
            Dwim::Range(r) => Some(r),
            _ => self.git.current_branch(),
        }
    }

    /// magit-diff--dwim
    pub(super) fn dwim(&self) -> Dwim {
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
            BufferMode::File { .. }
            | BufferMode::RebaseTodo
            | BufferMode::Resolve
            | BufferMode::Commit
            | BufferMode::TextEdit => Dwim::None,
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
    pub(super) fn diff_dwim(&mut self) -> Result<()> {
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
    pub(super) fn log_arguments(&self) -> Vec<String> {
        match (&self.transient_log_args, &self.mode) {
            (Some(a), _) => a.clone(),
            (None, m) if m.is_log() => self.buf_args.clone(),
            (None, _) => self.log_args.clone(),
        }
    }

    /// The log commands of `l` (magit-log-current, -head, -other, -related,
    /// -branches, -all-branches, -all) and magit-log-buffer-file.
    pub(super) fn log_view(&mut self, cmd: &str) -> Result<()> {
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
    pub(super) fn related_revs(&self) -> Vec<String> {
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
        let target = crate::ui::view::push_branch(g, &current)
            .filter(|t| g.git_ok(&["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{t}")]));
        let upstream = upstream_of(&current);
        let upup = upstream
            .as_ref()
            .filter(|u| g.git_ok(&["show-ref", "--verify", "--quiet", &format!("refs/heads/{u}")]))
            .and_then(|u| upstream_of(u));
        [Some(current), head, target, upstream, upup].into_iter().flatten().collect()
    }

    /// magit-log-goto-commit-section: move to the commit REV in the log.
    pub(super) fn log_goto_commit(&mut self, rev: &str) -> bool {
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
    pub(super) fn log_set_commit_limit(&mut self, f: Limit) {
        self.buf_args = set_commit_limit(&self.buf_args, f);
        self.refresh();
    }

    /// magit-commit-at-point: the commit section at point, else the
    /// revision of a revision or stash buffer.
    pub(super) fn commit_at_point(&self) -> Option<String> {
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
    pub(super) fn region_commits(&self) -> Option<Vec<String>> {
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
    pub(super) fn region_commit_values(&self) -> Option<Vec<String>> {
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
    pub(super) fn read_string_ns(&mut self, prompt: &str) -> Result<Option<String>> {
        self.read_string_ns_default(prompt, None)
    }

    /// magit-read-string-ns with DEFAULT (the value of an empty input).
    pub(super) fn read_string_ns_default(&mut self, prompt: &str, default: Option<&str>) -> Result<Option<String>> {
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
    pub(super) fn branch_spinoff(&mut self, branch: &str, from: Option<&str>, mut checkout: bool) {
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
    pub(super) fn done_keep_msg(&mut self, res: Result<()>) {
        self.refresh();
        if let Err(e) = res {
            self.report_error(&e);
        }
    }

    /// The previously checked out branch (`@{-1}`), if any.
    pub(super) fn previous_branch(&self) -> Option<String> {
        self.git
            .git_string(&["rev-parse", "--abbrev-ref", "@{-1}"])
            .ok()
            .filter(|s| !s.is_empty() && s != "HEAD")
    }

    /// magit--cherry-pick: revert or cherry-pick COMMITS (a single "A..B"
    /// is split), handling --mainline for merge commits.
    pub(super) fn cherry_pick_run(&mut self, commits: Vec<String>, args: Vec<String>, revert: bool) -> Result<()> {
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
        // the messages (with --edit, one per commit) are written in
        // xagit's text buffer
        let mut full = vec![command.to_string()];
        full.extend(args);
        full.extend(commits);
        self.start_edit_git(full, None, EditDone::Sequence(command.to_string()), None)
    }

    /// magit-log-select: show the log of the current branch to pick a
    /// commit for ACTION, at INITIAL, else at the first commit that is not
    /// a squash!, fixup! or amend! commit. MSG is the usage, "%p" and "%q"
    /// standing for the keys that pick and quit.
    pub(super) fn log_select(&mut self, action: SelectAction, msg: Option<&str>, initial: Option<String>) {
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
    pub(super) fn log_select_pick(&mut self) -> Result<()> {
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
    pub(super) fn rebase_commit(&mut self, cmd: &str, commit: &str, args: &[String]) -> Result<()> {
        if cmd != "rebase-remove-commit" && !self.rebase_assert(commit, cmd != "rebase-reword-commit", args)? {
            return Ok(());
        }
        let res = match cmd {
            // the todo is edited in xagit's git-rebase-mode buffer
            "rebase-interactive" => return self.rebase_todo_start(commit, args),
            "rebase-edit-commit" => self.git.rebase_edit_commit(commit, args),
            // `r w`: the new message is written in xagit's text buffer
            "rebase-reword-commit" => {
                let todo = self.git.rebase_reword_todo_editor(commit)?;
                let a = self.git.rebase_i_args(commit, args, &[])?;
                return self.start_edit_git(a, Some(todo), EditDone::Rebase, None);
            }
            _ => self.git.rebase_remove_commit(commit, args),
        };
        self.after_rebase(res);
        Ok(())
    }

    /// magit-rebase-autosquash after BASE (exact, no assertions).
    pub(super) fn rebase_autosquash(&mut self, base: &str, args: &[String]) -> Result<()> {
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
    pub(super) fn commit_assert(&mut self, args: &[String], nopatch: bool, strict: bool) -> Result<Option<Vec<String>>> {
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
    pub(super) fn commit_squash(&mut self, option: &str, commit: &str, args: &[String], edit: bool, rebase: bool) -> Result<()> {
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
            // the fixup/squash message is written in xagit's text buffer
            let mut full = vec!["commit".to_string()];
            full.extend(a.iter().cloned());
            return self.start_edit_git(full, None, EditDone::Plain(String::new()), None);
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
    pub(super) fn amend_assert(&mut self, commit: &str) -> Result<bool> {
        let published = self.publishing_branches(commit);
        if published.is_empty() {
            return Ok(true);
        }
        let (m1, m2) = ("This commit has already been published to ", ".\nDo you really want to modify it");
        self.confirm_items("amend-published", &format!("{m1}%s{m2}"), &format!("{m1}%d public branches{m2}"), &published)
    }

    /// magit-list-publishing-branches: the members of
    /// magit-published-branches COMMIT is an ancestor of.
    pub(super) fn publishing_branches(&self, commit: &str) -> Vec<String> {
        self.options
            .published_branches
            .iter()
            .filter(|b| self.git.rev_verify(b) && self.git.is_ancestor(commit, b))
            .cloned()
            .collect()
    }

    /// `j` in log buffers (magit-log-move-to-revision).
    pub(super) fn log_move_to_revision(&mut self) -> Result<()> {
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
    pub(super) fn log_move_to_parent(&mut self) {
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
    pub(super) fn cherry(&mut self) -> Result<()> {
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

}
