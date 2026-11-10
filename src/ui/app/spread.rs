//! The "Spread across commits" suffixes of magit-commit (magit-commit.el):
//! magit-commit-autofixup (git-autofixup) and magit-commit-absorb-modules.

use super::*;

/// magit-list-modified-modules: the modules whose checked out commit
/// differs from the one recorded ("+HASH PATH (DESCRIBE)").
fn modified_modules(status: &[String]) -> Vec<String> {
    status
        .iter()
        .filter_map(|l| {
            let rest = l.strip_prefix('+')?;
            let (hash, rest) = rest.split_once(' ')?;
            let (path, describe) = rest.rsplit_once(" (")?;
            (!hash.is_empty() && !path.is_empty() && describe.len() > 1 && describe.ends_with(')'))
                .then(|| path.to_string())
        })
        .collect()
}

impl App {
    /// magit-git-executable-find: NAME in git's exec path or in PATH.
    fn git_executable_find(&self, name: &str) -> bool {
        let exec = self.git.git_string(&["--exec-path"]).unwrap_or_default();
        let mut dirs: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from(exec.trim())];
        dirs.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
        dirs.iter().any(|d| d.join(name).is_file())
    }

    /// The suffixes of the "Spread across commits" group.
    pub(super) fn spread_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        match cmd {
            // magit-commit-autofixup: a transient with a prefix argument,
            // else run with its value
            "commit-autofixup" | "commit-autofixup-run" => {
                if cmd == "commit-autofixup" && self.prefix_arg.is_some() {
                    self.open_menu("commit-autofixup-menu");
                    return Ok(());
                }
                let args = if cmd == "commit-autofixup" { self.prefix_args("commit-autofixup-menu") } else { args.to_vec() };
                if !self.git_executable_find("git-autofixup") {
                    self.set_msg(
                        "This command requires the git-autofixup script, which is available from https://github.com/torbiak/git-autofixup",
                    );
                    return Ok(());
                }
                if !self.git.anything_modified() {
                    self.set_msg("There are no changes that could be absorbed");
                    return Ok(());
                }
                let commit = self.spread_start(true)?;
                let Some(commit) = commit else { return Ok(()) };
                self.log_select(SelectAction::Autofixup { args }, None, commit);
            }
            "commit-absorb-modules" => {
                if modified_modules(&self.git.git_lines(&["submodule", "status"])).is_empty() {
                    self.set_msg("There are no modified modules that could be absorbed");
                    return Ok(());
                }
                let commit = self.spread_start(false)?;
                let Some(commit) = commit else { return Ok(()) };
                self.log_select(SelectAction::AbsorbModules, None, commit);
            }
            _ => {}
        }
        Ok(())
    }

    /// The commit initially selected: the merge-base with the upstream
    /// (autofixup) or the upstream (absorb-modules), asserted with
    /// magit-rebase-interactive-assert. None if not confirmed, Some(None)
    /// without an upstream.
    fn spread_start(&mut self, merge_base: bool) -> Result<Option<Option<String>>> {
        let upstream = self.git.current_branch().and_then(|b| self.upstream_branch(&b));
        let commit = match upstream {
            Some(u) if merge_base => self.git.git_string(&["merge-base", &u, "HEAD"]).ok().map(|s| s.trim().to_string()),
            u => u,
        }
        .filter(|c| !c.is_empty());
        if let Some(c) = &commit {
            if !self.rebase_assert(c, true, &[])? {
                return Ok(None);
            }
        }
        Ok(Some(commit))
    }

    /// The commit COMMIT picked for magit-commit-autofixup: `git
    /// autofixup ARGS COMMIT`.
    pub(super) fn autofixup(&mut self, commit: &str, args: &[String]) -> Result<()> {
        if !self.rebase_assert(commit, true, &[])? {
            return Ok(());
        }
        let mut a: Vec<&str> = vec!["autofixup"];
        a.extend(args.iter().map(String::as_str));
        a.push(commit);
        let res = self.git.git_interactive(&a);
        self.done("", res.map(|()| String::new()));
        Ok(())
    }

    /// The commit COMMIT picked for magit-commit-absorb-modules: a
    /// "fixup! SUBJECT" commit of each modified module, SUBJECT that of
    /// the last commit after COMMIT that changed it.
    pub(super) fn absorb_modules(&mut self, commit: &str) -> Result<()> {
        if !self.rebase_assert(commit, true, &[])? {
            return Ok(());
        }
        let range = format!("{commit}..");
        for module in modified_modules(&self.git.git_lines(&["submodule", "status"])) {
            let msg = self.git.git_string(&["log", "-1", "--format=%s", &range, "--", &module]).unwrap_or_default();
            let Some(msg) = msg.lines().next().filter(|m| !m.is_empty()) else { continue };
            let msg = format!("fixup! {msg}");
            if let Err(e) = self.git.git_interactive(&["commit", "-m", &msg, "--only", "--", &module]) {
                self.set_msg(&format!("{e:#}"));
            }
        }
        self.refresh();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules() {
        let status = [
            "+0123abc lib/a (v1.0-2-g0123abc)".to_string(),
            " 4567def lib/b (heads/main)".to_string(),
            "-89abcde lib/c".to_string(),
            "+fedcba9 lib/with space (x (y))".to_string(),
        ];
        assert_eq!(modified_modules(&status), ["lib/a", "lib/with space (x"]);
    }
}
