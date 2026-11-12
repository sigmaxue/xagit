//! ui/app/extra.rs — extra commands: rebase, absorb and cherry-pick helpers.
//!
//! Magit extras (bisect / worktree / submodule / notes / ediff) are kept
//! out of `run_command`'s main dispatch so each feature stays together.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

/// Magit's `magit-run-git-sequencer` reports a rebase that stops at a
/// commit as a sequence in progress, not as a failed command: the status
/// buffer shows the sequence and the transient offers Continue/Skip/Edit/
/// Abort. Git itself exits non-zero when it stops, so an `Err` that leaves
/// a rebase in progress has to be reported the same way.
const REBASE_STOPPED: &str = "rebase stopped (r r continue, r s skip, r a abort)";

impl App {
    pub(super) fn done(&mut self, ok_msg: &str, res: Result<String>) {
        match res {
            Ok(out) => {
                let first = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                self.set_msg(if first.is_empty() { ok_msg } else { first });
                self.refresh();
            }
            Err(e) => self.report_error(&e),
        }
    }

    pub(super) fn run_extra(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        match cmd {
            // ---- bisect
            "bisect-start" | "bisect-bad" | "bisect-good" | "bisect-mark" | "bisect-skip" | "bisect-reset" | "bisect-run" => {
                self.bisect_command(cmd, args)?
            }

            // ---- worktree
            "worktree-checkout" | "worktree-branch" | "worktree-move" | "worktree-delete" | "worktree-status" => {
                self.worktree_command(cmd)?
            }

            // ---- submodule
            "submodule-list" | "submodule-add" | "submodule-register" | "submodule-populate" | "submodule-update"
            | "submodule-sync" | "submodule-unpopulate" | "submodule-remove" => self.submodule_command(cmd, args)?,

            // ---- notes
            "notes-edit" | "notes-remove" | "notes-merge" | "notes-merge-commit" | "notes-merge-abort" | "notes-prune" => {
                self.notes_command(cmd, args)?
            }

            // ---- ediff (text diff buffers instead of Emacs ediff sessions)
            "ediff-dwim" => {
                // `e` with unmerged conflicts opens the built-in resolution
                // view (the A/B/both part of an Ediff merge session)
                if !self.resolve_open()? {
                    self.diff_view("dwim")?;
                }
            }
            "ediff-unstaged" | "ediff-staged" | "ediff-worktree" | "ediff-range" | "ediff-commit"
            | "ediff-stash" => {
                self.diff_view(&cmd["ediff-".len()..])?;
            }

            // ---- section info / buffer display
            // magit-describe-section
            "section-info" => {
                let line = self.current_line().cloned();
                let text = match line {
                    Some(l) => {
                        let s = self.cur_sec();
                        let sec = self.sec(s).clone();
                        let mut chain = Vec::new();
                        let mut cur = Some(s);
                        while let Some(c) = cur {
                            let id = &self.sec(c).ident;
                            chain.push(if id.is_empty() { "status".to_string() } else { id.clone() });
                            cur = self.sec(c).parent;
                        }
                        format!(
                            "Section: {}\nLines: {}-{}\nHidden: {}\nChildren: {}\nLine: {}\nKind: {:?}\nPosition: {}/{}",
                            chain.join(" < "),
                            sec.start + 1,
                            sec.end,
                            self.hidden(s),
                            sec.children.len(),
                            l.text,
                            l.kind,
                            self.cursor + 1,
                            self.lines.len(),
                        )
                    }
                    None => "No line at point".into(),
                };
                self.view_text = Some(("section info".into(), text));
            }
            "cherries" => self.cherry()?,

            // ---- subtree
            "subtree-add" | "subtree-add-commit" | "subtree-merge" | "subtree-pull" | "subtree-push" | "subtree-split" => {
                self.subtree_command(cmd, args)?
            }

            // ---- patches
            "patch-create" | "patch-apply" | "patches-apply" | "patches-apply-maildir" | "patches-continue" | "patches-skip"
            | "patches-abort" | "patch-save" | "request-pull" => self.patch_command(cmd, args)?,

            // ---- branch configuration
            "update-default-branch" => self.update_default_branch()?,
            "merge-absorb" | "merge-dissolve" => self.merge_absorb_cmd(cmd, args)?,
            "cherry-harvest" | "cherry-donate" | "cherry-spinout" | "cherry-spinoff" => {
                self.cherry_move_cmd(cmd, args)?
            }
            // the commit at point, else one selected from the log
            // (magit-rebase-interactive-1)
            "rebase-interactive" | "rebase-edit-commit" | "rebase-reword-commit" | "rebase-remove-commit" => {
                match self.commit_at_point() {
                    Some(commit) => self.rebase_commit(cmd, &commit, args)?,
                    None => {
                        let msg = match cmd {
                            "rebase-interactive" => "Type %p on a commit to rebase it and all commits above it,",
                            "rebase-edit-commit" => "Type %p on a commit to edit it,",
                            "rebase-reword-commit" => "Type %p on a commit to reword its message,",
                            _ => "Type %p on a commit to remove it,",
                        };
                        let action = SelectAction::Rebase { cmd: cmd.to_string(), args: args.to_vec() };
                        self.log_select(action, Some(msg), None);
                    }
                }
            }
            "rebase-subset" => {
                let default = self.git.upstream_shortname();
                let Some(newbase) = self.read_rev("Rebase subset onto", default.as_deref())? else {
                    return Ok(());
                };
                let msg = format!("Type %p on a commit to rebase it and commits above it onto {newbase},");
                self.log_select(SelectAction::RebaseSubset { newbase, args: args.to_vec() }, Some(&msg), None);
            }
            "rebase-edit-todo" => self.rebase_todo_edit_in_progress()?,

            _ => self.set_msg(&format!("unknown command: {cmd}")),
        }
        Ok(())
    }

