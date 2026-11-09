//! git/libgit2.rs — in-process repository queries backed by libgit2 (`git2` crate).
//!
//! `exec.rs` / `ops.rs` still shell out to git so every command keeps its
//! Magit-compatible arguments. This module is the libgit2 entry point: it
//! exposes the same read-only information without spawning a git process.

use anyhow::{bail, Context, Result};
use git2::{BranchType, Oid, Repository, RepositoryState, Status, StatusOptions};
use std::path::{Path, PathBuf};

use super::exec::Git;

/// Repository handle backed by libgit2.
pub struct LibGit2 {
    pub repo: Repository,
    pub toplevel: PathBuf,
    pub gitdir: PathBuf,
}

/// One worktree entry as reported by libgit2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub path: String,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub conflicted: bool,
}

impl LibGit2 {
    /// Discover the repository containing `dir` (like magit-toplevel).
    pub fn discover(dir: &Path) -> Result<Self> {
        let repo = Repository::discover(dir)
            .with_context(|| format!("not inside a git repository: {}", dir.display()))?;
        Self::from_repo(repo)
    }

    /// Bridge from the existing `Git` command wrapper.
    pub fn from_git(g: &Git) -> Result<Self> {
        Self::discover(&g.toplevel)
    }

    fn from_repo(repo: Repository) -> Result<Self> {
        let toplevel = match repo.workdir() {
            Some(p) => p.to_path_buf(),
            None => bail!("bare repository has no worktree"),
        };
        let gitdir = repo.path().to_path_buf();
        Ok(LibGit2 {
            repo,
            toplevel,
            gitdir,
        })
    }

    /// HEAD commit oid, None before the first commit.
    pub fn head_oid(&self) -> Option<Oid> {
        self.repo.head().ok()?.target()
    }

    pub fn no_commit(&self) -> bool {
        self.head_oid().is_none()
    }

    /// Short branch name, None when detached or unborn.
    pub fn current_branch(&self) -> Option<String> {
        let head = self.repo.head().ok()?;
        if !head.is_branch() {
            return None;
        }
        head.shorthand().ok().map(str::to_string)
    }

    /// libgit2's view of an in-progress operation (merge/rebase/cherry-pick...).
    pub fn state(&self) -> RepositoryState {
        self.repo.state()
    }

    pub fn branches(&self) -> Result<Vec<String>> {
        let mut v: Vec<String> = self
            .repo
            .branches(Some(BranchType::Local))?
            .filter_map(|b| b.ok())
            .filter_map(|(b, _)| b.name().ok().flatten().map(str::to_string))
            .collect();
        v.sort();
        Ok(v)
    }

    pub fn tags(&self) -> Result<Vec<String>> {
        let names = self.repo.tag_names(None)?;
        let mut v: Vec<String> = names
            .iter()
            .filter_map(|n| n.ok().flatten())
            .map(str::to_string)
            .collect();
        v.sort();
        Ok(v)
    }

    pub fn remotes(&self) -> Result<Vec<String>> {
        let names = self.repo.remotes()?;
        let mut v: Vec<String> = names
            .iter()
            .filter_map(|n| n.ok().flatten())
            .map(str::to_string)
            .collect();
        v.sort();
        Ok(v)
    }

    /// Status entries for the whole worktree, without spawning `git status`.
    pub fn statuses(&self) -> Result<Vec<FileStatus>> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true).recurse_untracked_dirs(true);
        let list = self.repo.statuses(Some(&mut opts))?;
        Ok(list
            .iter()
            .filter_map(|e| {
                let s = e.status();
                let staged = s.intersects(
                    Status::INDEX_NEW
                        | Status::INDEX_MODIFIED
                        | Status::INDEX_DELETED
                        | Status::INDEX_RENAMED
                        | Status::INDEX_TYPECHANGE,
                );
                let unstaged = s.intersects(
                    Status::WT_MODIFIED
                        | Status::WT_DELETED
                        | Status::WT_RENAMED
                        | Status::WT_TYPECHANGE,
                );
                e.path().ok().map(|p| FileStatus {
                    path: p.to_string(),
                    staged,
                    unstaged,
                    untracked: s.contains(Status::WT_NEW),
                    conflicted: s.contains(Status::CONFLICTED),
                })
            })
            .collect())
    }
}
