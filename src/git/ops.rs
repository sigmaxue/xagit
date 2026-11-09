//! git/ops.rs — high-level git operations mirroring Magit commands.
//!
//! Each function corresponds to a Magit command and uses the same underlying
//! git invocations documented in the Elisp docstrings (e.g. "(git reset --hard REV)").

use super::exec::{Editor, Git};
use anyhow::{bail, Context, Result};
use std::fs;

impl Git {
    // ---------------------------------------------------------------- staging

    /// magit-stage: `git add -- <file>`
    pub fn stage(&self, file: &str) -> Result<()> {
        self.git_interactive(&["add", "--", file])
    }

    /// magit-stage-modified ("S"): `git add -u .` — stage all new content of
    /// tracked files and remove deleted ones; untracked files are left alone
    /// (they are only included with a prefix argument, `git add --all .`).
    pub fn stage_all(&self) -> Result<()> {
        self.git_interactive(&["add", "-u", "."])
    }

    /// magit-stage-modified with a prefix argument: `git add --all .`
    pub fn stage_all_with_untracked(&self) -> Result<()> {
        self.git_interactive(&["add", "--all", "."])
    }

    /// magit-unstage-1: before first commit `git rm --cached -- <file>`,
    /// otherwise `git reset HEAD -- <file>`. A staged rename is unstaged as a
    /// pair (source and destination), like Magit does for the rename section.
    pub fn unstage(&self, file: &str) -> Result<()> {
        if self.no_commit() {
            return self.git_interactive(&["rm", "--cached", "--", file]);
        }
        match self.staged_rename_source(file) {
            Some(src) => self.git_interactive(&["reset", "HEAD", "--", &src, file]),
            None => self.git_interactive(&["reset", "HEAD", "--", file]),
        }
    }

    /// Source path when `file` is the destination of a staged rename.
    pub fn staged_rename_source(&self, file: &str) -> Option<String> {
        let out = self
            .git_string(&["diff", "--cached", "--name-status", "-M", "-z"])
            .ok()?;
        let mut it = out.split('\0');
        while let Some(status) = it.next() {
            if status.starts_with('R') || status.starts_with('C') {
                let (src, dst) = (it.next()?, it.next()?);
                if dst == file && status.starts_with('R') {
                    return Some(src.to_string());
                }
            } else {
                it.next();
            }
        }
        None
    }

    /// magit-unstage-all ("U"): before first commit `git rm --cached -r .`,
    /// otherwise `git reset HEAD -- .` (actually `git reset`).
    pub fn unstage_all(&self) -> Result<()> {
        if self.no_commit() {
            self.git_interactive(&["rm", "--cached", "-r", "."])
        } else {
            self.git_interactive(&["reset", "HEAD", "--", "."])
        }
    }

    /// magit-discard for untracked file: delete the file.
    pub fn discard_untracked(&self, file: &str) -> Result<()> {
        let p = self.toplevel.join(file);
        if p.is_dir() {
            fs::remove_dir_all(&p).with_context(|| format!("removing {}", p.display()))?;
        } else {
            fs::remove_file(&p).with_context(|| format!("removing {}", p.display()))?;
        }
        Ok(())
    }

    /// magit-discard for unstaged modification: `git checkout -- <file>` (git 2.38 ok)
    pub fn discard_unstaged(&self, file: &str) -> Result<()> {
        self.git_interactive(&["checkout", "--", file])
    }

    /// magit-discard for staged change: reset then checkout. A staged rename
    /// is undone like magit-discard-files--rename: `git mv <dst> <src>` when
    /// the destination still exists, otherwise `rm --cached` + `reset`.
    pub fn discard_staged(&self, file: &str) -> Result<()> {
        if let Some(src) = self.staged_rename_source(file) {
            if self.toplevel.join(file).exists() {
                if let Some(parent) = self.toplevel.join(&src).parent() {
                    fs::create_dir_all(parent)?;
                }
                return self.git_interactive(&["mv", file, &src]);
            }
            self.git_interactive(&["rm", "--cached", "-q", "--", file])?;
            return self.git_interactive(&["reset", "-q", "--", &src]);
        }
        self.unstage(file)?;
        // file may not exist in HEAD (newly added); then it just stays untracked
        if self.git_ok(&["cat-file", "-e", &format!("HEAD:{file}")]) {
            self.git_interactive(&["checkout", "HEAD", "--", file])?;
        }
        Ok(())
    }

    /// magit-reverse on staged changes: apply the staged diff in reverse to
    /// the working tree only (`git apply --reverse --reject`), index untouched.
    pub fn reverse_staged(&self, files: &[String]) -> Result<()> {
        let mut a: Vec<&str> = vec!["diff", "--cached", "--no-color", "--binary", "--"];
        a.extend(files.iter().map(|s| s.as_str()));
        let patch = self.git_output(&a)?;
        if !patch.status.success() {
            bail!("{}", super::exec::failure(&self.toplevel, &a, &patch));
        }
        if patch.stdout.is_empty() {
            bail!("nothing staged to reverse");
        }
        self.git_with_input(&["apply", "--reverse", "--reject", "-"], &patch.stdout)?;
        Ok(())
    }

    // ---------------------------------------------------------------- commit

