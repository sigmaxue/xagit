//! ui/app/apply.rs — staging, unstaging, discarding and reversing changes.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

impl App {

    /// `s`: magit-stage where a section keymap remaps magit-stage-files
    /// (files, hunks, the untracked and unstaged sections), else
    /// magit-stage-files.
    pub(super) fn stage_cmd(&mut self) -> Result<()> {
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
                self.wip_hook(&files, "before", "stage")?;
                self.git.git_interactive(&a)?;
                self.wip_hook(&files, "after", "stage")?;
            }
            "unstaged" => {
                if let Some((path, scope)) = Self::hunk_scope(&sel) {
                    let fd = self.file_diff("unstaged", &path)?;
                    let files = [path];
                    self.wip_hook(&files, "before", "stage")?;
                    self.git.stage_change(&fd, &scope, ctx)?;
                    self.wip_hook(&files, "after", "stage")?;
                } else if ws {
                    let files = self.sel_paths(&sel);
                    self.wip_hook(&files, "before", "stage")?;
                    for p in &files {
                        let fd = self.file_diff("unstaged", p)?;
                        self.git.stage_change(&fd, &Scope::File, ctx)?;
                    }
                    self.wip_hook(&files, "after", "stage")?;
                } else if matches!(sel, DiffSel::List { .. }) {
                    return self.stage_modified();
                } else {
                    // magit-stage-1
                    let files = self.sel_paths(&sel);
                    let mut a = vec!["add", "-u", "--"];
                    a.extend(files.iter().map(String::as_str));
                    self.wip_hook(&files, "before", "stage")?;
                    self.git.git_interactive(&a)?;
                    self.wip_hook(&files, "after", "stage")?;
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
    pub(super) fn stage_files(&mut self) -> Result<()> {
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
        self.wip_hook(&files, "before", "stage")?;
        self.git.git_interactive(&a)?;
        self.wip_hook(&files, "after", "stage")?;
        self.set_msg("staged");
        self.refresh();
        Ok(())
    }

    /// magit-completing-read-multiple of files from CHOICES (separated by
    /// commas), the default being the file at point or the visited file.
    pub(super) fn pick_files(&mut self, prompt: &str, choices: &[String]) -> Result<Option<Vec<String>>> {
        if choices.is_empty() {
            self.set_msg("no candidates");
            return Ok(None);
        }
        let default = self
            .current_file()
            .map(|f| f.1)
            .or_else(|| self.visited_file().map(|f| f.1))
            .filter(|f| choices.contains(f));
        let table = Completion { candidates: choices.to_vec(), require_match: true, multiple: true };
        let Some(inp) = self
            .completing_read(prompt, default.as_deref(), Hist::Minibuffer, Some(table))?
            .filter(|s| !s.is_empty())
        else {
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
    pub(super) fn stage_modified(&mut self) -> Result<()> {
        if self.git.anything_staged() && !self.confirm("stage-all-changes", "Stage all changes")? {
            return Ok(());
        }
        self.wip_hook(&[], "before", "stage")?;
        if self.prefix_arg.is_some() {
            self.git.stage_all_with_untracked()?;
            self.set_msg("staged all changes");
        } else {
            self.git.stage_all()?;
            self.set_msg("staged all modified");
        }
        self.wip_hook(&[], "after", "stage")?;
        self.refresh();
        Ok(())
    }

    /// `u`: magit-unstage on files, hunks and the staged section, else
    /// magit-unstage-files.
    pub(super) fn unstage_cmd(&mut self) -> Result<()> {
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
                self.unstage_1(&intent)?;
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
                    let files = [path];
                    self.wip_hook(&files, "before", "unstage")?;
                    self.git.unstage_change(&fd, &scope, ctx)?;
                    self.wip_hook(&files, "after", "unstage")?;
                } else if ws {
                    let files = self.sel_paths(&sel);
                    self.wip_hook(&files, "before", "unstage")?;
                    for p in &files {
                        let fd = self.file_diff("staged", p)?;
                        self.git.unstage_change(&fd, &Scope::File, ctx)?;
                    }
                    self.wip_hook(&files, "after", "unstage")?;
                } else if matches!(sel, DiffSel::List { .. }) {
                    return self.unstage_all();
                } else {
                    let files = self.sel_paths(&sel);
                    self.unstage_1(&files)?;
                }
            }
        }
        self.refresh();
        Ok(())
    }

    /// magit-unstage-files: read staged files, default the file at point.
    pub(super) fn unstage_files(&mut self) -> Result<()> {
        let choices = self.git.staged_files();
        let Some(files) = self.pick_files("Unstage file,s", &choices)? else {
            return Ok(());
        };
        self.unstage_1(&files)?;
        self.set_msg("unstaged");
        self.refresh();
        Ok(())
    }

    /// magit-rev-name REV PATTERN (anchored), unless relative to the ref
    /// (with `~` or `^`).
    pub(super) fn rev_name_exact(&self, rev: &str, pattern: &str) -> Option<String> {
        let refs = format!("--refs={pattern}");
        let exclude = format!("--exclude=*/{pattern}");
        let a = ["name-rev", "--name-only", "--no-undefined", &refs, "--exclude=*/HEAD", &exclude, rev];
        let n = self.git.git_string(&a).ok()?;
        let n = n.trim();
        (!n.is_empty() && !n.contains(['~', '^'])).then(|| n.to_string())
    }

    /// magit-name-local-branch: the local branch at REV.
    pub(super) fn name_local_branch(&self, rev: &str) -> Option<String> {
        self.rev_name_exact(rev, "refs/heads/*")
    }

    /// magit-name-remote-branch: the remote branch at REV.
    pub(super) fn name_remote_branch(&self, rev: &str) -> Option<String> {
        self.rev_name_exact(rev, "refs/remotes/*").and_then(|n| n.strip_prefix("remotes/").map(str::to_string))
    }

    /// magit-ref-maybe-qualify: NAME, or PREFIX and NAME if NAME alone is
    /// ambiguous (None if that is too).
    pub(super) fn ref_maybe_qualify(&self, name: &str, prefix: &str) -> Option<String> {
        let ambiguous = |r: &str| {
            let base = r.split(['~', '^']).next().unwrap_or(r);
            self.git
                .git_string(&["rev-parse", "--verify", "--symbolic-full-name", base])
                .map_or(true, |f| f.trim().is_empty())
        };
        if !ambiguous(name) {
            return Some(name.to_string());
        }
        let q = format!("{prefix}{name}");
        (!ambiguous(&q)).then_some(q)
    }

    /// magit-branch-or-commit-at-point: the branch (local or remote) or
    /// tag at point, the commit at point (the branch at it, if any), the
    /// revision of a revision or stash buffer.
    pub(super) fn branch_or_commit_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Ref { kind: RefKind::Local | RefKind::Remote, name }) => self.ref_maybe_qualify(&name, "heads/"),
            Some(LineKind::Ref { kind: RefKind::Tag, name }) => self.ref_maybe_qualify(&name, "tags/"),
            Some(LineKind::Commit { hash, .. }) => Some(
                self.name_local_branch(&hash)
                    .or_else(|| self.name_remote_branch(&hash))
                    .unwrap_or_else(|| self.git.rev_short(&hash)),
            ),
            _ => match &self.mode {
                BufferMode::Revision { rev, .. } => Some(rev.clone()),
                BufferMode::Stash { stash } => Some(stash.clone()),
                _ => None,
            },
        }
    }

    /// magit-read-other-branch-or-commit: a ref (or any input) other
    /// than the current branch; the default the branch or commit at
    /// point (not HEAD's commit when detached), else the previous branch.
    pub(super) fn read_other_branch_or_commit(&mut self, prompt: &str) -> Result<Option<String>> {
        self.read_other_branch_or_commit_excluding(prompt, None)
    }

    /// magit-read-other-branch-or-commit with EXCLUDE (else the current
    /// branch): the current branch also a default when not excluded.
    pub(super) fn read_other_branch_or_commit_excluding(&mut self, prompt: &str, exclude: Option<&str>) -> Result<Option<String>> {
        let current = self.git.current_branch();
        let exclude = exclude.map(str::to_string).or_else(|| current.clone());
        let rev = |r: &str| self.git.git_string(&["rev-parse", "--verify", "-q", r]).ok().filter(|s| !s.is_empty());
        let default = self
            .branch_or_commit_at_point()
            .filter(|a| Some(a) != exclude.as_ref() && !(current.is_none() && rev(a) == rev("HEAD")))
            .or_else(|| current.clone().filter(|c| Some(c) != exclude.as_ref()))
            .or_else(|| self.previous_branch());
        let mut candidates = self.git.refnames();
        candidates.retain(|r| Some(r) != exclude.as_ref());
        let table = Completion { candidates, ..Default::default() };
        self.read_any(prompt, default.as_deref(), Hist::Revision, table)
    }

    /// magit-completing-read with REQUIRE-MATCH `any`: an empty input is
    /// the default, without one "Nothing selected".
    pub(super) fn read_any(&mut self, prompt: &str, default: Option<&str>, hist: Hist, table: Completion) -> Result<Option<String>> {
        let r = self.completing_read(prompt, default, hist, Some(table))?;
        if r.as_deref() == Some("") {
            self.set_msg("Nothing selected");
            return Ok(None);
        }
        Ok(r)
    }

    /// magit--checkout: `git checkout ARGS REV` (a leading heads/
    /// removed).
    pub(super) fn checkout_rev(&mut self, rev: &str, args: &[String]) {
        let rev = rev.strip_prefix("heads/").unwrap_or(rev);
        let mut a: Vec<&str> = vec!["checkout"];
        a.extend(args.iter().map(String::as_str));
        a.push(rev);
        let res = self.git.git_interactive(&a);
        self.done("", res.map(|_| String::new()));
    }

    /// magit-branch-checkout (b l): a local branch (not the current one)
    /// is checked out; a remote branch (unless a local branch has its
    /// name) becomes a new local branch of that name, the remote its
    /// push-remote unless it is remote.pushDefault; another name is a
    /// new branch, its starting point read.
    pub(super) fn branch_checkout(&mut self, args: &[String]) -> Result<()> {
        let current = self.git.current_branch();
        let local = self.git.branches();
        let remote: Vec<String> = self
            .git
            .remote_branches()
            .into_iter()
            .filter(|r| r.split_once('/').is_some_and(|(_, b)| b != "HEAD" && !local.iter().any(|l| l == b)))
            .collect();
        let mut choices: Vec<String> = local.iter().filter(|b| Some(*b) != current.as_ref()).cloned().collect();
        choices.extend(remote.iter().cloned());
        let atpoint = self.branch_at_point();
        let default = atpoint.as_ref().filter(|a| choices.contains(a)).cloned().or_else(|| {
            let short = atpoint.as_ref()?.split_once('/')?.1.to_string();
            choices.contains(&short).then_some(short)
        });
        let table = Completion { candidates: choices, ..Default::default() };
        let choice = self.completing_read("Checkout branch", default.as_deref(), Hist::Revision, Some(table))?;
        let Some(choice) = choice.filter(|c| !c.is_empty()) else {
            return Ok(());
        };
        let (branch, start) = if remote.contains(&choice) {
            (choice.split_once('/').map_or(choice.clone(), |(_, b)| b.to_string()), choice.clone())
        } else if local.contains(&choice) {
            self.checkout_rev(&choice, args);
            return Ok(());
        } else {
            let Some(start) = self.read_starting_point("Create", Some(&choice), None)? else {
                return Ok(());
            };
            (choice, start)
        };
        let modified = !self.git.staged_files().is_empty() || !self.git.unstaged_files().is_empty();
        if modified && !args.iter().any(|a| a == "--merge") {
            self.set_msg("Cannot checkout when there are uncommitted changes");
            return Ok(());
        }
        let mut a: Vec<&str> = vec!["checkout"];
        a.extend(args.iter().map(String::as_str));
        a.extend(["-b", &branch, &start]);
        let res = self.git.git_interactive(&a);
        if res.is_ok() {
            self.branch_maybe_adjust_upstream(&branch, &start);
            if let Some((r, b)) = self.split_remote_branch(&start) {
                let push_default = self.git.git_string(&["config", "remote.pushDefault"]).unwrap_or_default();
                if b == branch && r != push_default.trim() {
                    self.git.git_interactive(&["config", &format!("branch.{branch}.pushRemote"), &r])?;
                }
            }
        }
        self.done("", res.map(|_| String::new()));
        Ok(())
    }

    /// magit-split-branch-name of the remote branch NAME: its remote and
    /// branch (None if not a remote branch).
    pub(super) fn split_remote_branch(&self, name: &str) -> Option<(String, String)> {
        if !self.git.git_ok(&["show-ref", "--verify", "--quiet", &format!("refs/remotes/{name}")]) {
            return None;
        }
        self.git.remotes().into_iter().find_map(|r| {
            let b = name.strip_prefix(&format!("{r}/"))?.to_string();
            Some((r, b))
        })
    }

    /// magit-read-starting-point: "PROMPT [BRANCH] starting at" a ref
    /// (the upstream first), HEAD or a *_HEAD of the git directory; the
    /// default DEFAULT, else magit--default-starting-point.
    pub(super) fn read_starting_point(&mut self, prompt: &str, branch: Option<&str>, default: Option<&str>) -> Result<Option<String>> {
        let prompt = match branch {
            Some(b) => format!("{prompt} {b} starting at"),
            None => format!("{prompt} starting at"),
        };
        let mut candidates = self.git.refnames();
        if let Some(u) = self.git.upstream_shortname() {
            candidates.retain(|r| *r != u);
            candidates.insert(0, u);
        }
        candidates.push("HEAD".into());
        if let Ok(rd) = std::fs::read_dir(&self.git.gitdir) {
            let mut heads: Vec<String> =
                rd.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.ends_with("_HEAD")).collect();
            heads.sort();
            candidates.extend(heads);
        }
        let default = default.map(str::to_string).or_else(|| self.default_starting_point());
        let table = Completion { candidates, ..Default::default() };
        self.read_any(&prompt, default.as_deref(), Hist::Revision, table)
    }

    /// magit--default-starting-point: the local, else the remote branch
    /// at point, the commit or stash at point, the current branch.
    pub(super) fn default_starting_point(&self) -> Option<String> {
        let kind = self.current_line().map(|l| l.kind.clone());
        let (local, remote) = match &kind {
            Some(LineKind::Ref { kind: RefKind::Local, name }) => (Some(name.clone()), None),
            Some(LineKind::Ref { kind: RefKind::Remote, name }) => (None, Some(name.clone())),
            Some(LineKind::Commit { hash, .. }) => (self.name_local_branch(hash), self.name_remote_branch(hash)),
            _ => (None, None),
        };
        local
            .or(remote)
            .or_else(|| self.commit_at_point().map(|c| if is_stash_ref(&c) { c } else { self.git.rev_short(&c) }))
            .or(match kind {
                Some(LineKind::Stash { name, .. }) => Some(name),
                _ => None,
            })
            .or_else(|| self.git.current_branch())
    }

    /// magit-branch-read-args (magit-branch-read-upstream-first t): the
    /// starting point, then the name of the new branch.
    pub(super) fn branch_read_args(&mut self, prompt: &str) -> Result<Option<(String, String)>> {
        let Some(start) = self.read_starting_point(prompt, None, None)? else {
            return Ok(None);
        };
        if !self.git.git_ok(&["rev-parse", "--verify", "-q", &start]) {
            self.set_msg(&format!("Not a valid starting-point: {start}"));
            return Ok(None);
        }
        Ok(self.branch_read_name("Name for new branch", Some(&start))?.map(|n| (n, start)))
    }

    /// magit-branch--read-name: a new branch name (the branch name of a
    /// remote branch START-POINT suggested) with minibuffer-history; the
    /// names of existing branches and invalid names are refused.
    pub(super) fn branch_read_name(&mut self, prompt: &str, start_point: Option<&str>) -> Result<Option<String>> {
        let taken = self.git.branches();
        let mut choices = self.options.branch_name_suggestions.clone();
        if let Some((_, b)) = start_point.and_then(|s| self.split_remote_branch(s)) {
            if !choices.contains(&b) {
                choices.insert(0, b);
            }
        }
        choices.retain(|c| !taken.contains(c));
        loop {
            let table = Completion { candidates: choices.clone(), ..Default::default() };
            let Some(name) = self.completing_read(prompt, None, Hist::Minibuffer, Some(table))? else {
                return Ok(None);
            };
            if name.is_empty() {
                self.set_msg("Nothing selected");
                return Ok(None);
            }
            if taken.contains(&name) {
                self.set_msg("conflicts with existing branch");
            } else if !self.git.git_ok(&["check-ref-format", "--branch", &name]) {
                self.set_msg("not a valid branch name");
            } else {
                return Ok(Some(name));
            }
        }
    }

    /// magit-read-branch-or-commit: a ref or commit, the default the
    /// branch or commit at point, else the current branch.
    pub(super) fn read_branch_or_commit(&self, prompt: &str) -> Result<Option<String>> {
        let default = self
            .branch_at_point()
            .or_else(|| self.commit_at_point().map(|c| self.git.rev_short(&c)))
            .or_else(|| self.git.current_branch());
        self.read_rev(prompt, default.as_deref())
    }

    /// magit-reset-internal: `git reset ARG COMMIT -- PATH` between the
    /// wip hooks ("unstage" when only the index is reset to HEAD, else
    /// "reset", which has no after hook).
    pub(super) fn reset_internal(&mut self, arg: Option<&str>, commit: &str, path: Option<&str>) -> Result<()> {
        let task = if commit == "HEAD" && arg.is_none() { "unstage" } else { "reset" };
        self.wip_hook(&[], "before", task)?;
        let mut a = vec!["reset"];
        a.extend(arg);
        a.extend([commit, "--"]);
        a.extend(path);
        let res = self.git.git_interactive(&a);
        if res.is_ok() && task == "unstage" {
            self.wip_hook(&[], "after", "unstage")?;
        }
        self.done("", res.map(|_| String::new()));
        Ok(())
    }

    /// magit-unstage-1: unstage FILES, between the wip hooks.
    pub(super) fn unstage_1(&mut self, files: &[String]) -> Result<()> {
        self.wip_hook(files, "before", "unstage")?;
        for f in files {
            self.git.unstage(f)?;
        }
        self.wip_hook(files, "after", "unstage")
    }

    /// magit-run-before-change-functions (WHEN "before") and
    /// magit-run-after-apply-functions ("after") with magit-wip-mode:
    /// FILES (all tracked files when empty) committed to the wip refs,
    /// the message ending with " WHEN TASK".
    pub(super) fn wip_hook(&mut self, files: &[String], when: &str, task: &str) -> Result<()> {
        if !self.options.wip_mode {
            return Ok(());
        }
        if let Some(m) = self.options.wip.commit(&self.git, files, Some(&format!(" {when} {task}")))? {
            self.set_msg(&m);
        }
        Ok(())
    }

    /// magit-unstage-all (U)
    pub(super) fn unstage_all(&mut self) -> Result<()> {
        if !self.git.anything_staged() {
            self.set_msg("Nothing to unstage");
            return Ok(());
        }
        let others = !self.git.unstaged_files().is_empty() || !self.git.untracked_files().is_empty();
        if others && !self.confirm("unstage-all-changes", "Unstage all changes")? {
            return Ok(());
        }
        self.wip_hook(&[], "before", "unstage")?;
        self.git.unstage_all()?;
        self.wip_hook(&[], "after", "unstage")?;
        self.set_msg("unstaged all");
        self.refresh();
        Ok(())
    }

    /// `k`: magit-discard on changes, magit-stash-drop on a stash,
    /// magit-stash-clear on the stashes section.
    pub(super) fn discard_cmd(&mut self) -> Result<()> {
        match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Stash { .. }) => return self.stash_drop(),
            // magit-worktree-section-map: magit-delete-thing is
            // magit-worktree-delete
            Some(LineKind::Worktree { .. }) => return self.worktree_delete(),
            Some(LineKind::SectionHeader { section, .. }) if section == "stashes" => {
                if self.confirm("drop-stashes", "Drop all stashes in refs/stash")? {
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
            if !self.confirm("discard", &Self::hunk_prompt("Discard", &sel))? {
                return Ok(());
            }
            // magit-discard-hunk: conflicts are resolved with smerge
            if matches!(sel, DiffSel::Hunk { .. })
                && file_status(&self.git, &["--", path.as_str()]).first().is_some_and(|s| (s.x, s.y) == ('U', 'U'))
            {
                return self.call_smerge(Keep::Current, true);
            }
            let fd = self.file_diff(&group, &path)?;
            let files = [path];
            self.wip_hook(&files, "before", "discard")?;
            self.git.discard_change(&fd, &scope, typ == "staged", ctx)?;
            self.wip_hook(&files, "after", "discard")?;
        } else {
            let r = if matches!(&sel, DiffSel::List { typ } if typ == "untracked") {
                // magit-discard-untracked: magit-list-untracked-files
                self.set_msg("");
                let files = self.list_untracked_files();
                self.discard_files_delete(&files, "untracked", &[]).map(|_| ())
            } else {
                let files = self.sel_paths(&sel);
                self.discard_files(&typ, &group, &files)
            };
            self.refresh();
            return r;
        }
        self.set_msg("discarded");
        self.refresh();
        Ok(())
    }

    /// magit-call-smerge: smerge-keep-* (HOW) on the conflict at the line
    /// the hunk line at point visits (magit-diff-visit-file--noselect),
    /// then the file is saved and the buffer refreshed. Outside a
    /// conflict the next one is used; for smerge-keep-current there only
    /// magit-discard (DISCARD) does so, asking which side to keep.
    pub(super) fn call_smerge(&mut self, how: Keep, discard: bool) -> Result<()> {
        let (group, path, hunk, line) = match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Hunk { section, path, hunk }) => (section, path, hunk, None),
            Some(LineKind::DiffLine { section, path, hunk, line }) => (section, path, hunk, Some(line)),
            _ => return Ok(()),
        };
        let fd = self.file_diff(&group, &path)?;
        let (old, new) = self.visit_sides(&group, &path);
        let goto_from = line.is_some_and(|l| {
            !fd.combined && fd.hunks.get(hunk).and_then(|h| h.lines.get(l)).is_some_and(|t| t.starts_with('-'))
        });
        let (rev, file) = if goto_from { old } else { new };
        let n = Self::hunk_line(&fd, hunk, line, goto_from);
        let worktree = rev == "{worktree}";
        let abs = self.git.toplevel.join(&file);
        let text = if worktree {
            std::fs::read(&abs).map_err(|_| anyhow::anyhow!("Cannot visit {file}: file does not exist"))?
        } else {
            crate::ui::blame::file_contents(&self.git, &rev, &file)?.into_bytes()
        };
        let name = std::path::Path::new(&file).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let mut pos = smerge::line_position(&text, n);
        let mut how = how;
        if smerge::match_conflict(&text, pos, &name).is_err() && (how != Keep::Current || discard) {
            pos = smerge::next_conflict(&text, pos);
            if how == Keep::Current {
                let prompt = "Keep side: [o]urs/upper, [b]ase, [t]heirs/lower, or [C-g] to abort ";
                how = match self.read_char_choice(prompt, &['o', 'b', 't'])? {
                    Some('o') => Keep::Upper,
                    Some('b') => Keep::Base,
                    Some(_) => Keep::Lower,
                    None => {
                        self.set_msg("Quit");
                        return Ok(());
                    }
                };
                self.set_msg("");
            }
        }
        match smerge::keep(&text, pos, how, &name) {
            Err(e) => self.set_msg(&e),
            Ok(_) if !worktree => self.set_msg(&format!("Buffer is read-only: #<buffer {name}.~{rev}~>")),
            Ok(out) => {
                std::fs::write(&abs, out)?;
                self.refresh();
                self.set_msg(&format!("Wrote {}", abs.display()));
            }
        }
        Ok(())
    }

    /// magit-list-untracked-files: the untracked files and directories
    /// (`--untracked-files=normal`, or `all` per the local
    /// status.showUntrackedFiles; none for `no`).
    pub(super) fn list_untracked_files(&self) -> Vec<String> {
        let local = self.git.git_string(&["config", "--local", "status.showUntrackedFiles"]).unwrap_or_default();
        let mode = match local.trim() {
            "no" | "off" | "false" | "0" => return Vec::new(),
            "all" => "all",
            _ => "normal",
        };
        file_status(&self.git, &["--porcelain", &format!("--untracked-files={mode}")])
            .into_iter()
            .filter(|s| s.x == '?')
            .map(|s| s.file)
            .collect()
    }

    /// magit-discard-files: the FILES of a TYP (staged, unstaged or
    /// untracked) section of GROUP, by their status: conflicts are
    /// resolved, deletions resurrected, new and untracked files deleted,
    /// renames undone, other changes discarded, each kind confirmed.
    pub(super) fn discard_files(&mut self, typ: &str, group: &str, files: &[String]) -> Result<()> {
        let status = file_status(&self.git, &[]);
        let t = match typ {
            "staged" => 'X',
            "unstaged" => 'Y',
            _ => 'Z',
        };
        let (mut delete, mut resurrect, mut rename, mut discard, mut discard_new, mut resolve) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for file in files {
            let f = file.clone();
            match (t, status.iter().find(|s| s.file == *file).map(|s| (s.x, s.y))) {
                // an untracked directory whose files are listed
                ('Z', None) => delete.extend(
                    file_status(&self.git, &["--porcelain", "--untracked-files=all", "--", file])
                        .into_iter()
                        .filter(|s| matches!((s.x, s.y), ('?', '?') | ('!', '!')))
                        .map(|s| s.file),
                ),
                ('Z', Some(('?', '?') | ('!', '!') | ('D', ' '))) => delete.push(f),
                (_, Some(('D', 'D') | ('U', _) | (_, 'U') | ('A', 'A'))) => resolve.push(f),
                ('X', Some(('M', ' ' | 'M' | 'D'))) | ('Y', Some((_, 'M'))) => discard.push(f),
                ('X', Some(('A' | 'C', 'M'))) => discard_new.push(f),
                ('X', Some(('A' | 'C', ' ' | 'D'))) => delete.push(f),
                ('X', Some(('D', ' ' | 'M'))) | ('Y', Some((_, 'D'))) => resurrect.push(f),
                ('X', Some(('R', ' ' | 'M' | 'D'))) => rename.push(f),
                _ => {}
            }
        }
        self.set_msg("");
        self.wip_hook(files, "before", "discard")?;
        if self.discard_files_kinds(typ, group, &status, [resolve, resurrect, delete, rename, discard, discard_new])? {
            self.wip_hook(files, "after", "discard")?;
        }
        Ok(())
    }

    /// The kinds of files of magit-discard-files handled in order, false
    /// when one of them is declined or fails.
    pub(super) fn discard_files_kinds(&mut self, typ: &str, group: &str, status: &[FileStatus], kinds: [Vec<String>; 6]) -> Result<bool> {
        let [resolve, resurrect, delete, rename, discard, discard_new] = kinds;
        if !resolve.is_empty() && !self.discard_files_resolve(&resolve)? {
            return Ok(false);
        }
        if !resurrect.is_empty() {
            if !self.confirm_files("resurrect", "Resurrect", &resurrect)? {
                return Ok(false);
            }
            let mut args = if typ == "staged" { vec!["reset", "--"] } else { vec!["checkout", "--"] };
            args.extend(resurrect.iter().map(String::as_str));
            self.git.git_interactive(&args)?;
        }
        if !delete.is_empty() && !self.discard_files_delete(&delete, typ, status)? {
            return Ok(false);
        }
        if !rename.is_empty() {
            let orig = |f: &String| status.iter().find(|s| s.file == *f).and_then(|s| s.orig.clone()).unwrap_or_default();
            let items: Vec<String> = rename.iter().map(|f| format!("{} -> {f}", orig(f))).collect();
            if !self.confirm_items("rename", "Undo rename %s", "Undo %d renames", &items)? {
                return Ok(false);
            }
            let top = self.git.toplevel.clone();
            for f in &rename {
                let o = orig(f);
                if top.join(f).exists() {
                    if let Some(parent) = top.join(&o).parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    self.git.git_interactive(&["mv", f, &o])?;
                } else {
                    self.git.git_interactive(&["rm", "--cached", "--", f])?;
                    self.git.git_interactive(&["reset", "--", &o])?;
                }
            }
        }
        if !discard.is_empty() || !discard_new.is_empty() {
            return self.discard_files_discard(typ, group, &discard, &discard_new);
        }
        Ok(true)
    }

    /// magit-discard-files--discard: FILES with changes and NEW_FILES
    /// (added and modified) of a TYP section, confirmed together.
    pub(super) fn discard_files_discard(&mut self, typ: &str, group: &str, files: &[String], new_files: &[String]) -> Result<bool> {
        let all: Vec<String> = files.iter().chain(new_files).cloned().collect();
        if !self.confirm_files("discard", &format!("Discard {typ} changes in"), &all)? {
            return Ok(false);
        }
        if typ == "unstaged" {
            let mut args = vec!["checkout", "--"];
            args.extend(files.iter().map(String::as_str));
            return self.git.git_interactive(&args).map(|_| true);
        }
        if !new_files.is_empty() {
            for cmd in ["add", "reset"] {
                let mut args = vec![cmd, "--"];
                args.extend(new_files.iter().map(String::as_str));
                self.git.git_interactive(&args)?;
            }
        }
        let mut diffs = Vec::new();
        let mut binaries = Vec::new();
        for f in files {
            let fd = self.file_diff(group, f)?;
            if fd.binary {
                binaries.push(f.clone());
            } else {
                diffs.push(fd);
            }
        }
        if !diffs.is_empty() {
            // magit-discard-apply with magit-apply-diffs
            let ctx = self.context();
            let refs: Vec<&FileDiff> = diffs.iter().collect();
            if self.git.file_has_unstaged(&diffs[0].path) {
                self.git.apply_files(&refs, &["--reverse", "--cached"], ctx)?;
                self.git.apply_files(&refs, &["--reverse", "--reject"], ctx)?;
            } else {
                self.git.apply_files(&refs, &["--reverse", "--index"], ctx)?;
            }
        }
        if !binaries.is_empty() {
            let modified = self.git.unstaged_files();
            let (dirty, clean): (Vec<String>, Vec<String>) = binaries.into_iter().partition(|b| modified.contains(b));
            if !clean.is_empty() {
                let mut args = vec!["reset", "--"];
                args.extend(clean.iter().map(String::as_str));
                self.git.git_interactive(&args)?;
            }
            if !dirty.is_empty() {
                self.set_msg(
                    "Cannot discard staged changes to binary files, which also have unstaged changes.  Unstage instead.",
                );
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// magit-discard-files--resolve: check out a stage of the conflicted
    /// FILES, chosen for all of them or for each. False when quit.
    pub(super) fn discard_files_resolve(&mut self, files: &[String]) -> Result<bool> {
        let stage = |c: char| match c {
            'o' => Some("--ours"),
            't' => Some("--theirs"),
            'c' => Some("--merge"),
            _ => None,
        };
        let mut arg = None;
        if files.len() > 1 {
            let list: Vec<String> = files.iter().map(|f| format!("  {f}")).collect();
            let prompt = format!(
                "For these {} files\n{}\ncheckout:\n[o]ur stage, [t]heir stage, [c]onflict, decide [i]ndividually, or [C-g] to abort ",
                files.len(),
                list.join("\n")
            );
            let Some(c) = self.read_char_choice(&prompt, &['o', 't', 'c', 'i'])? else {
                self.set_msg("Quit");
                return Ok(false);
            };
            arg = stage(c);
        }
        for file in files {
            let a = match arg {
                Some(a) => a,
                None => {
                    let prompt = format!("For {file} checkout: [o]ur stage, [t]heir stage, [c]onflict, or [C-g] to abort ");
                    match self.read_char_choice(&prompt, &['o', 't', 'c'])?.and_then(stage) {
                        Some(a) => a,
                        None => {
                            self.set_msg("Quit");
                            return Ok(false);
                        }
                    }
                }
            };
            self.checkout_stage(file, a)?;
        }
        Ok(true)
    }

    /// magit-checkout-stage: check out and stage the side ARG (--ours,
    /// --theirs) of the conflicted FILE (removed if deleted on that side),
    /// or restore the conflict (--merge).
    pub(super) fn checkout_stage(&mut self, file: &str, arg: &str) -> Result<()> {
        let st = file_status(&self.git, &["--", file]).into_iter().next().map(|s| (s.x, s.y));
        match (arg, st) {
            ("--ours", Some(('D', _) | ('U', 'A'))) | ("--theirs", Some((_, 'D') | ('A', 'U'))) => {
                self.git.git_interactive(&["rm", "--", file])
            }
            ("--merge", _) => self.git.git_interactive(&["checkout", "--merge", "--", file]),
            _ => {
                self.git.git_interactive(&["checkout", arg, "--", file])?;
                self.git.git_interactive(&["add", "-u", "--", file])
            }
        }
    }

    /// magit-discard-files--delete of FILES of a TYP section (STATUS
    /// their magit-file-status): confirmed as `trash` (moved to the
    /// trash, magit-delete-by-moving-to-trash) or `delete`. Untracked
    /// and unstaged files are deleted, non-empty directories confirmed
    /// recursively (dired-delete-file with dired-recursive-deletes
    /// `top`); new staged files are also removed from the index. False
    /// when declined or failed.
    pub(super) fn discard_files_delete(&mut self, files: &[String], typ: &str, status: &[FileStatus]) -> Result<bool> {
        let trash = self.options.delete_by_moving_to_trash;
        let (action, prompt) = if trash { ("trash", "Trash") } else { ("delete", "Delete") };
        if !self.confirm_files(action, prompt, files)? {
            return Ok(false);
        }
        let top = self.git.toplevel.clone();
        let mut always = false;
        for file in files {
            if file.starts_with('~') || file.starts_with("\\~") {
                self.set_msg(&format!("Refusing to delete \"{file}\", too dangerous"));
                return Ok(false);
            }
            let path = top.join(file);
            let y = status.iter().find(|s| s.file == *file).map(|s| s.y);
            let res = match y {
                _ if matches!(typ, "unstaged" | "untracked") => self.dired_delete_file(file, &path, trash, &mut always)?,
                Some(' ') => self.delete_file(&path, trash).and_then(|()| self.git_call(&["rm", "--cached", "--", file])),
                Some('M') => self.trash_index_version(file, trash),
                Some('D') => self
                    .git_call(&["checkout", "--", file])
                    .and_then(|()| self.delete_file(&path, trash))
                    .and_then(|()| self.git_call(&["rm", "--cached", "--force", "--", file])),
                _ => Ok(()),
            };
            if let Err(e) = res {
                self.set_msg(&e);
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The version of the new FILE in the index moved to the trash as
    /// FILE.~{index}~ (or deleted), FILE removed from the index.
    pub(super) fn trash_index_version(&self, file: &str, trash: bool) -> std::result::Result<(), String> {
        let out = self.git.git_string(&["checkout-index", "--temp", "--", file]).map_err(|e| e.to_string())?;
        let temp = out.split('\t').next().unwrap_or_default().to_string();
        let top = &self.git.toplevel;
        let saved = top.join(format!("{file}.~{{index}}~"));
        std::fs::rename(top.join(temp), &saved).map_err(|e| e.to_string())?;
        self.delete_file(&saved, trash)?;
        self.git_call(&["rm", "--cached", "--force", "--", file])
    }

    /// magit-call-git with the error as a message.
    pub(super) fn git_call(&self, args: &[&str]) -> std::result::Result<(), String> {
        self.git.git_interactive(args).map_err(|e| e.to_string())
    }

    /// dired-delete-file of FILE (at PATH): a non-empty directory is
    /// deleted recursively if confirmed (yes, no, all: ALWAYS from now
    /// on, quit). Err for the message of a failure or quit.
    pub(super) fn dired_delete_file(
        &mut self,
        file: &str,
        path: &std::path::Path,
        trash: bool,
        always: &mut bool,
    ) -> Result<std::result::Result<(), String>> {
        let nonempty_dir = !path.is_symlink()
            && path.is_dir()
            && std::fs::read_dir(path).map(|mut d| d.next().is_some()).unwrap_or(false);
        if nonempty_dir && !*always {
            let verb = if trash { "trash" } else { "delete" };
            let answers = ["yes", "no", "all", "quit"];
            match self.read_answer(&format!("Recursively {verb} {file}?"), &answers)?.as_deref() {
                Some("yes") => {}
                Some("all") => *always = true,
                Some("no") => {
                    return Ok(Err(if trash {
                        "Directory is not empty, not moving to trash".to_string()
                    } else {
                        format!("Removing directory: Directory not empty, {}", path.display())
                    }))
                }
                _ => return Ok(Err("Quit".into())),
            }
        }
        Ok(self.delete_file(path, trash))
    }

    /// delete-file / delete-directory (recursive) of PATH, moving it to
    /// the trash if TRASH.
    pub(super) fn delete_file(&self, path: &std::path::Path, trash: bool) -> std::result::Result<(), String> {
        if trash {
            return crate::ui::trash::move_to_trash(path, self.options.trash_directory.as_deref());
        }
        let r = if path.is_dir() && !path.is_symlink() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
        r.map_err(|e| format!("Removing {}: {e}", path.display()))
    }

    /// read-answer with the long ANSWERS (and help): one of them, None
    /// when quit.
    pub(super) fn read_answer(&mut self, question: &str, answers: &[&str]) -> Result<Option<String>> {
        let mut all: Vec<&str> = answers.to_vec();
        all.push("help");
        let prompt = format!("{question} ({})", all.join(", "));
        loop {
            let Some(ans) = self.read_line_hist(&prompt, None, Hist::None, None)? else {
                return Ok(None);
            };
            let ans = ans.trim().to_lowercase();
            if answers.contains(&ans.as_str()) {
                return Ok(Some(ans));
            }
            let list: Vec<String> = all.iter().map(|a| format!("`{a}'")).collect();
            self.set_msg(&format!("Please answer {}.", list.join(" or ")));
        }
    }

    /// `v`: magit-reverse on changes; magit-revert-no-commit elsewhere
    /// (default: the commit at point).
    pub(super) fn reverse_cmd(&mut self) -> Result<()> {
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
    pub(super) fn reverse_sel(&mut self, sel: &DiffSel, args: &[&str]) -> Result<()> {
        let group = sel.typ().to_string();
        let ctx = self.context();
        // the command in the wip messages: magit-unstage for committed changes
        let task = if args.contains(&"--cached") { "unstage" } else { "reverse" };
        let mut a = vec!["--reverse"];
        a.extend(args);
        if !args.contains(&"--3way") {
            a.push("--reject");
        }
        if let Some((path, scope)) = Self::hunk_scope(sel) {
            if !self.confirm("reverse", &Self::hunk_prompt("Reverse", sel))? {
                return Ok(());
            }
            let fd = self.file_diff(&group, &path)?;
            let files = [path];
            self.wip_hook(&files, "before", task)?;
            self.git.apply_scope(&fd, &scope, &a, ctx)?;
            self.wip_hook(&files, "after", task)?;
        } else {
            let (binaries, files): (Vec<String>, Vec<String>) = self
                .sel_paths(sel)
                .into_iter()
                .partition(|f| self.buf.diff(&group, f).is_some_and(|d| d.binary));
            if !self.confirm_files("reverse", "Reverse", &files)? {
                return Ok(());
            }
            let fds = files.iter().map(|f| self.file_diff(&group, f)).collect::<Result<Vec<_>>>()?;
            self.wip_hook(&files, "before", task)?;
            self.git.apply_files(&fds.iter().collect::<Vec<_>>(), &a, ctx)?;
            self.wip_hook(&files, "after", task)?;
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
    pub(super) fn apply_cmd(&mut self, args: &[String]) -> Result<()> {
        if let Some(LineKind::Stash { .. }) = self.current_line().map(|l| l.kind.clone()) {
            return self.stash_command("stash-apply", &[]);
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
                    let files = [path];
                    self.wip_hook(&files, "before", "apply")?;
                    self.git.apply_scope(&fd, &scope, &three, ctx)?;
                    self.wip_hook(&files, "after", "apply")?;
                } else {
                    let files = self.sel_paths(&sel);
                    let fds = files.iter().map(|f| self.file_diff(&group, f)).collect::<Result<Vec<_>>>()?;
                    self.wip_hook(&files, "before", "apply")?;
                    self.git.apply_files(&fds.iter().collect::<Vec<_>>(), &three, ctx)?;
                    self.wip_hook(&files, "after", "apply")?;
                }
                self.refresh();
            }
        }
        Ok(())
    }

    /// magit-diff-set-context: `+` / `-` / `0` change `-U<n>` of the diffs
    /// of the buffer shown.
    pub(super) fn set_diff_context(&mut self, f: impl Fn(usize) -> Option<usize>) {
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

}
