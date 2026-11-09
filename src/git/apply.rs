//! git/apply.rs — applying file / hunk / region patches like magit-apply.el.

use super::diff::{self, FileDiff, Scope};
use super::exec::Git;
use anyhow::{bail, Result};

/// Number of context lines from diff arguments (`-U<n>`), default 3
/// (magit-diff-get-context).
pub fn diff_context(args: &[String]) -> usize {
    args.iter()
        .filter_map(|a| a.strip_prefix("-U").or_else(|| a.strip_prefix("--unified=")))
        .filter_map(|n| n.parse().ok())
        .last()
        .unwrap_or(3)
}

impl Git {
    /// `git diff [--cached] <args> --no-prefix` split into file sections, as
    /// inserted by magit-insert-unstaged-changes / magit-insert-staged-changes.
    pub fn diff_sections(&self, cached: bool, args: &[String]) -> Result<Vec<FileDiff>> {
        let mut a: Vec<&str> = vec!["diff"];
        if cached {
            a.push("--cached");
        }
        a.extend(args.iter().map(String::as_str));
        a.extend(["--no-prefix", "--no-ext-diff", "--"]);
        Ok(diff::parse(&self.git_string(&a)?))
    }

    /// Sections of an arbitrary diff command (show, stash show -p, diff A B);
    /// `--no-prefix` is added after the subcommand.
    pub fn diff_sections_of(&self, args: &[&str]) -> Result<Vec<FileDiff>> {
        Ok(diff::parse(&self.diff_output(args)?))
    }

    /// Output of a diff command (`ARGS[0]` is the subcommand) with
    /// `--no-prefix` added after the subcommand.
    pub fn diff_output(&self, args: &[&str]) -> Result<String> {
        let mut a: Vec<&str> = args.iter().take(1).copied().collect();
        a.push("--no-prefix");
        a.extend(args.iter().skip(1));
        self.git_string(&a)
    }

    /// magit-apply-patch: `git apply <args> -p0 -C<context> --ignore-space-change -`
    pub fn apply_patch(&self, patch: &str, args: &[&str], context: usize) -> Result<()> {
        let c = format!("-C{context}");
        let mut a: Vec<&str> = vec!["apply"];
        a.extend(args);
        a.extend(["-p0", &c, "--ignore-space-change", "-"]);
        self.git_with_input(&a, patch.as_bytes()).map(|_| ())
    }

    /// magit-apply-hunk / -hunks / -region / -diff with ARGS.
    pub fn apply_scope(&self, file: &FileDiff, scope: &Scope, args: &[&str], context: usize) -> Result<()> {
        if file.combined && *scope != Scope::File {
            bail!("Cannot un-/stage resolution hunks.  Stage the whole file");
        }
        let reverse = args.contains(&"--reverse");
        let Some(patch) = diff::scope_patch(file, scope, reverse) else {
            bail!("no such hunk");
        };
        self.apply_patch(&patch, args, context)
    }

    /// magit-stage on an unstaged hunk / region: `apply --cached`
    pub fn stage_change(&self, file: &FileDiff, scope: &Scope, context: usize) -> Result<()> {
        self.apply_scope(file, scope, &["--cached"], context)
    }

    /// magit-unstage on a staged hunk / region: `apply --reverse --cached`
    pub fn unstage_change(&self, file: &FileDiff, scope: &Scope, context: usize) -> Result<()> {
        self.apply_scope(file, scope, &["--reverse", "--cached"], context)
    }

    /// magit-discard-apply: unstaged changes are reversed in the worktree;
    /// staged ones in the index and the worktree (`--index`), or first in
    /// the index and then with `--reject` in the worktree when the file
    /// also has unstaged changes.
    pub fn discard_change(&self, file: &FileDiff, scope: &Scope, staged: bool, context: usize) -> Result<()> {
        if !staged {
            return self.apply_scope(file, scope, &["--reverse"], context);
        }
        if self.file_has_unstaged(&file.path) {
            self.apply_scope(file, scope, &["--reverse", "--cached"], context)?;
            self.apply_scope(file, scope, &["--reverse", "--reject"], context)
        } else {
            self.apply_scope(file, scope, &["--reverse", "--index"], context)
        }
    }

    /// magit-reverse-apply: `apply --reverse --reject` on the worktree.
    pub fn reverse_change(&self, file: &FileDiff, scope: &Scope, context: usize) -> Result<()> {
        self.apply_scope(file, scope, &["--reverse", "--reject"], context)
    }

    /// magit-apply on a committed / stashed change: plain `apply`.
    pub fn apply_change(&self, file: &FileDiff, scope: &Scope, context: usize) -> Result<()> {
        self.apply_scope(file, scope, &[], context)
    }

    /// magit-apply-diffs: one patch made of several whole files.
    pub fn apply_files(&self, files: &[&FileDiff], args: &[&str], context: usize) -> Result<()> {
        let patch: String = files.iter().map(|f| f.text()).collect();
        if patch.is_empty() {
            return Ok(());
        }
        self.apply_patch(&patch, args, context)
    }

    /// magit-anything-unstaged-p for one file
    pub fn file_has_unstaged(&self, path: &str) -> bool {
        !self.git_ok(&["diff", "--quiet", "--", path])
    }
}