    /// magit-commit-create: `git commit <args>` (message via editor or -m).
    pub fn commit(&self, msg: Option<&str>, args: &[String]) -> Result<()> {
        let mut full: Vec<String> = vec!["commit".into()];
        if let Some(m) = msg {
            full.push("-m".into());
            full.push(m.to_string());
        }
        full.extend(args.iter().cloned());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    /// `git commit ARGS` with the message written in MESSAGE
    /// (magit-run-git-with-editor).
    pub fn commit_with_editor(&self, args: &[String], message: Editor) -> Result<()> {
        self.commit_with_editor_env(args, message, &[])
    }

    /// commit_with_editor with the environment ENV.
    pub fn commit_with_editor_env(&self, args: &[String], message: Editor, env: &[(&str, &str)]) -> Result<()> {
        let mut full: Vec<&str> = vec!["commit"];
        full.extend(args.iter().map(String::as_str));
        self.git_with_editors_env(Editor::User, message, env, &full)
    }

    /// magit-commit-extend ("c e"): `git commit --amend --no-edit <args>`
    pub fn commit_extend(&self, args: &[String]) -> Result<()> {
        self.commit_extend_env(args, &[])
    }

    /// commit_extend with the environment ENV (GIT_COMMITTER_DATE to keep
    /// the committer date).
    pub fn commit_extend_env(&self, args: &[String], env: &[(&str, &str)]) -> Result<()> {
        let mut full: Vec<String> = vec!["commit".into(), "--amend".into(), "--no-edit".into()];
        full.extend(args.iter().cloned());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive_env(env, &strs)
    }

    /// magit-commit-amend ("c a"): `git commit --amend <args>`
    pub fn commit_amend(&self, msg: Option<&str>, args: &[String]) -> Result<()> {
        let mut full: Vec<String> = vec!["commit".into(), "--amend".into()];
        if let Some(m) = msg {
            full.push("-m".into());
            full.push(m.to_string());
        }
        full.extend(args.iter().cloned());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    /// magit-commit-reword ("c w"): reword HEAD message only.
    /// `git commit --amend --only -m <msg>`
    pub fn commit_reword(&self, msg: &str) -> Result<()> {
        self.git_interactive(&["commit", "--amend", "--only", "-m", msg])
    }

    /// magit-commit-fixup ("c f"): `git commit --fixup=<rev>`
    pub fn commit_fixup(&self, rev: &str) -> Result<()> {
        self.git_interactive(&["commit", "--fixup", rev])
    }

    /// magit-commit-squash ("c s"): `git commit --squash=<rev>`
    pub fn commit_squash(&self, rev: &str) -> Result<()> {
        self.git_interactive(&["commit", "--squash", rev])
    }

    /// magit-commit-instant-fixup ("c F"): squash HEAD into target with rebase --autosquash.
    /// Implementation: `git rebase -i --autosquash <rev>~` sequence — simplified to
    /// `git reset --soft <rev>; git commit --amend` when target is parent chain.
    /// For fidelity we use the documented equivalent:
    ///   git commit --fixup=<rev> && git rebase -i --autosquash <rev>~
    /// Since -i requires an editor we run rebase with GIT_SEQUENCE_EDITOR=true.
    pub fn commit_instant_fixup(&self, rev: &str) -> Result<()> {
        let hash = self.rev_parse(rev)?;
        self.git_interactive(&["commit", "--fixup", &hash])?;
        let base = format!("{hash}~");
        self.rebase_autosquash(&base)
    }

    /// magit-commit-instant-squash ("c S")
    pub fn commit_instant_squash(&self, rev: &str) -> Result<()> {
        let hash = self.rev_parse(rev)?;
        self.git_interactive(&["commit", "--squash", &hash])?;
        let base = format!("{hash}~");
        self.rebase_autosquash(&base)
    }

    fn rebase_autosquash(&self, base: &str) -> Result<()> {
        use std::process::Command;
        let st = Command::new("git")
            // sequence editor: accept the generated todo list
            .env("GIT_SEQUENCE_EDITOR", "true")
            // commit-message editor for squash merges: keep combined message
            .env("GIT_EDITOR", "true")
            .args(["rebase", "-i", "--autosquash", base])
            .current_dir(&self.toplevel)
            .status()?;
        if !st.success() {
            bail!("rebase --autosquash failed: {st}");
        }
        Ok(())
    }

    // ---------------------------------------------------------------- branch

    /// magit-checkout ("b b"): `git checkout <rev>`
    pub fn checkout(&self, rev: &str) -> Result<()> {
        self.git_interactive(&["checkout", rev])
    }

    /// magit-branch-and-checkout ("b c"): `git checkout -b <name> <start>`
    pub fn branch_and_checkout(&self, name: &str, start: Option<&str>) -> Result<()> {
        match start {
            Some(s) => self.git_interactive(&["checkout", "-b", name, s]),
            None => self.git_interactive(&["checkout", "-b", name]),
        }
    }

    /// magit-branch-create ("b n"): `git branch <name> <start>`
    pub fn branch_create(&self, name: &str, start: Option<&str>) -> Result<()> {
        match start {
            Some(s) => self.git_interactive(&["branch", name, s]),
            None => self.git_interactive(&["branch", name]),
        }
    }

    /// magit-branch-delete ("b k"): `git branch -D <name>`
    pub fn branch_delete(&self, name: &str) -> Result<()> {
        self.git_interactive(&["branch", "-D", name])
    }

    /// magit-branch-rename ("b m"): `git branch -m <old> <new>`
    pub fn branch_rename(&self, old: &str, new: &str) -> Result<()> {
        self.git_interactive(&["branch", "-m", old, new])
    }

    /// magit-branch-spinoff ("b s"): create branch at HEAD tracking current;
    /// reset current to its upstream if it has unpushed commits.
    pub fn branch_spinoff(&self, new: &str) -> Result<()> {
        let cur = self
            .current_branch()
            .ok_or_else(|| anyhow::anyhow!("cannot spin off: detached HEAD"))?;
        // create + checkout new branch at current HEAD
        self.git_interactive(&["checkout", "-b", new, &cur])?;
        // reset old branch to its upstream if configured
        if let Ok(_up) = self.git_string(&[
            "rev-parse",
            "--abbrev-ref",
            &format!("{cur}@{{u}}"),
        ]) {
            let target = format!("{cur}@{{u}}");
            let _ = Command_proxy::run(self, &["branch", "-f", &cur, &target]);
        }
        Ok(())
    }

    /// magit-branch-spinout ("b S"): like spinoff but stays on current branch.
    pub fn branch_spinout(&self, new: &str) -> Result<()> {
        let cur = self
            .current_branch()
            .ok_or_else(|| anyhow::anyhow!("cannot spin out: detached HEAD"))?;
        self.git_interactive(&["branch", new, &cur])?;
        if let Ok(_) = self.git_string(&[
            "rev-parse",
            "--abbrev-ref",
            &format!("{cur}@{{u}}"),
        ]) {
            let target = format!("{cur}@{{u}}");
            let _ = self.git_interactive(&["branch", "-f", &cur, &target]);
            let _ = self.git_interactive(&["reset", "--keep", &target]);
        }
        Ok(())
    }

    // ---------------------------------------------------------------- remote

    /// magit-remote-add ("M a"): `git remote add <name> <url>` (+ -f when fetch arg set)
    pub fn remote_add(&self, name: &str, url: &str, fetch: bool) -> Result<()> {
        let mut args: Vec<&str> = vec!["remote", "add"];
        if fetch {
            args.push("-f");
        }
        args.push(name);
        args.push(url);
        self.git_interactive(&args)
    }

    /// magit-remote-rename ("M r"): `git remote rename <old> <new>`
    pub fn remote_rename(&self, old: &str, new: &str) -> Result<()> {
        self.git_interactive(&["remote", "rename", old, new])
    }

    /// magit-remote-remove ("M k"): `git remote remove <name>`
    pub fn remote_remove(&self, name: &str) -> Result<()> {
        self.git_interactive(&["remote", "remove", name])
    }

    /// magit-remote-prune ("M p"): `git fetch <remote> --prune`
    pub fn remote_prune(&self, remote: &str) -> Result<()> {
        self.git_interactive(&["fetch", remote, "--prune"])
    }

    // ---------------------------------------------------------------- fetch/pull/push

    pub fn fetch(&self, remote: &str, args: &[String]) -> Result<()> {
        let mut full: Vec<String> = vec!["fetch".into(), remote.into()];
        full.extend(args.iter().cloned());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    pub fn fetch_all(&self, prune: bool) -> Result<()> {
        if prune {
            self.git_interactive(&["fetch", "--all", "--prune"])
        } else {
            self.git_interactive(&["fetch", "--all"])
        }
    }

    /// magit-pull: `git pull <args> <remote> <branch>` (rebase args included)
    pub fn pull(&self, remote: &str, branch: &str, args: &[String]) -> Result<()> {
        let mut full: Vec<String> = vec!["pull".into()];
        full.extend(args.iter().cloned());
        full.push(remote.into());
        full.push(branch.into());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    /// magit-push ("P p"/"P u"): `git push <args> <remote> <branch>`
    pub fn push(&self, remote: &str, branch: &str, args: &[String]) -> Result<()> {
        let mut full: Vec<String> = vec!["push".into()];
        full.extend(args.iter().cloned());
        full.push(remote.into());
        full.push(branch.into());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    pub fn push_tag(&self, remote: &str, tag: &str) -> Result<()> {
        self.git_interactive(&["push", remote, &format!("refs/tags/{tag}")])
    }

    /// Configured upstream of `branch` as (remote, branch) from
    /// `branch.<b>.remote` / `branch.<b>.merge`. Remote "." means local.
    pub fn upstream_of(&self, branch: &str) -> Option<(String, String)> {
        let remote = self
            .git_string(&["config", &format!("branch.{branch}.remote")])
            .ok()?
            .trim()
            .to_string();
        let merge = self
            .git_string(&["config", &format!("branch.{branch}.merge")])
            .ok()?
            .trim()
            .to_string();
        if remote.is_empty() || merge.is_empty() {
            return None;
        }
        let merge = merge.strip_prefix("refs/heads/").unwrap_or(&merge).to_string();
        Some((remote, merge))
    }

    fn current_upstream(&self) -> Result<(String, String, String)> {
        let Some(branch) = self.current_branch() else {
            bail!("no current branch (detached HEAD)");
        };
        let Some((remote, merge)) = self.upstream_of(&branch) else {
            bail!("no upstream configured for {branch} (set it with b C u)");
        };
        Ok((branch, remote, merge))
    }

    fn run_args(&self, mut head: Vec<String>, args: &[String], tail: &[String]) -> Result<()> {
        head.extend(args.iter().cloned());
        head.extend(tail.iter().cloned());
        let strs: Vec<&str> = head.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    /// magit-fetch-from-upstream: `git fetch <args> <upstream-remote>`; returns the remote.
    pub fn fetch_upstream(&self, args: &[String]) -> Result<String> {
        let (_, remote, _) = self.current_upstream()?;
        self.run_args(vec!["fetch".into()], args, &[remote.clone()])?;
        Ok(remote)
    }

    /// magit-fetch-branch: `git fetch <args> <remote> <branch>`
    pub fn fetch_branch(&self, remote: &str, branch: &str, args: &[String]) -> Result<()> {
        self.run_args(vec!["fetch".into()], args, &[remote.into(), branch.into()])
    }

    /// magit-pull-from-upstream: `git pull <args> <remote> <merge>`
    pub fn pull_upstream(&self, args: &[String]) -> Result<()> {
        let (_, remote, merge) = self.current_upstream()?;
        self.pull(&remote, &merge, args)
    }

    /// magit-push-current-to-upstream:
    /// `git push <args> <remote> refs/heads/<branch>:refs/heads/<merge>`
    pub fn push_upstream(&self, args: &[String]) -> Result<()> {
        let (branch, remote, merge) = self.current_upstream()?;
        self.push_refspec(&remote, &branch, &merge, args)
    }

    /// magit-push-current / magit-push-other: push `src` to `<remote>/<dst>`.
    pub fn push_refspec(&self, remote: &str, src: &str, dst: &str, args: &[String]) -> Result<()> {
        let spec = format!("refs/heads/{src}:refs/heads/{dst}");
        self.run_args(vec!["push".into()], args, &[remote.into(), spec])
    }

    pub fn remote_branches(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .git_lines(&["for-each-ref", "--format=%(refname:short)", "refs/remotes/"])
            .into_iter()
            .filter(|r| !r.ends_with("/HEAD"))
            .collect();
        v.sort();
        v
    }

    /// magit-get-tracked: "origin/foo" -> ("origin", "foo"), local "foo" -> (".", "foo").
    pub fn tracked_of(&self, name: &str) -> (String, String) {
        for r in self.remotes() {
            if let Some(b) = name.strip_prefix(&format!("{r}/")) {
                if !b.is_empty() {
                    return (r, b.to_string());
                }
            }
        }
        (".".into(), name.to_string())
    }

    // ---------------------------------------------------------------- merge/rebase

    /// magit-merge-plain: `git merge <args> <rev>`
    pub fn merge(&self, rev: &str, args: &[String]) -> Result<()> {
        let mut full: Vec<String> = vec!["merge".into()];
        full.extend(args.iter().cloned());
        full.push(rev.into());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    /// magit-merge-abort: `git merge --abort`
    pub fn merge_abort(&self) -> Result<()> {
        self.git_interactive(&["merge", "--abort"])
    }

    /// magit-merge-squash: `git merge --squash <rev>`
    pub fn merge_squash(&self, rev: &str) -> Result<()> {
        self.git_interactive(&["merge", "--squash", rev])
    }

    /// magit-merge-nocommit: `git merge --no-commit --no-ff <args> <rev>`
    pub fn merge_nocommit(&self, rev: &str, args: &[String]) -> Result<()> {
        let mut a: Vec<&str> = vec!["merge", "--no-commit"];
        if !args.iter().any(|x| x == "--no-ff") {
            a.push("--no-ff");
        }
        a.extend(args.iter().map(String::as_str));
        a.push(rev);
        self.git_interactive(&a)
    }

    /// magit-rebase: `git rebase <args> <onto>`
    pub fn rebase(&self, onto: &str, args: &[String]) -> Result<()> {
        let mut full: Vec<String> = vec!["rebase".into()];
        full.extend(args.iter().cloned());
        full.push(onto.into());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    /// magit-rebase-continue ("r r"): `git rebase --continue`
    pub fn rebase_continue(&self) -> Result<()> {
        self.git_interactive(&["rebase", "--continue"])
    }

    /// magit-rebase-skip ("r s"): `git rebase --skip`
    pub fn rebase_skip(&self) -> Result<()> {
        self.git_interactive(&["rebase", "--skip"])
    }

    /// magit-rebase-abort ("r a"): `git rebase --abort`
    pub fn rebase_abort(&self) -> Result<()> {
        self.git_interactive(&["rebase", "--abort"])
    }

    // ---------------------------------------------------------------- reset

    /// magit-reset-internal: `git reset <arg> <commit> [-- <path>]`
    pub fn reset(&self, arg: Option<&str>, commit: &str, path: Option<&str>) -> Result<()> {
        let mut args: Vec<&str> = vec!["reset"];
        if let Some(a) = arg {
            args.push(a);
        }
        args.push(commit);
        if let Some(p) = path {
            args.push("--");
            args.push(p);
        }
        self.git_interactive(&args)
    }

    pub fn reset_mixed(&self, commit: &str) -> Result<()> {
        self.reset(Some("--mixed"), commit, None)
    }
    pub fn reset_soft(&self, commit: &str) -> Result<()> {
        self.reset(Some("--soft"), commit, None)
    }
    pub fn reset_hard(&self, commit: &str) -> Result<()> {
        self.reset(Some("--hard"), commit, None)
    }
    pub fn reset_keep(&self, commit: &str) -> Result<()> {
        self.reset(Some("--keep"), commit, None)
    }
    /// magit-reset-index ("X i"): `git reset <commit> -- .`
    pub fn reset_index(&self, commit: &str) -> Result<()> {
        self.reset(None, commit, Some("."))
    }
    /// magit-reset-worktree ("X w"): checkout-index from temp index at commit
    pub fn reset_worktree(&self, commit: &str) -> Result<()> {
        use std::process::Command;
        // Equivalent of magit-with-temp-index + checkout-index --all --force
        let index_file = self.gitdir.join("ngit-tmp-index");
        let env = format!("GIT_INDEX_FILE={}", index_file.display());
        let read_tree = Command::new("git")
            .envs(env_vars(&env))
            .args(["read-tree", commit])
            .current_dir(&self.toplevel)
            .status()?;
        if !read_tree.success() {
            bail!("read-tree failed");
        }
        let co = Command::new("git")
            .envs(env_vars(&env))
            .args(["checkout-index", "--all", "--force"])
            .current_dir(&self.toplevel)
            .status()?;
        let _ = fs::remove_file(&index_file);
        if !co.success() {
            bail!("checkout-index failed");
        }
        Ok(())
    }

    // ---------------------------------------------------------------- tag

    /// magit-tag-create ("t t"): `git tag [-a] [-m msg] <name> <rev>`
    /// magit-tag-create ("t t"): `git tag ARGS [-m MSG] NAME REV`
    pub fn tag_create(&self, name: &str, rev: &str, args: &[String], msg: Option<&str>) -> Result<()> {
        let mut a: Vec<&str> = vec!["tag"];
        a.extend(args.iter().map(String::as_str));
        if let Some(m) = msg {
            if !args.iter().any(|x| x == "--annotate" || x == "--sign" || x.starts_with("--local-user=")) {
                a.push("--annotate");
            }
            a.push("-m");
            a.push(m);
        }
        a.push(name);
        a.push(rev);
        self.git_interactive(&a)
    }

    /// magit-tag-delete ("t k"): `git tag -d <name>`
    pub fn tag_delete(&self, name: &str) -> Result<()> {
        self.git_interactive(&["tag", "-d", name])
    }

    // ---------------------------------------------------------------- revert/cherry-pick

    /// magit-revert: `git revert <args> <commits>`
    pub fn revert(&self, commits: &[String], args: &[String], no_commit: bool) -> Result<()> {
        let mut full: Vec<String> = vec!["revert".into()];
        if no_commit {
            full.push("--no-commit".into());
        }
        full.extend(args.iter().cloned());
        full.extend(commits.iter().cloned());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    /// magit-merge-commit-p: whether COMMIT has more than one parent.
    pub fn is_merge_commit(&self, commit: &str) -> bool {
        self.git_string(&["rev-list", "-1", "--parents", commit])
            .map(|s| s.split_whitespace().count() > 2)
            .unwrap_or(false)
    }

    /// magit-run-git-sequencer: `git <revert|cherry-pick> <args> <commits>`
    /// with the user's editor for the messages.
    pub fn sequencer_run(&self, command: &str, args: &[String], commits: &[String]) -> Result<()> {
        let mut full: Vec<&str> = vec![command];
        full.extend(args.iter().map(String::as_str));
        full.extend(commits.iter().map(String::as_str));
        self.git_with_editors(Editor::User, Editor::User, &full)
    }

    /// magit-cherry-copy ("A A"): `git cherry-pick <args> <commits>`
    pub fn cherry_pick(&self, commits: &[String], args: &[String]) -> Result<()> {
        let mut full: Vec<String> = vec!["cherry-pick".into()];
        full.extend(args.iter().cloned());
        full.extend(commits.iter().cloned());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    /// magit-cherry-apply ("A a"): `git cherry-pick --no-commit <commits>`
    pub fn cherry_apply(&self, commits: &[String], args: &[String]) -> Result<()> {
        let mut full: Vec<String> = vec!["cherry-pick".into(), "--no-commit".into()];
        full.extend(args.iter().cloned());
        full.extend(commits.iter().cloned());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    // ---------------------------------------------------------------- stash (git native, like Magit's)

    /// magit-stash-both ("z z"): stash index+worktree. Magit builds a stash commit itself,
    /// but the net effect equals `git stash push [-u|-a] [-m msg]`.
    pub fn stash_both(&self, msg: Option<&str>, untracked: Untracked) -> Result<()> {
        let mut args: Vec<&str> = vec!["stash", "push"];
        match untracked {
            Untracked::No => {}
            Untracked::Include => args.push("--include-untracked"),
            Untracked::All => args.push("--all"),
        }
        if let Some(m) = msg {
            args.push("-m");
            args.push(m);
        }
        self.git_interactive(&args)
    }

    /// magit-stash-index ("z i"): stash only staged changes, reverse-apply to index+worktree.
    /// Implemented like magit-stash-save(index=t, worktree=nil): stash everything then
    /// restore worktree from the stash. Equivalent net effect:
    ///   git stash push --staged (git >= 2.35), fallback via temp stash dance.
    pub fn stash_index(&self, msg: Option<&str>) -> Result<()> {
        // git 2.38 supports --staged
        let mut args: Vec<&str> = vec!["stash", "push", "--staged"];
        if let Some(m) = msg {
            args.push("-m");
            args.push(m);
        }
        self.git_interactive(&args)
    }

    /// magit-stash-worktree ("z w"): stash only unstaged changes, keep index.
    /// magit-stash-save(index=nil, worktree=t, keep='index).
    /// Equivalent: snapshot staged state, stash everything, restore staged state.
    pub fn stash_worktree(&self, msg: Option<&str>, untracked: Untracked) -> Result<()> {
        let m = msg
            .map(|s| s.to_string())
            .unwrap_or_else(|| "ngit-stash-worktree".into());
        // remember the staged tree so we can restore it afterwards
        let had_staged = !self.staged_files().is_empty();
        let staged_tree = if had_staged {
            Some(self.git_string(&["write-tree"]).unwrap_or_default())
        } else {
            None
        };
        self.stash_both(Some(&m), untracked)?;
        // restore staged state (index + worktree for those paths)
        if let Some(tree) = staged_tree.filter(|t| !t.is_empty()) {
            self.restore_index_from(&tree)?;
            self.git_interactive(&["checkout-index", "--all", "--force"])?;
        }
        Ok(())
    }

    fn restore_index_from(&self, rev: &str) -> Result<()> {
        // Write rev's tree into the real index, then refresh worktree files for it.
        self.git_interactive(&["read-tree", "--reset", rev])?;
        Ok(())
    }

    /// magit-stash-keep-index ("z x"): stash everything but keep staged changes staged.
    /// Mirrors magit-stash-save(index=t, worktree=t, keep='index): after stashing,
    /// restore the staged snapshot (stash@{0}^2) into index AND worktree.
    pub fn stash_keep_index(&self, msg: Option<&str>, untracked: Untracked) -> Result<()> {
        let m = msg
            .map(|s| s.to_string())
            .unwrap_or_else(|| "ngit-stash-keep-index".into());
        self.stash_both(Some(&m), untracked)?;
        // stash@{0}^2 is the index-state commit recorded by git stash
        if self.rev_parse("stash@{0}^2").is_ok() {
            self.restore_index_from("stash@{0}^2")?;
            // materialize staged content into the worktree
            self.git_interactive(&["checkout-index", "--all", "--force"])?;
        }
        if untracked != Untracked::No {
            // bring back untracked files that were stashed away
            let _ = self.git_interactive(&["checkout-index", "--all", "--force"]);
        }
        Ok(())
    }

    /// magit-snapshot-* ("z Z"/"z I"/"z W"): stash without removing changes —
    /// implemented by stashing then immediately popping/applying back.
    pub fn snapshot(&self, scope: SnapshotScope, untracked: Untracked) -> Result<()> {
        match scope {
            SnapshotScope::Both => self.stash_both(None, untracked)?,
            SnapshotScope::Index => self.stash_index(None)?,
            SnapshotScope::Worktree => {
                // stash unstaged only, then immediately re-apply
                self.stash_both(None, untracked)?;
                self.stash_apply(&"stash@{0}".to_string(), true)?;
                return Ok(());
            }
        }
        // for both/index: re-apply so nothing is removed from the worktree
        self.stash_apply(&"stash@{0}".to_string(), true)
    }

    /// magit-stash-apply ("z a"): `git stash apply [--index] <stash>`
    pub fn stash_apply(&self, stash: &str, index: bool) -> Result<()> {
        let mut args: Vec<&str> = vec!["stash", "apply"];
        if index {
            args.push("--index");
        }
        args.push(stash);
        self.git_interactive(&args)
    }

    /// magit-stash-pop ("z p"): `git stash pop [--index] <stash>`
    pub fn stash_pop(&self, stash: &str, index: bool) -> Result<()> {
        let mut args: Vec<&str> = vec!["stash", "pop"];
        if index {
            args.push("--index");
        }
        args.push(stash);
        self.git_interactive(&args)
    }

    /// magit-stash-drop ("z k"): `git stash drop <stash>`
    pub fn stash_drop(&self, stash: &str) -> Result<()> {
        self.git_interactive(&["stash", "drop", stash])
    }

    /// magit-stash-clear (k on stashes section): drop all stashes
    pub fn stash_clear(&self) -> Result<()> {
        let list = self.stashes();
        for (name, _) in list {
            self.git_interactive(&["stash", "drop", &name])?;
        }
        Ok(())
    }

    /// magit-stash-list ("z l"): `git stash list`
    pub fn stash_list(&self) -> Vec<String> {
        self.git_lines_pub(&["stash", "list"])
    }

    pub fn git_lines_pub(&self, args: &[&str]) -> Vec<String> {
        self.git_string(args)
            .map(|s| s.lines().map(|l| l.to_string()).collect())
            .unwrap_or_default()
    }

    /// magit-stash-show ("z v"): `git stash show -p <stash>`
    pub fn stash_show(&self, stash: &str) -> Result<String> {
        self.git_string(&["stash", "show", "-p", stash])
    }

    /// magit-stash-branch ("z b"): `git stash branch <name> <stash>`
    pub fn stash_branch(&self, name: &str, stash: &str) -> Result<()> {
        self.git_interactive(&["stash", "branch", name, stash])
    }

    // ---------------------------------------------------------------- misc

    /// magit-gitignore-in-topdir ("i t"): append pattern to .gitignore in toplevel
    pub fn ignore(&self, pattern: &str) -> Result<()> {
        self.ignore_in_subdir("", pattern)
    }

    /// magit-gitignore-in-subdir ("i s"): append pattern to `<dir>/.gitignore`
    /// (dir is relative to the toplevel and must already exist).
    pub fn ignore_in_subdir(&self, dir: &str, pattern: &str) -> Result<()> {
        let d = self.toplevel.join(dir.trim_matches('/'));
        if !d.is_dir() {
            bail!("not a directory: {dir}");
        }
        let f = d.join(".gitignore");
        let mut cur = fs::read_to_string(&f).unwrap_or_default();
        if !cur.ends_with('\n') && !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(pattern);
        cur.push('\n');
        fs::write(&f, cur)?;
        Ok(())
    }

    /// magit-file-untrack ("K"): `git rm --cached -- <file>`
    pub fn untrack(&self, file: &str) -> Result<()> {
        self.git_interactive(&["rm", "--cached", "--", file])
    }

    /// magit-git-command (":"): run arbitrary git command with inherited stdio
    pub fn run_git_command(&self, rest: &[String]) -> Result<()> {
        let strs: Vec<&str> = rest.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    /// magit-log-current etc: `git log --oneline -n <limit> [rev]`
    pub fn log(&self, rev: Option<&str>, limit: usize) -> Result<Vec<String>> {
        let mut args: Vec<String> = vec![
            "log".into(),
            "--oneline".into(),
            "--decorate".into(),
            "-n".into(),
            limit.to_string(),
        ];
        if let Some(r) = rev {
            args.push(r.into());
        }
        let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = self.git_string(&strs)?;
        Ok(out.lines().map(|l| l.to_string()).collect())
    }

    /// magit-show-commit: `git show <rev>`
    pub fn show_commit(&self, rev: &str) -> Result<String> {
        self.git_string(&["show", "--no-color", rev])
    }

    /// magit-diff staged/unstaged
    pub fn diff_staged(&self) -> Result<String> {
        self.git_string(&["diff", "--cached", "--no-color"])
    }
    pub fn diff_unstaged(&self) -> Result<String> {
        self.git_string(&["diff", "--no-color"])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Untracked {
    No,
    Include,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotScope {
    Both,
    Index,
    Worktree,
}

#[allow(dead_code)]
fn env_vars(kv: &str) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    if let Some((k, v)) = kv.split_once('=') {
        m.insert(k.to_string(), v.to_string());
    }
    m
}

/// small helper shim used inside branch_spinoff
#[allow(non_snake_case, dead_code)]
struct Command_proxy;
impl Command_proxy {
    fn run(g: &Git, args: &[&str]) -> Result<()> {
        g.git_interactive(args)
    }
}

// ---------------------------------------------------------------------------
// Magit extras: bisect / worktree / submodule / notes / ediff.
// Like every other command here these go through the git CLI, so the extra
// features keep the exact same semantics as their Magit counterparts.

impl Git {
    // ---- bisect (magit-bisect)

    pub fn bisect_active(&self) -> bool {
        self.gitdir.join("BISECT_LOG").exists()
    }

    /// `git bisect start [bad] [good...]`
    pub fn bisect_start(&self, bad: Option<&str>, good: &[String]) -> Result<String> {
        let mut a: Vec<String> = vec!["bisect".into(), "start".into()];
        if let Some(b) = bad {
            a.push(b.into());
        }
        a.extend(good.iter().cloned());
        self.git_fmt(&a)
    }

    /// `git bisect bad|good|skip [rev]`
    pub fn bisect_mark(&self, state: &str, rev: Option<&str>) -> Result<String> {
        if !matches!(state, "bad" | "good" | "skip") {
            bail!("invalid bisect state: {state}");
        }
        let mut a: Vec<String> = vec!["bisect".into(), state.into()];
        if let Some(r) = rev {
            a.push(r.into());
        }
        self.git_fmt(&a)
    }

    /// `git bisect reset [rev]`
    pub fn bisect_reset(&self, rev: Option<&str>) -> Result<String> {
        let mut a: Vec<String> = vec!["bisect".into(), "reset".into()];
        if let Some(r) = rev {
            a.push(r.into());
        }
        self.git_fmt(&a)
    }

    /// `git bisect log` — also serves as magit-bisect-visualize's text view.
    pub fn bisect_log(&self) -> Result<String> {
        self.git_fmt(&["bisect".into(), "log".into()])
    }

    /// `git bisect run sh -c <command>`
    pub fn bisect_run(&self, command: &str) -> Result<String> {
        self.git_fmt(&[
            "bisect".into(),
            "run".into(),
            "sh".into(),
            "-c".into(),
            command.into(),
        ])
    }

    // ---- worktree (magit-worktree)

    pub fn worktree_list(&self) -> Result<String> {
        self.git_fmt(&["worktree".into(), "list".into()])
    }

    /// Paths from `git worktree list --porcelain`.
    pub fn worktree_paths(&self) -> Vec<String> {
        self.git_lines(&["worktree", "list", "--porcelain"])
            .iter()
            .filter_map(|l| l.strip_prefix("worktree ").map(str::to_string))
            .collect()
    }

    /// `git worktree add [-b <branch>] <path> [commit-ish]`
    pub fn worktree_add(
        &self,
        path: &str,
        branch: Option<&str>,
        new_branch: bool,
        start: Option<&str>,
    ) -> Result<String> {
        let mut a: Vec<String> = vec!["worktree".into(), "add".into()];
        if new_branch {
            let Some(b) = branch else {
                bail!("worktree branch requires a branch name")
            };
            a.push("-b".into());
            a.push(b.into());
        }
        a.push(path.into());
        match start {
            Some(s) => a.push(s.into()),
            None if !new_branch => {
                if let Some(b) = branch {
                    a.push(b.into());
                }
            }
            None => {}
        }
        self.git_fmt(&a)
    }

    pub fn worktree_move(&self, from: &str, to: &str) -> Result<String> {
        self.git_fmt(&["worktree".into(), "move".into(), from.into(), to.into()])
    }

    pub fn worktree_remove(&self, path: &str, force: bool) -> Result<String> {
        let mut a: Vec<String> = vec!["worktree".into(), "remove".into()];
        if force {
            a.push("--force".into());
        }
        a.push(path.into());
        self.git_fmt(&a)
    }

    pub fn worktree_prune(&self) -> Result<String> {
        self.git_fmt(&["worktree".into(), "prune".into(), "--verbose".into()])
    }

    // ---- submodule (magit-submodule)

    pub fn submodule_list(&self) -> Result<String> {
        self.git_fmt(&["submodule".into(), "status".into()])
    }

    /// Paths declared in .gitmodules.
    pub fn submodule_paths(&self) -> Vec<String> {
        self.git_lines(&[
            "config",
            "-f",
            ".gitmodules",
            "--get-regexp",
            r"^submodule\..*\.path$",
        ])
        .iter()
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_string))
        .collect()
    }

    /// `git -c protocol.file.allow=always submodule add <url> <path>`
    /// (the file protocol exception keeps local submodules usable).
    pub fn submodule_add(&self, url: &str, path: &str) -> Result<String> {
        self.git_fmt(&[
            "-c".into(),
            "protocol.file.allow=always".into(),
            "submodule".into(),
            "add".into(),
            url.into(),
            path.into(),
        ])
    }

    pub fn submodule_init(&self) -> Result<String> {
        self.git_fmt(&["submodule".into(), "init".into()])
    }

    pub fn submodule_update(&self, init: bool, recursive: bool) -> Result<String> {
        let mut a: Vec<String> = vec![
            "-c".into(),
            "protocol.file.allow=always".into(),
            "submodule".into(),
            "update".into(),
        ];
        if init {
            a.push("--init".into());
        }
        if recursive {
            a.push("--recursive".into());
        }
        self.git_fmt(&a)
    }

    pub fn submodule_sync(&self) -> Result<String> {
        self.git_fmt(&["submodule".into(), "sync".into(), "--recursive".into()])
    }

    pub fn submodule_deinit(&self, path: &str) -> Result<String> {
        self.git_fmt(&[
            "submodule".into(),
            "deinit".into(),
            "-f".into(),
            "--".into(),
            path.into(),
        ])
    }

    /// magit-submodule-remove: `git rm -f -- <path>` plus .git/modules cleanup.
    pub fn submodule_remove(&self, path: &str) -> Result<String> {
        let out = self.git_fmt(&["rm".into(), "-f".into(), "--".into(), path.into()])?;
        let modules = self.gitdir.join("modules").join(path);
        if modules.is_dir() {
            std::fs::remove_dir_all(&modules)
                .with_context(|| format!("removing {}", modules.display()))?;
        }
        Ok(out)
    }

    // ---- notes (magit-notes)

    pub fn notes_list(&self) -> Result<String> {
        self.git_fmt(&["notes".into(), "list".into()])
    }

    pub fn notes_show(&self, rev: &str) -> Result<String> {
        self.git_fmt(&["notes".into(), "show".into(), rev.into()])
    }

    /// Single-line note input (TUI prompts are one line, like commit messages).
    pub fn notes_add(&self, rev: &str, message: &str) -> Result<String> {
        self.git_fmt(&[
            "notes".into(),
            "add".into(),
            "-f".into(),
            "-m".into(),
            message.into(),
            rev.into(),
        ])
    }

    pub fn notes_remove(&self, rev: &str) -> Result<String> {
        self.git_fmt(&["notes".into(), "remove".into(), rev.into()])
    }

    pub fn notes_prune(&self) -> Result<String> {
        self.git_fmt(&["notes".into(), "prune".into()])
    }

    // ---- ediff / diff views

    pub fn diff_range(&self, from: &str, to: &str) -> Result<String> {
        self.git_fmt(&["diff".into(), "--no-color".into(), from.into(), to.into()])
    }

    pub fn diff_file(&self, file: &str, cached: bool) -> Result<String> {
        let mut a: Vec<String> = vec!["diff".into(), "--no-color".into()];
        if cached {
            a.push("--cached".into());
        }
        a.push("--".into());
        a.push(file.into());
        self.git_fmt(&a)
    }

    /// Diff (change): the patch of one commit.
    pub fn diff_change(&self, rev: &str) -> Result<String> {
        self.git_fmt(&[
            "show".into(),
            "--no-color".into(),
            "--format=fuller".into(),
            rev.into(),
        ])
    }

    // ---- subtree (magit-subtree)

    /// Prefixes recorded in subtree merge commits.
    pub fn subtree_prefixes(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .git_lines(&["log", "--all", "--grep=git-subtree-dir:", "--format=%B"])
            .iter()
            .filter_map(|l| l.strip_prefix("git-subtree-dir:").map(|p| p.trim().to_string()))
            .filter(|p| !p.is_empty())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn subtree_add(&self, prefix: &str, repo: &str, rev: &str) -> Result<String> {
        self.git_fmt(&[
            "subtree".into(),
            "add".into(),
            format!("--prefix={prefix}"),
            repo.into(),
            rev.into(),
        ])
    }

    pub fn subtree_pull(&self, prefix: &str, repo: &str, rev: &str) -> Result<String> {
        self.git_fmt(&[
            "subtree".into(),
            "pull".into(),
            format!("--prefix={prefix}"),
            repo.into(),
            rev.into(),
            "-m".into(),
            format!("Merge subtree {prefix}"),
        ])
    }

    pub fn subtree_push(&self, prefix: &str, repo: &str, rev: &str) -> Result<String> {
        self.git_fmt(&[
            "subtree".into(),
            "push".into(),
            format!("--prefix={prefix}"),
            repo.into(),
            rev.into(),
        ])
    }

    pub fn subtree_split(&self, prefix: &str, branch: &str) -> Result<String> {
        self.git_fmt(&[
            "subtree".into(),
            "split".into(),
            format!("--prefix={prefix}"),
            "-b".into(),
            branch.into(),
        ])
    }

    // ---- patches (magit-am / magit-format-patch)

    /// `git format-patch -o <outdir> <range>`; stdout is the generated file list.
    pub fn patch_format(&self, range: &str, outdir: &str) -> Result<String> {
        self.git_fmt(&[
            "format-patch".into(),
            "-o".into(),
            outdir.into(),
            range.into(),
        ])
    }

    pub fn patch_apply_am(&self, path: &str, three_way: bool) -> Result<String> {
        let mut a: Vec<String> = vec!["am".into()];
        if three_way {
            a.push("-3".into());
        }
        a.push(path.into());
        self.git_fmt(&a)
    }

    /// Apply a patch to the worktree only (`git apply`), without committing.
    pub fn patch_apply_worktree(&self, path: &str) -> Result<String> {
        self.git_fmt(&["apply".into(), path.into()])
    }

    pub fn patch_am_abort(&self) -> Result<String> {
        self.git_fmt(&["am".into(), "--abort".into()])
    }

    pub fn patch_am_continue(&self) -> Result<String> {
        self.git_fmt(&["am".into(), "--continue".into()])
    }

    pub fn patch_am_show(&self) -> Result<String> {
        self.git_fmt(&["am".into(), "--show-current-patch".into()])
    }

    // ---- cherries / reflog / file log / branch configuration

    /// `git cherry -v <upstream> [head]`
    pub fn cherries(&self, upstream: Option<&str>, head: Option<&str>) -> Result<String> {
        let up = upstream
            .map(str::to_string)
            .or_else(|| self.upstream_shortname());
        let Some(up) = up else {
            bail!("no upstream configured for cherries");
        };
        let mut a: Vec<String> = vec!["cherry".into(), "-v".into(), up];
        if let Some(h) = head {
            a.push(h.into());
        }
        self.git_fmt(&a)
    }

    /// `git reflog show --date=iso -n <limit> [rev]`
    pub fn reflog(&self, rev: Option<&str>, limit: usize) -> Result<String> {
        let mut a: Vec<String> = vec![
            "reflog".into(),
            "show".into(),
            "--date=iso".into(),
            "-n".into(),
            limit.to_string(),
        ];
        if let Some(r) = rev {
            a.push(r.into());
        }
        self.git_fmt(&a)
    }

    /// Log (change): history of one file.
    pub fn log_file(&self, path: &str, limit: usize) -> Result<String> {
        self.git_fmt(&[
            "log".into(),
            "--oneline".into(),
            "--follow".into(),
            "-n".into(),
            limit.to_string(),
            "--".into(),
            path.into(),
        ])
    }

    pub fn branch_config(&self) -> Result<String> {
        self.git_fmt(&["config".into(), "--get-regexp".into(), r"^branch\.".into()])
    }

    pub fn branch_set_upstream(&self, upstream: &str) -> Result<String> {
        self.git_fmt(&["branch".into(), format!("--set-upstream-to={upstream}")])
    }

    pub fn branch_unset_upstream(&self) -> Result<String> {
        self.git_fmt(&["branch".into(), "--unset-upstream".into()])
    }

    /// Set `branch.<current>.<key>` (Magit branch configure infixes).
    pub fn branch_set_config(&self, key: &str, value: &str) -> Result<String> {
        let Some(b) = self.current_branch() else {
            bail!("cannot configure a branch on detached HEAD");
        };
        self.git_fmt(&["config".into(), format!("branch.{b}.{key}"), value.into()])
    }
}

// ---------------------------------------------------------------------------
// Commands whose keys previously meant something else than in Magit.

impl Git {
    // ---- push (magit-push)

    /// magit-get-push-remote: `branch.<b>.pushRemote`, else `remote.pushDefault`.
    pub fn push_remote_of(&self, branch: &str) -> Option<String> {
        [format!("branch.{branch}.pushRemote"), "remote.pushDefault".to_string()]
            .iter()
            .find_map(|k| {
                self.git_string(&["config", k])
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            })
    }

    /// magit-push-current-to-pushremote ("P p"); returns the remote used.
    pub fn push_pushremote(&self, args: &[String]) -> Result<String> {
        let Some(branch) = self.current_branch() else {
            bail!("no current branch (detached HEAD)");
        };
        let Some(remote) = self.push_remote_of(&branch) else {
            bail!("no push-remote configured for {branch} (set it with b C p)");
        };
        self.push_refspec(&remote, &branch, &branch, args)?;
        Ok(remote)
    }

    /// magit-push-tags ("P t"): `git push <args> <remote> --tags`
    pub fn push_tags(&self, remote: &str, args: &[String]) -> Result<()> {
        self.run_args(vec!["push".into()], args, &[remote.into(), "--tags".into()])
    }

    /// magit-push-matching ("P m"): `git push <args> <remote> :`
    pub fn push_matching(&self, remote: &str, args: &[String]) -> Result<()> {
        self.run_args(vec!["push".into()], args, &[remote.into(), ":".into()])
    }

    /// magit-push-refspecs ("P r"): `git push <args> <remote> <refspec>...`
    pub fn push_refspecs(&self, remote: &str, refspecs: &[String], args: &[String]) -> Result<()> {
        if refspecs.is_empty() {
            bail!("no refspecs given");
        }
        let mut tail = vec![remote.to_string()];
        tail.extend(refspecs.iter().cloned());
        self.run_args(vec!["push".into()], args, &tail)
    }

    // ---- merge (magit-merge)

    /// magit-merge-editmsg ("m e"): `git merge --edit --no-ff [args] <rev>`.
    /// Without an editor buffer the message is given on the prompt (`-m`).
    pub fn merge_with_message(&self, rev: &str, message: &str, args: &[String]) -> Result<()> {
        let mut a: Vec<String> = vec!["merge".into(), "--no-ff".into()];
        a.extend(args.iter().filter(|x| *x != "--ff-only" && *x != "--no-ff").cloned());
        a.push("-m".into());
        a.push(message.into());
        a.push(rev.into());
        let strs: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
    }

    /// magit-merge-preview ("m p"): `git merge-tree <base> HEAD <rev>`.
    pub fn merge_preview(&self, rev: &str) -> Result<String> {
        let base = self.git_string(&["merge-base", "HEAD", rev])?;
        self.git_string(&["merge-tree", base.trim(), "HEAD", rev])
    }

    /// Commit an in-progress merge with the prepared MERGE_MSG ("m m" while merging).
    pub fn merge_commit(&self) -> Result<()> {
        if !self.merge_in_progress() {
            bail!("no merge in progress");
        }
        self.git_interactive(&["commit", "--no-edit"])
    }

    // ---- rebase (magit-rebase)

    /// magit-rebase-onto-pushremote ("r p"): `git rebase <args> <pushremote>/<branch>`
    pub fn rebase_pushremote(&self, args: &[String]) -> Result<String> {
        let Some(branch) = self.current_branch() else {
            bail!("no current branch (detached HEAD)");
        };
        let Some(remote) = self.push_remote_of(&branch) else {
            bail!("no push-remote configured for {branch} (set it with b C p)");
        };
        let target = format!("{remote}/{branch}");
        self.rebase(&target, args)?;
        Ok(target)
    }

    /// magit-rebase-autosquash ("r f"): `git rebase -i --autosquash --keep-empty
    /// [args] <base>`, with the todo list accepted unchanged.
    pub fn rebase_autosquash_with(&self, base: &str, args: &[String]) -> Result<()> {
        let mut a: Vec<String> = vec![
            "rebase".into(),
            "-i".into(),
            "--autosquash".into(),
            "--keep-empty".into(),
        ];
        a.extend(args.iter().filter(|x| *x != "--interactive").cloned());
        a.push(base.into());
        let strs: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
        self.git_with_editors(Editor::Accept, Editor::Accept, &strs)
    }

    /// Default base for autosquash: merge-base with the upstream (as in Magit).
    pub fn autosquash_base(&self) -> Option<String> {
        let up = self.upstream_shortname()?;
        self.git_string(&["merge-base", &up, "HEAD"])
            .ok()
            .map(|s| s.trim().to_string())
    }

    // ---- cherry-pick / revert sequencer (magit-sequencer-*)

    pub fn sequencer_command(&self) -> Result<&'static str> {
        if self.revert_in_progress() {
            return Ok("revert");
        }
        if self.cherry_in_progress() {
            return Ok("cherry-pick");
        }
        let todo = std::fs::read_to_string(self.gitdir.join("sequencer/todo")).unwrap_or_default();
        match todo.split_whitespace().next() {
            Some("revert") => Ok("revert"),
            Some(_) => Ok("cherry-pick"),
            None => bail!("no cherry-pick or revert in progress"),
        }
    }

    /// magit-sequencer-continue / -skip / -abort
    pub fn sequencer(&self, action: &str) -> Result<()> {
        if !matches!(action, "continue" | "skip" | "abort") {
            bail!("invalid sequencer action: {action}");
        }
        let cmd = self.sequencer_command()?;
        self.git_interactive(&[cmd, &format!("--{action}")])
    }

    // ---- am (magit-am)

    /// magit-am-apply-patches ("w w"): `git am <args> <files>`
    pub fn am_apply(&self, files: &[String], args: &[String]) -> Result<String> {
        let mut a: Vec<String> = vec!["am".into()];
        a.extend(args.iter().cloned());
        a.extend(files.iter().cloned());
        self.git_fmt(&a)
    }

    /// magit-am-skip ("w s" while applying)
    pub fn am_skip(&self) -> Result<String> {
        self.git_fmt(&["am".into(), "--skip".into()])
    }

    // ---- patch (magit-patch)

    /// magit-request-pull ("W r"): `git request-pull <start> <url> [<end>]`
    pub fn request_pull(&self, start: &str, url: &str, end: Option<&str>) -> Result<String> {
        let mut a: Vec<String> = vec!["request-pull".into(), start.into(), url.into()];
        if let Some(e) = end {
            a.push(e.into());
        }
        self.git_fmt(&a)
    }

    // ---- submodule (magit-submodule)

    /// magit-submodule-register ("o r"): `git submodule init -- <modules>`
    pub fn submodule_register(&self, modules: &[String]) -> Result<String> {
        let mut a: Vec<String> = vec!["submodule".into(), "init".into(), "--".into()];
        a.extend(modules.iter().cloned());
        self.git_fmt(&a)
    }

    fn submodule_cmd(&self, sub: &[&str], args: &[String], modules: &[String]) -> Result<String> {
        let mut a: Vec<String> = vec!["-c".into(), "protocol.file.allow=always".into(), "submodule".into()];
        a.extend(sub.iter().map(|s| s.to_string()));
        a.extend(args.iter().cloned());
        a.push("--".into());
        a.extend(modules.iter().cloned());
        self.git_fmt(&a)
    }

    /// magit-submodule-populate ("o p"): `git submodule update --init <args> -- <modules>`
    pub fn submodule_populate(&self, args: &[String], modules: &[String]) -> Result<String> {
        self.submodule_cmd(&["update", "--init"], args, modules)
    }

    /// magit-submodule-update ("o u"): `git submodule update <args> -- <modules>`
    pub fn submodule_update_with(&self, args: &[String], modules: &[String]) -> Result<String> {
        self.submodule_cmd(&["update"], args, modules)
    }

    /// magit-submodule-unpopulate ("o d"): `git submodule deinit <args> -- <modules>`
    pub fn submodule_unpopulate(&self, args: &[String], modules: &[String]) -> Result<String> {
        self.submodule_cmd(&["deinit"], args, modules)
    }

    /// magit-fetch-modules ("o f"): `git fetch --recurse-submodules <args>`
    pub fn fetch_modules(&self, args: &[String]) -> Result<String> {
        let mut a: Vec<String> = vec![
            "-c".into(),
            "protocol.file.allow=always".into(),
            "fetch".into(),
            "--recurse-submodules".into(),
        ];
        a.extend(args.iter().cloned());
        self.git_fmt(&a)
    }

    // ---- variables (magit--set-popup-variable)

    /// Cycle a repository variable through `choices`, then back to unset.
    /// Returns the new value (None = unset).
    pub fn cycle_config(&self, key: &str, choices: &[&str]) -> Result<Option<String>> {
        let cur = self
            .git_string(&["config", "--local", key])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let next = match cur.as_deref() {
            None => choices.first().copied(),
            Some(c) => choices
                .iter()
                .position(|x| *x == c)
                .and_then(|i| choices.get(i + 1))
                .copied(),
        };
        self.set_config(key, next)?;
        Ok(next.map(str::to_string))
    }

    /// `git config --local <key> <value>`, or `--unset` when value is None.
    pub fn set_config(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(v) => {
                self.git_fmt(&["config".into(), "--local".into(), key.into(), v.into()])?;
            }
            None => {
                // exit 5 when the key is not set: nothing to do
                let _ = self.git_fmt(&["config".into(), "--local".into(), "--unset".into(), key.into()]);
            }
        }
        Ok(())
    }

    // ---- diff / log arguments (magit-diff-refresh / magit-log-refresh)

    /// `git diff --no-color <args> <rest...>`
    pub fn diff_with(&self, args: &[String], rest: &[&str]) -> Result<String> {
        let mut a: Vec<&str> = vec!["diff", "--no-color"];
        a.extend(args.iter().map(|s| s.as_str()));
        a.extend_from_slice(rest);
        self.git_string(&a)
    }

    /// `git show --no-color <args> <rev>`
    pub fn show_with(&self, rev: &str, args: &[String]) -> Result<String> {
        let mut a: Vec<&str> = vec!["show", "--no-color"];
        a.extend(args.iter().map(|s| s.as_str()));
        a.push(rev);
        self.git_string(&a)
    }

    /// `git log --oneline -n <limit> <args> [rev]`. `--follow` needs exactly one
    /// path, so it is dropped here (file logs use `log_file`).
    pub fn log_with(&self, rev: Option<&str>, limit: usize, args: &[String]) -> Result<Vec<String>> {
        let mut a: Vec<String> = vec!["log".into(), "--oneline".into(), "-n".into(), limit.to_string()];
        a.extend(args.iter().filter(|x| *x != "--follow").cloned());
        if let Some(r) = rev {
            a.push(r.into());
        }
        let strs: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
        let out = self.git_string(&strs)?;
        Ok(out.lines().map(|l| l.to_string()).collect())
    }
}

// ---------------------------------------------------------------------------
// Magit suffixes that edit the rebase todo list or move commits between branches.

/// magit-rebase--perl-editor: rewrite the first `pick <abbrev>` todo line.
pub fn perl_pick_editor(abbrev: &str, replacement: &str) -> String {
    format!("perl -i -p -e '++$x if not $x and s/^pick {abbrev}/{replacement} {abbrev}/'")
}

impl Git {
    /// magit-rev-ancestor-p: `git merge-base --is-ancestor <a> <b>`
    pub fn is_ancestor(&self, a: &str, b: &str) -> bool {
        self.git_ok(&["merge-base", "--is-ancestor", a, b])
    }

    /// magit-rev-verify for commits
    pub fn rev_verify(&self, rev: &str) -> bool {
        self.git_ok(&["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")])
    }

    pub fn local_branch_exists(&self, branch: &str) -> bool {
        self.git_ok(&["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")])
    }

    /// magit-rev-abbrev
    pub fn abbrev(&self, rev: &str) -> Result<String> {
        Ok(self.git_string(&["rev-parse", "--short", rev])?.trim().to_string())
    }

    /// magit-main-branch: `init.defaultBranch`, else main/master/trunk/development.
    pub fn main_branch(&self) -> Option<String> {
        let branches = self.branches();
        let configured = self
            .git_string(&["config", "init.defaultBranch"])
            .ok()
            .map(|s| s.trim().to_string());
        configured
            .into_iter()
            .chain(["main", "master", "trunk", "development"].map(String::from))
            .find(|b| branches.contains(b))
    }

    /// magit-primary-remote: `magit.primaryRemote`, else "upstream", else "origin".
    pub fn primary_remote(&self) -> Option<String> {
        let remotes = self.remotes();
        let configured = self
            .git_string(&["config", "magit.primaryRemote"])
            .ok()
            .map(|s| s.trim().to_string());
        configured
            .into_iter()
            .chain(["upstream", "origin"].map(String::from))
            .find(|r| remotes.contains(r))
    }

    // ---- interactive rebase (magit-rebase-interactive-1)

    /// Base for rebasing COMMIT and everything after it: `<commit>^`, or None
    /// for a root commit (then `--root` is used).
    pub fn rebase_since(&self, commit: &str) -> Result<Option<String>> {
        if !self.is_ancestor(commit, "HEAD") {
            bail!("{commit} isn't an ancestor of HEAD");
        }
        let parent = format!("{commit}^");
        Ok(self.rev_verify(&parent).then_some(parent))
    }

    fn rebase_i(&self, commit: &str, args: &[String], todo: Editor, message: Editor, global: &[&str]) -> Result<()> {
        let since = self.rebase_since(commit)?;
        let mut a: Vec<String> = global.iter().map(|s| s.to_string()).collect();
        a.extend(["rebase".to_string(), "-i".to_string()]);
        a.extend(args.iter().filter(|x| *x != "--interactive").cloned());
        a.push(since.unwrap_or_else(|| "--root".into()));
        let strs: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
        self.git_with_editors(todo, message, &strs)
    }

    /// magit-rebase-interactive ("r i"): rebase COMMIT and later commits,
    /// with the todo list and messages edited in the given editors.
    pub fn rebase_interactive(&self, commit: &str, args: &[String], todo: Editor, message: Editor) -> Result<()> {
        self.rebase_i(commit, args, todo, message, &[])
    }

    /// magit-rebase-edit-commit ("r m"): stop at COMMIT so it can be amended.
    pub fn rebase_edit_commit(&self, commit: &str, args: &[String]) -> Result<()> {
        let ed = perl_pick_editor(&self.abbrev(commit)?, "edit");
        self.rebase_i(commit, args, Editor::Command(&ed), Editor::Accept, &[])
    }

    /// magit-rebase-reword-commit ("r w"): the new message is written in `message`.
    pub fn rebase_reword_commit(&self, commit: &str, args: &[String], message: Editor) -> Result<()> {
        let ed = perl_pick_editor(&self.abbrev(commit)?, "reword");
        self.rebase_i(commit, args, Editor::Command(&ed), message, &[])
    }

    /// magit-rebase-remove-commit ("r k"): drop COMMIT from the history.
    pub fn rebase_remove_commit(&self, commit: &str, args: &[String]) -> Result<()> {
        let ed = perl_pick_editor(&self.abbrev(commit)?, "noop\\n# pick");
        self.rebase_i(commit, args, Editor::Command(&ed), Editor::Accept, &["-c", "core.commentChar=#"])
    }

    /// magit-rebase-subset ("r s"): `git rebase --onto <newbase> <start> <args>`,
    /// i.e. move the commits after START onto NEWBASE.
    pub fn rebase_subset(&self, newbase: &str, start: &str, args: &[String]) -> Result<()> {
        let mut a: Vec<&str> = vec!["rebase", "--onto", newbase, start];
        a.extend(args.iter().map(|s| s.as_str()));
        self.git_interactive(&a)
    }

    /// magit-rebase-edit ("r e" while rebasing): `git rebase --edit-todo`
    pub fn rebase_edit_todo(&self, todo: Editor) -> Result<()> {
        if !self.rebase_in_progress() {
            bail!("No rebase in progress");
        }
        self.git_with_editors(todo, Editor::Accept, &["rebase", "--edit-todo"])
    }

    // ---- merge (magit-merge-absorb / magit-merge-dissolve)

    /// magit--merge-absorb: force-push BRANCH to its push target when that
    /// remote branch exists, merge it with `--no-edit`, then unset its
    /// pushRemote and delete it.
    pub fn merge_absorb(&self, branch: &str, args: &[String]) -> Result<()> {
        if let Some(remote) = self.push_remote_of(branch) {
            if self.rev_verify(&format!("refs/remotes/{remote}/{branch}")) {
                self.git_interactive(&[
                    "push",
                    "-v",
                    "--force-with-lease",
                    &remote,
                    &format!("{branch}:refs/heads/{branch}"),
                ])?;
            }
        }
        let mut a: Vec<&str> = vec!["merge"];
        a.extend(args.iter().map(|s| s.as_str()));
        a.extend(["--no-edit", branch]);
        self.git_interactive(&a)?;
        self.set_config(&format!("branch.{branch}.pushRemote"), None)?;
        self.git_interactive(&["branch", "-D", branch])
    }

    /// magit-merge-dissolve: check out INTO, then absorb the branch that was
    /// current (or merge the detached HEAD commit).
    pub fn merge_dissolve(&self, into: &str, args: &[String]) -> Result<()> {
        let current = self.current_branch();
        let head = self.rev_parse("HEAD")?;
        self.git_interactive(&["checkout", into])?;
        match current {
            Some(c) => self.merge_absorb(&c, args),
            None => {
                let mut a: Vec<&str> = vec!["merge"];
                a.extend(args.iter().map(|s| s.as_str()));
                a.push(&head);
                self.git_interactive(&a)
            }
        }
    }

    // ---- cherry moves (magit--cherry-move)

    /// Commits named by SPEC, oldest first: one commit, or a range `A..B`.
    pub fn resolve_cherries(&self, spec: &str) -> Result<Vec<String>> {
        if spec.contains("..") {
            let out = self.git_string(&["rev-list", "--reverse", spec])?;
            let v: Vec<String> = out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
            if v.is_empty() {
                bail!("no commits in {spec}");
            }
            Ok(v)
        } else {
            Ok(vec![self.rev_parse(&format!("{spec}^{{commit}}"))?])
        }
    }

    /// magit-list-containing-branches
    pub fn branches_containing(&self, commit: &str) -> Vec<String> {
        self.git_string(&["branch", "--contains", commit, "--format=%(refname:short)"])
            .map(|o| {
                o.lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty() && !l.starts_with('('))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// magit--cherry-move: cherry-pick COMMITS (oldest first) onto DST, creating
    /// DST at START_POINT when needed, then remove them from SRC (reset when
    /// they are its tip, otherwise drop them with rebase -i). Ends on SRC
    /// unless `checkout_dst`. SRC None only cherry-picks (harvest from no branch).
    pub fn cherry_move(
        &self,
        commits: &[String],
        src: Option<&str>,
        dst: &str,
        args: &[String],
        start_point: Option<&str>,
        checkout_dst: bool,
    ) -> Result<()> {
        let Some(first) = commits.first() else {
            bail!("no commits to move");
        };
        let tip = commits.last().unwrap_or(first).clone();
        let current = self.current_branch();
        if !self.local_branch_exists(dst) {
            let mut a = vec!["branch", dst];
            a.extend(start_point);
            self.git_interactive(&a)?;
        }
        if current.as_deref() != Some(dst) {
            self.git_interactive(&["checkout", dst])?;
        }
        let Some(src) = src else {
            return self.cherry_pick(commits, args);
        };
        let keep = format!("{first}^");
        self.cherry_pick(commits, args)?;
        if self.rev_parse(&tip)? == self.rev_parse(src)? {
            if self.local_branch_exists(src) {
                self.git_interactive(&[
                    "update-ref",
                    "-m",
                    &format!("reset: moving to {keep}"),
                    &format!("refs/heads/{src}"),
                    &keep,
                    &tip,
                ])?;
                if !checkout_dst {
                    self.git_interactive(&["checkout", src])?;
                }
            } else if !checkout_dst {
                // src was a detached HEAD: detach at the commit before the cherries
                self.git_interactive(&["checkout", "--detach", &keep])?;
            }
        } else {
            self.git_interactive(&["checkout", src])?;
            let abbrevs: Vec<String> = commits.iter().map(|c| self.abbrev(c)).collect::<Result<_>>()?;
            let ed = format!("perl -i -ne '/^pick ({})/ or print'", abbrevs.join("|"));
            self.git_with_editors(Editor::Command(&ed), Editor::Accept, &["rebase", "-i", &keep])?;
            if checkout_dst {
                self.git_interactive(&["checkout", dst])?;
            }
        }
        Ok(())
    }

    // ---- default branch (magit-update-default-branch)

    /// magit--get-default-branch: the primary remote and the branch its
    /// `refs/remotes/<remote>/HEAD` points at. With `update`, fetch and run
    /// `git remote set-head --auto` first.
    pub fn default_branch(&self, update: bool) -> Result<(String, Option<String>)> {
        let Some(remote) = self.primary_remote() else {
            bail!("Cannot determine primary remote");
        };
        if update {
            self.git_interactive(&["fetch", "--prune", &remote])?;
            self.git_interactive(&["remote", "set-head", "--auto", &remote])?;
        }
        let head = self
            .git_string(&["symbolic-ref", "--short", &format!("refs/remotes/{remote}/HEAD")])
            .ok()
            .map(|h| {
                let h = h.trim();
                h.strip_prefix(&format!("{remote}/")).unwrap_or(h).to_string()
            });
        if update && head.is_none() {
            bail!("Cannot determine new default branch");
        }
        Ok((remote, head))
    }

    /// magit--set-default-branch: rename local OLD to NEW (unless NEW exists)
    /// and point every branch that tracked OLD (locally or on the primary
    /// remote) at NEW.
    pub fn set_default_branch(&self, new: &str, old: &str) -> Result<()> {
        let Some(remote) = self.primary_remote() else {
            bail!("Cannot determine primary remote");
        };
        let out = self.git_string(&[
            "for-each-ref",
            "refs/heads",
            "--format=%(refname:short)\t%(upstream:short)",
        ])?;
        let mut branches: Vec<(String, String)> = out
            .lines()
            .map(|l| {
                let mut p = l.splitn(2, '\t');
                (p.next().unwrap_or("").to_string(), p.next().unwrap_or("").to_string())
            })
            .collect();
        if branches.iter().any(|(b, _)| b == old) && !branches.iter().any(|(b, _)| b == new) {
            self.git_interactive(&["branch", "-m", old, new])?;
            for e in branches.iter_mut().filter(|e| e.0 == old) {
                e.0 = new.to_string();
            }
        }
        let new_local = if self.local_branch_exists(new) {
            new.to_string()
        } else {
            format!("{remote}/{new}")
        };
        let old_remote = format!("{remote}/{old}");
        let new_remote = format!("{remote}/{new}");
        for (branch, upstream) in &branches {
            let target = if upstream == old {
                &new_local
            } else if *upstream == old_remote {
                &new_remote
            } else {
                continue;
            };
            self.git_interactive(&["branch", "--set-upstream-to", target, branch])?;
        }
        Ok(())
    }
}
