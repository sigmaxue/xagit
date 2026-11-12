//! The suffixes of magit-stash (magit-stash.el): stashes and snapshots
//! created like Magit (magit-stash-save: the stash commits made with
//! commit-tree, then the changes removed), applying and popping with
//! the fallbacks of magit-stash--apply-1, and transforming stashes.

use super::*;
use crate::git::ops::Untracked;
use anyhow::bail;

/// What magit-stash-save keeps (its KEEP argument).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keep {
    /// nil: reset the index and the worktree
    Nothing,
    /// `index`: checkout the worktree from the index
    Index,
    /// `worktree`: reverse-apply the staged changes
    Worktree,
    /// t: a snapshot, nothing removed
    All,
}

impl App {
    /// The suffixes of magit-stash and magit-stash-push.
    pub(super) fn stash_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let untracked = || {
            // magit-stash-read-untracked
            let prefix = self.prefix_numeric();
            if prefix == 16 || args.iter().any(|a| a == "--all") {
                Untracked::All
            } else if prefix == 4 || args.iter().any(|a| a == "--include-untracked") {
                Untracked::Include
            } else {
                Untracked::No
            }
        };
        let untracked = untracked();
        match cmd {
            "stash-both" => {
                if self.git.merge_in_progress()
                    && !self.magit_y_or_n_p(
                        "Stashing and resetting during a merge conflict.  Applying the resulting stash won't restore the merge state.  Proceed anyway? ",
                        None,
                    )?
                {
                    self.set_msg("Abort");
                    return Ok(());
                }
                let Some(msg) = self.stash_read_message()? else { return Ok(()) };
                self.stash_save(&msg, true, true, untracked, Keep::Nothing)?;
            }
            "stash-index" => {
                let Some(msg) = self.stash_read_message()? else { return Ok(()) };
                self.stash_save(&msg, true, false, Untracked::No, Keep::Worktree)?;
            }
            "stash-worktree" => {
                let Some(msg) = self.stash_read_message()? else { return Ok(()) };
                self.stash_save(&msg, false, true, untracked, Keep::Index)?;
            }
            "stash-keep-index" => {
                let Some(msg) = self.stash_read_message()? else { return Ok(()) };
                self.stash_save(&msg, true, true, untracked, Keep::Index)?;
            }
            // magit-snapshot-save
            "snapshot-both" | "snapshot-index" | "snapshot-worktree" => {
                let msg = format!("WIP on {}", self.stash_summary());
                match cmd {
                    "snapshot-both" => self.stash_save(&msg, true, true, untracked, Keep::All)?,
                    "snapshot-index" => self.stash_save(&msg, true, false, Untracked::No, Keep::All)?,
                    _ => self.stash_save(&msg, false, true, untracked, Keep::All)?,
                }
            }
            // magit-stash-push: `git stash push ARGS -- FILES`
            "stash-push" => {
                let mut a: Vec<String> = vec!["stash".into(), "push".into()];
                let mut files: Option<Vec<String>> = None;
                for arg in args {
                    match arg.strip_prefix("-- ") {
                        Some(f) => files = Some(f.split(' ').filter(|f| !f.is_empty()).map(str::to_string).collect()),
                        None => a.push(arg.clone()),
                    }
                }
                if let Some(files) = files {
                    a.push("--".into());
                    a.extend(files);
                }
                let strs: Vec<&str> = a.iter().map(String::as_str).collect();
                let res = self.git.git_interactive(&strs);
                self.done("", res.map(|()| String::new()));
            }
            "stash-apply" | "stash-pop" => {
                let action = if cmd == "stash-apply" { "apply" } else { "pop" };
                let prompt = if cmd == "stash-apply" { "Apply stash" } else { "Pop stash" };
                let Some(stash) = self.read_stash(prompt)? else { return Ok(()) };
                self.stash_apply_1(action, &stash)?;
                self.refresh();
            }
            // magit-stash-branch
            "stash-branch" => {
                let Some(stash) = self.read_stash("Branch stash")? else { return Ok(()) };
                let Some(branch) = self.read_string_ns("Branch name")? else { return Ok(()) };
                let res = self.git.git_interactive(&["stash", "branch", &branch, &stash]);
                self.done("", res.map(|()| String::new()));
            }
            // magit-stash-branch-here: a branch at the current branch (or
            // HEAD), the stash applied to it
            "stash-branch-here" => {
                let Some(stash) = self.read_stash("Branch stash")? else { return Ok(()) };
                let Some(branch) = self.read_string_ns("Branch name")? else { return Ok(()) };
                let start = self.git.current_branch().unwrap_or_else(|| "HEAD".into());
                if let Err(e) = self.git.git_interactive(&["checkout", "-b", &branch, &start]) {
                    self.set_msg(&format!("{e:#}"));
                }
                self.branch_maybe_adjust_upstream(&branch, &start);
                self.stash_apply_1("apply", &stash)?;
                self.refresh();
            }
            // magit-stash-format-patch: 0001-SUBJECT.patch
            "stash-format-patch" => {
                let Some(stash) = self.read_stash("Create patch from stash")? else { return Ok(()) };
                let name = self.git.git_string(&["log", "-1", "--format=0001-%f.patch", &stash]).unwrap_or_default();
                let patch = self.git.git_output(&["stash", "show", "-p", &stash]).map(|o| o.stdout).unwrap_or_default();
                std::fs::write(self.git.toplevel.join(name.trim_end_matches('\n')), patch)?;
                self.refresh();
            }
            _ => {}
        }
        Ok(())
    }

    /// magit-stash-summary: "BRANCH: HASH SUBJECT".
    fn stash_summary(&self) -> String {
        let branch = self.git.current_branch().unwrap_or_else(|| "(no branch)".into());
        let head = self.git.git_string(&["show", "-s", "--format=%h %s", "HEAD"]).unwrap_or_default();
        format!("{branch}: {}", head.trim_end_matches('\n'))
    }

    /// magit-stash-read-message: the message git would use when empty,
    /// that and "On BRANCH: " the future history.
    fn stash_read_message(&mut self) -> Result<Option<String>> {
        let branch = self.git.current_branch().unwrap_or_else(|| "(no branch)".into());
        let head = self.git.git_string(&["show", "-s", "--format=%h %s", "HEAD"]).unwrap_or_default();
        let default = format!("On {branch}: {}", head.trim_end_matches('\n'));
        let Some(msg) = self.read_line_hist("Stash message (default: On…:…): ", Some(&default), Hist::Minibuffer, None)?
        else {
            return Ok(None);
        };
        Ok(Some(if msg.is_empty() { default } else { msg }))
    }

    /// magit-commit-tree: a commit of TREE (else the index's) with
    /// PARENTS, signing disabled.
    fn stash_commit_tree(&self, env: &[(&str, &str)], message: &str, parents: &[&str]) -> Result<String> {
        let out = self.git.git_output_env(env, &["write-tree"])?;
        let tree = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || tree.is_empty() {
            bail!("Cannot write the tree");
        }
        let mut a = vec!["commit-tree", "--no-gpg-sign", "-m", message];
        for p in parents {
            a.extend(["-p", p]);
        }
        a.push(&tree);
        let out = self.git.git_output_env(env, &a)?;
        let commit = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || commit.is_empty() {
            bail!("Cannot create a commit");
        }
        Ok(commit)
    }

    /// magit-update-files: FILES added to or removed from the index.
    fn update_files(&self, env: &[(&str, &str)], files: &[String]) -> bool {
        let mut a = vec!["update-index", "--add", "--remove", "--ignore-skip-worktree-entries", "--"];
        a.extend(files.iter().map(String::as_str));
        self.git.git_output_env(env, &a).is_ok_and(|o| o.status.success())
    }

    /// magit-untracked-files: ignored ones too with ALL.
    fn untracked_list(&self, all: bool, files: &[String]) -> Vec<String> {
        let mut a = vec!["ls-files", "-z", "--others"];
        if !all {
            a.push("--exclude-standard");
        }
        a.push("--");
        a.extend(files.iter().map(String::as_str));
        self.git.git_lines(&a)
    }

    /// magit-stash-create: the stash commit (its parents HEAD, or a
    /// "pre-stash index" commit when only the worktree is stashed, the
    /// index commit and the commit of the untracked files).
    fn stash_create(&self, message: &str, index: bool, worktree: bool, untracked: Untracked) -> Result<String> {
        if self.git.git_string(&["rev-parse", "--verify", "-q", "HEAD"]).unwrap_or_default().trim().is_empty() {
            bail!("You do not have the initial commit yet");
        }
        let summary = self.stash_summary();
        let mut head = "HEAD".to_string();
        if worktree && !index {
            head = self
                .stash_commit_tree(&[], "pre-stash index", &["HEAD"])
                .map_err(|_| anyhow::anyhow!("Cannot save the current index state"))?;
        }
        let index_commit = self
            .stash_commit_tree(&[], &format!("index on {summary}"), &[&head])
            .map_err(|_| anyhow::anyhow!("Cannot save the current index state"))?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let temp = |n: u8| self.git.gitdir.join(format!("index.magit.{}{nanos}{n}", std::process::id()));
        let mut untracked_commit = None;
        if untracked != Untracked::No {
            let files = self.untracked_list(untracked == Untracked::All, &[]);
            if !files.is_empty() {
                // magit-with-temp-index nil: an empty index
                let file = temp(0);
                let path = file.to_string_lossy().into_owned();
                let env = [("GIT_INDEX_FILE", path.as_str())];
                let res = if self.update_files(&env, &files) {
                    self.stash_commit_tree(&env, &format!("untracked files on {summary}"), &[])
                } else {
                    Err(anyhow::anyhow!(""))
                };
                let _ = std::fs::remove_file(&file);
                untracked_commit = Some(res.map_err(|_| anyhow::anyhow!("Cannot save the untracked files"))?);
            }
        }
        // magit-with-temp-index INDEX "-m"
        let file = temp(1);
        let path = file.to_string_lossy().into_owned();
        let res = (|| {
            if !self.git.git_ok(&["read-tree", "-m", &index_commit, &format!("--index-output={path}")]) {
                bail!("Cannot read the index state");
            }
            let env = [("GIT_INDEX_FILE", path.as_str())];
            if worktree {
                let changed = self.git.git_output_env(&env, &["diff", "-z", "--name-only", &head])?;
                let changed: Vec<String> = String::from_utf8_lossy(&changed.stdout)
                    .split('\0')
                    .filter(|f| !f.is_empty())
                    .map(str::to_string)
                    .collect();
                if !self.update_files(&env, &changed) {
                    bail!("Cannot save the current worktree state");
                }
            }
            let mut parents = vec![head.as_str(), index_commit.as_str()];
            parents.extend(untracked_commit.as_deref());
            self.stash_commit_tree(&env, message, &parents)
                .map_err(|_| anyhow::anyhow!("Cannot save the current worktree state"))
        })();
        let _ = std::fs::remove_file(&file);
        res
    }

    /// magit-stash-save: store a stash of the INDEX, the WORKTREE and
    /// the UNTRACKED files (if there are such changes) in refs/stash,
    /// then remove the changes but those KEEP says.
    fn stash_save(&mut self, message: &str, index: bool, worktree: bool, untracked: Untracked, keep: Keep) -> Result<()> {
        let changes = (index && !self.git.staged_files().is_empty())
            || (worktree && !self.git.unstaged_files().is_empty())
            || (untracked != Untracked::No && !self.untracked_list(untracked == Untracked::All, &[]).is_empty());
        if !changes {
            let kind = if !index {
                "unstaged"
            } else if !worktree {
                "staged"
            } else {
                "local"
            };
            self.set_msg(&format!("No {kind} changes to save"));
            return Ok(());
        }
        let stash = match self.stash_create(message, index, worktree, untracked) {
            Ok(s) => s,
            Err(e) => {
                self.set_msg(&format!("{e:#}"));
                return Ok(());
            }
        };
        // magit-stash-store / magit-update-ref
        let old = self.git.git_string(&["rev-parse", "--verify", "-q", "refs/stash"]).unwrap_or_default();
        if !self.git.git_ok(&["update-ref", "--create-reflog", "-m", message, "refs/stash", &stash, old.trim()]) {
            self.set_msg(&format!("Cannot update refs/stash with {stash}"));
            return Ok(());
        }
        match keep {
            Keep::Worktree => {
                let diff = self.git.git_output(&["diff", "--cached", "--no-ext-diff"]).map(|o| o.stdout).unwrap_or_default();
                for a in [&["apply", "--reverse", "--cached", "--ignore-space-change", "-"][..], &["apply", "--reverse", "--ignore-space-change", "-"]] {
                    if let Err(e) = self.git.git_with_input(a, &diff) {
                        self.set_msg(&format!("{e:#}"));
                    }
                }
            }
            Keep::All => {}
            Keep::Index | Keep::Nothing => {
                let res = if keep == Keep::Index {
                    self.git.git_interactive(&["checkout", "--", "."])
                } else {
                    self.git.git_interactive(&["reset", "--hard", "HEAD", "--"])
                };
                if let Err(e) = res {
                    self.set_msg(&format!("{e:#}"));
                }
                if untracked != Untracked::No {
                    let mut a = vec!["clean", "--force", "-d"];
                    if untracked == Untracked::All {
                        a.push("-x");
                    }
                    if let Err(e) = self.git.git_interactive(&a) {
                        self.set_msg(&format!("{e:#}"));
                    }
                }
            }
        }
        self.refresh();
        Ok(())
    }

    /// magit--run-git-stash: whether `git stash ARGS` succeeded (or
    /// installed conflicts).  Magit binds `LC_ALL` to an English locale so
    /// that the "error: " prefix can be detected regardless of the user's
    /// language, and checks the beginning of the combined process output.
    /// Git writes that prefix to stderr while the status text goes to stdout,
    /// so we must look at stderr first (a plain stdout+stderr concatenation
    /// would put "On branch ..." first and hide a genuine failure).
    fn run_git_stash(&mut self, args: &[&str]) -> bool {
        let mut a = vec!["stash"];
        a.extend(args);
        match self.git.git_output_env(&[("LC_ALL", "C")], &a) {
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let stdout = String::from_utf8_lossy(&o.stdout);
                // `magit-process-file` with DESTINATION=t merges both streams
                // into one buffer; git emits the "error: " line first.
                let combined = format!("{stderr}{stdout}");
                crate::git::exec::record(&self.git.toplevel, &a, o.status.success(), o.status.code(), &combined);
                match o.status.code() {
                    Some(0) => true,
                    Some(1) => !combined.starts_with("error: "),
                    _ => false,
                }
            }
            Err(_) => false,
        }
    }

    /// magit-stash--apply-1: `git stash ACTION --index`, else `git stash
    /// apply`, else `git apply --3way` (the files touched by the stash
    /// that have unstaged changes staged first, if confirmed) or
    /// `--reject`.
    fn stash_apply_1(&mut self, action: &str, stash: &str) -> Result<()> {
        if self.run_git_stash(&[action, "--index", stash]) || self.run_git_stash(&["apply", stash]) {
            // Like Magit, a stash that applied with conflicts is a state,
            // not a failed command: say what to do next.
            if let Some(hint) = self.conflict_stopped_hint() {
                self.set_msg(hint);
            }
            return Ok(());
        }
        let range = format!("{stash}^..{stash}");
        let stashed = self.git.git_lines(&["diff", "-z", "--name-only", &range, "--"]);
        let mut a = vec!["diff-files", "-z", "--name-only", "--ignore-submodules", "--"];
        a.extend(stashed.iter().map(String::as_str));
        let mut conflicts = self.git.git_lines(&a);
        for f in self.untracked_list(true, &stashed) {
            if !conflicts.contains(&f) {
                conflicts.push(f);
            }
        }
        conflicts.sort();
        let arg = if conflicts.is_empty() || self.options.no_confirm("stash-apply-3way") {
            Some("--3way")
        } else {
            let files: String = conflicts.iter().map(|f| format!("  {f}")).collect::<Vec<_>>().join("\n");
            let plural = if conflicts.len() > 1 { "s" } else { "" };
            let prompt = format!(
                "Could not apply stash because of unstaged changes.\n\nTo do a three-way merge, these files have to be staged\n{files}\n\n[s] stage file{plural} and apply with \"git apply --3way\", \n[r] apply with \"git apply --reject\", or \n[c] cancel "
            );
            match self.read_char_choice(&prompt, &['s', 'r', 'c'])? {
                Some('s') => Some("--3way"),
                Some('r') => Some("--reject"),
                _ => None,
            }
        };
        if let Some(arg) = arg {
            if arg == "--3way" && !conflicts.is_empty() {
                let mut a = vec!["add", "--"];
                a.extend(conflicts.iter().map(String::as_str));
                if let Err(e) = self.git.git_interactive(&a) {
                    self.report_error(&e);
                }
            }
            let diff = self.git.git_output(&["diff", &range]).map(|o| o.stdout).unwrap_or_default();
            if let Err(e) = self.git.git_with_input(&["apply", arg, "-"], &diff) {
                self.report_error(&e);
            }
        }
        Ok(())
    }
}