    /// Report the outcome of a rebase that may stop half-way.
    pub(super) fn after_rebase(&mut self, res: Result<()>) {
        self.rebase_result(res, "rebased");
    }

    /// Report a rebase command (start, continue, skip, ...). OK_MSG is what
    /// a completed sequence reports. When git exits non-zero but leaves a
    /// rebase in progress, that is a stopped sequence, not a failed command:
    /// show the actions that resume it and leave the status buffer (with its
    /// rebase sequence) in front. The command is still in the process log.
    pub(super) fn rebase_result(&mut self, res: Result<()>, ok_msg: &str) {
        let stopped = self.git.rebase_in_progress();
        match res {
            Ok(()) if stopped => self.set_msg(REBASE_STOPPED),
            Ok(()) => self.set_msg(ok_msg),
            // Git exits non-zero when it stops at a commit. The command is
            // already in the process log ($), so don't replace the status
            // buffer with a "failed" details view as a real error would.
            Err(_) if stopped => self.set_msg(REBASE_STOPPED),
            Err(e) => self.report_error(&e),
        }
        self.refresh();
    }

    /// magit-rebase-interactive-assert: confirm rewriting published commits
    /// and rebasing a range that contains merges. With `delay_edit_confirm`,
    /// do not ask when the published branches point exactly at COMMIT.
    pub(super) fn rebase_assert(&mut self, commit: &str, delay_edit_confirm: bool, args: &[String]) -> Result<bool> {
        let since = match self.git.rebase_since(commit) {
            Ok(s) => s,
            Err(e) => {
                self.set_msg(&format!("{e:#}"));
                return Ok(false);
            }
        };
        let target = self.git.rev_parse(commit)?;
        let published = self.publishing_branches(commit);
        let all_at_commit = published
            .iter()
            .all(|b| self.git.rev_parse(b).ok().as_deref() == Some(target.as_str()));
        if !(delay_edit_confirm && all_at_commit) {
            let (m1, m2) = ("Some of these commits have already been published to ", ".\nDo you really want to modify them");
            if !self.confirm_items("rebase-published", &format!("{m1}%s{m2}"), &format!("{m1}%d public branches{m2}"), &published)? {
                return Ok(false);
            }
        }
        if !args.iter().any(|a| a.starts_with("--rebase-merges")) {
            let range = format!("{}..HEAD", since.as_deref().unwrap_or(commit));
            let merges = self.git.git_string(&["rev-list", "--merges", &range]).unwrap_or_default();
            if !merges.trim().is_empty() && !self.yes_no("Proceed despite merge in rebase range?")? {
                self.set_msg("Abort");
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// magit-merge-absorb ("m a") / magit-merge-dissolve ("m d")
    pub(super) fn merge_absorb_cmd(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let current = self.git.current_branch();
        let others: Vec<String> = self
            .git
            .branches()
            .into_iter()
            .filter(|b| Some(b) != current.as_ref())
            .collect();
        let (absorbed, res) = if cmd == "merge-absorb" {
            let Some(branch) = self.pick_rev("Absorb branch", &others, None)?.cloned() else {
                return Ok(());
            };
            if !self.confirm_absorb(&branch)? {
                return Ok(());
            }
            let res = self.git.merge_absorb(&branch, args);
            (branch, res)
        } else {
            let label = match &current {
                Some(c) => c.clone(),
                None => self.git.rev_parse("HEAD")?,
            };
            // default: the current branch's upstream when that is a local branch
            let default = current
                .as_ref()
                .and_then(|b| self.git.upstream_of(b))
                .filter(|(r, _)| r == ".")
                .map(|(_, b)| b);
            let Some(into) = self.pick_rev(&format!("Merge `{label}' into"), &others, default.as_deref())?.cloned()
            else {
                return Ok(());
            };
            if current.is_some() && !self.confirm_absorb(&label)? {
                return Ok(());
            }
            let res = self.git.merge_dissolve(&into, args);
            (label, res)
        };
        match res {
            Ok(()) => self.set_msg(&format!("merged and removed {absorbed}")),
            Err(e) => self.report_error(&e),
        }
        self.refresh();
        Ok(())
    }

    /// magit--merge-absorb asks before merging the main branch into another one.
    pub(super) fn confirm_absorb(&mut self, branch: &str) -> Result<bool> {
        if self.git.main_branch().as_deref() == Some(branch)
            && !self.yes_or_no_p(&format!("Do you really want to merge `{branch}' into another branch?"))?
        {
            self.set_msg("Abort");
            return Ok(false);
        }
        Ok(true)
    }

    /// magit-cherry-harvest / -donate / -spinout / -spinoff ("A h/d/n/s")
    pub(super) fn cherry_move_cmd(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let verb = &cmd["cherry-".len()..];
        let away = cmd != "cherry-harvest";
        let current = match self.git.current_branch() {
            Some(c) => c,
            None if cmd == "cherry-donate" => self.git.rev_parse("HEAD")?,
            None => {
                self.set_msg(&format!("Cannot {verb} cherries while HEAD is detached"));
                return Ok(());
            }
        };
        let Some(spec) = self.read_rev(&format!("{verb} cherry (commit or A..B)"), None)? else {
            return Ok(());
        };
        let commits = match self.git.resolve_cherries(&spec) {
            Ok(c) => c,
            Err(e) => {
                self.set_msg(&format!("{e:#}"));
                return Ok(());
            }
        };
        let reachable = self.git.is_ancestor(&commits[0], &current);
        if away != reachable {
            let are = if reachable { "are" } else { "are not" };
            self.set_msg(&format!("Cannot {verb} cherries that {are} reachable from HEAD"));
            return Ok(());
        }
        let n = commits.len();
        let cherries = if n == 1 { "1 cherry".to_string() } else { format!("{n} cherries") };
        let res = match cmd {
            "cherry-harvest" => {
                let branches = self.git.branches_containing(&commits[0]);
                let src = match branches.len() {
                    0 => None,
                    1 => Some(branches[0].clone()),
                    _ => match self.pick_rev(&format!("Remove {cherries} from branch"), &branches, None)? {
                        Some(b) => Some(b.clone()),
                        None => return Ok(()),
                    },
                };
                self.git.cherry_move(&commits, src.as_deref(), &current, args, None, true)
            }
            "cherry-donate" => {
                let others: Vec<String> =
                    self.git.branches().into_iter().filter(|b| *b != current).collect();
                let Some(dst) = self.pick_rev(&format!("Move {cherries} to branch"), &others, None)?.cloned() else {
                    return Ok(());
                };
                self.git.cherry_move(&commits, Some(&current), &dst, args, None, false)
            }
            _ => {
                let Some(name) =
                    self.read_line_prompt(&format!("Create branch from {cherries} named"), None)?
                else {
                    return Ok(());
                };
                if self.git.local_branch_exists(&name) {
                    self.set_msg(&format!("branch {name} already exists"));
                    return Ok(());
                }
                let default_start = self.git.upstream_shortname().unwrap_or_else(|| current.clone());
                let Some(start) = self.read_rev(
                    &format!("Create branch from {cherries} starting at"),
                    Some(&default_start),
                )?
                else {
                    return Ok(());
                };
                let checkout = cmd == "cherry-spinoff";
                self.git.cherry_move(&commits, Some(&current), &name, args, Some(&start), checkout)
            }
        };
        match res {
            Ok(()) => self.set_msg(&format!("{verb}: moved {cherries}")),
            Err(e) => self.set_msg(&format!("{e} (resolve, then A A to continue)")),
        }
        self.refresh();
        Ok(())
    }

    /// magit-update-default-branch ("b C B")
    pub(super) fn update_default_branch(&mut self) -> Result<()> {
        let old = match self.git.default_branch(false) {
            Ok((_, o)) => o,
            Err(e) => {
                self.set_msg(&format!("{e:#}"));
                return Ok(());
            }
        };
        let (remote, new) = match self.git.default_branch(true) {
            Ok((r, Some(n))) => (r, n),
            Ok((_, None)) => {
                self.set_msg("Cannot determine new default branch");
                return Ok(());
            }
            Err(e) => {
                self.set_msg(&format!("{e:#}"));
                return Ok(());
            }
        };
        let old = if old.as_deref() == Some(new.as_str()) {
            let Some(o) = self.read_line_prompt(
                &format!(
                    "Name of default branch is still `{new}', but the upstreams of some local branches \
                     might need updating. Name of upstream branches to replace with `{new}'"
                ),
                None,
            )?
            else {
                return Ok(());
            };
            o
        } else {
            let old = match old {
                Some(o) => o,
                None => {
                    let others: Vec<String> = self.git.branches().into_iter().filter(|b| *b != new).collect();
                    let Some(o) = self
                        .pick(&format!("Name of old default branch to be renamed to `{new}'"), &others, Some("master"))?
                        .cloned()
                    else {
                        return Ok(());
                    };
                    o
                }
            };
            if !self.yes_no(&format!(
                "Default branch changed from `{old}' to `{new}' on {remote}.  Do the same locally?"
            ))? {
                self.set_msg("Abort");
                return Ok(());
            }
            old
        };
        match self.git.set_default_branch(&new, &old) {
            Ok(()) => self.set_msg(&format!("default branch is now {new}")),
            Err(e) => self.report_error(&e),
        }
        self.refresh();
        Ok(())
    }
}
