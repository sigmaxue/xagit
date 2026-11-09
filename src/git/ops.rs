//! git/ops.rs — high-level git operations mirroring Magit commands.
//!
//! Each function corresponds to a Magit command and uses the same underlying
//! git invocations documented in the Elisp docstrings (e.g. "(git reset --hard REV)").

use super::exec::Git;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

impl Git {
    // ---------------------------------------------------------------- staging

    /// magit-stage: `git add -- <file>`
    pub fn stage(&self, file: &str) -> Result<()> {
        self.git_interactive(&["add", "--", file])
    }

    /// magit-stage-modified ("S"): stage all modified and deleted tracked files,
    /// plus untracked files (magit stages untracked via `git add` too).
    /// Implementation mirrors magit-stage-modified: `git add -u` then `git add` untracked.
    pub fn stage_all(&self) -> Result<()> {
        self.git_interactive(&["add", "-u"])?;
        let untracked = self.untracked_files();
        for f in untracked {
            self.git_interactive(&["add", "--", &f])?;
        }
        Ok(())
    }

    /// magit-unstage-1: before first commit `git rm --cached -- <file>`,
    /// otherwise `git reset HEAD -- <file>`.
    pub fn unstage(&self, file: &str) -> Result<()> {
        if self.no_commit() {
            self.git_interactive(&["rm", "--cached", "--", file])
        } else {
            self.git_interactive(&["reset", "HEAD", "--", file])
        }
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

    /// magit-discard for staged change: reset then checkout.
    pub fn discard_staged(&self, file: &str) -> Result<()> {
        self.unstage(file)?;
        if Path::new(file).exists() || true {
            // file may be deleted in HEAD; ignore checkout failure
            let _ = self.git_interactive(&["checkout", "HEAD", "--", file]);
        }
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

    /// magit-commit-extend ("c e"): `git commit --amend --no-edit <args>`
    pub fn commit_extend(&self, args: &[String]) -> Result<()> {
        let mut full: Vec<String> = vec!["commit".into(), "--amend".into(), "--no-edit".into()];
        full.extend(args.iter().cloned());
        let strs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
        self.git_interactive(&strs)
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

    /// magit-merge-nocommit: `git merge --no-commit <rev>`
    pub fn merge_nocommit(&self, rev: &str) -> Result<()> {
        self.git_interactive(&["merge", "--no-commit", rev])
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
    pub fn tag_create(&self, name: &str, rev: &str, annotate: bool, msg: Option<&str>) -> Result<()> {
        let mut args: Vec<&str> = vec!["tag"];
        if annotate || msg.is_some() {
            args.push("-a");
        }
        if let Some(m) = msg {
            args.push("-m");
            args.push(m);
        }
        args.push(name);
        args.push(rev);
        self.git_interactive(&args)
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

    /// magit-gitignore ("i t"): append pattern to .gitignore in toplevel
    pub fn ignore(&self, pattern: &str) -> Result<()> {
        let f = self.toplevel.join(".gitignore");
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
}
