//! The suffixes of magit-worktree (magit-worktree.el).

use super::*;
use crate::ui::options::WorktreeDirectory;

/// An element of magit-list-worktrees.
#[derive(Debug, Clone, Default)]
pub(super) struct Worktree {
    pub path: String,
    /// the branch checked out (without refs/heads/)
    pub branch: Option<String>,
    /// Some reason (maybe empty) if locked
    pub locked: Option<String>,
}

/// abbreviate-file-name: the home directory as "~".
fn abbreviate(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && (path == home || path.starts_with(&format!("{home}/"))) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// expand-file-name of a file name read in the minibuffer.
fn expand(path: &str, dir: &std::path::Path) -> std::path::PathBuf {
    let path = path.trim();
    let expanded = match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{rest}", std::env::var("HOME").unwrap_or_default())
        }
        _ => path.to_string(),
    };
    let p = dir.join(expanded.trim_end_matches('/'));
    // the "." and ".." components resolved
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// The name of the worktree up to its first underscore, and "_".
fn worktree_prefix(toplevel: &std::path::Path) -> String {
    let name = toplevel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    format!("{}_", name.split('_').next().unwrap_or_default())
}

impl App {
    /// magit-list-worktrees: the main worktree first.
    pub(super) fn list_worktrees(&self) -> Vec<Worktree> {
        let mut out: Vec<Worktree> = Vec::new();
        for line in self.git.git_lines(&["worktree", "list", "--porcelain", "-z"]) {
            if let Some(p) = line.strip_prefix("worktree ") {
                // the worktree, not the git directory, of a separate one
                let top = self.git.git_string(&["-C", p, "rev-parse", "--show-toplevel"]).ok();
                let path = top.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| p.to_string());
                out.push(Worktree { path, ..Default::default() });
            } else if let Some(w) = out.last_mut() {
                if let Some(b) = line.strip_prefix("branch ") {
                    w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
                } else if line == "locked" {
                    w.locked = Some(String::new());
                } else if let Some(r) = line.strip_prefix("locked ") {
                    w.locked = Some(r.to_string());
                }
            }
        }
        out
    }

    /// magit--read-worktree-directory with
    /// magit-read-worktree-directory-function: the directory of a new
    /// worktree for REV (absolute).
    fn read_worktree_directory(&mut self, rev: &str) -> Result<Option<std::path::PathBuf>> {
        let top = self.git.toplevel.clone();
        let parent = top.parent().map(std::path::Path::to_path_buf).unwrap_or_else(|| top.clone());
        let name = rev.replace('/', "-");
        let (dir, initial) = match self.options.read_worktree_directory {
            WorktreeDirectory::Plain => (top.clone(), String::new()),
            WorktreeDirectory::Nested => (top.clone(), name),
            WorktreeDirectory::Sibling => (parent, format!("{}{name}", worktree_prefix(&top))),
            WorktreeDirectory::Offsite => {
                let base = self.options.read_worktree_offsite_directory.clone();
                std::fs::create_dir_all(&base)?;
                (base, format!("{}{name}", worktree_prefix(&top)))
            }
        };
        self.read_directory_name(&format!("Checkout {rev} in new worktree: "), &dir, &initial)
    }

    /// read-directory-name in DIR with INITIAL (the minibuffer starts
    /// with DIR abbreviated and INITIAL).
    pub(super) fn read_directory_name(&mut self, prompt: &str, dir: &std::path::Path, initial: &str) -> Result<Option<std::path::PathBuf>> {
        let shown = format!("{}/{initial}", abbreviate(dir.to_string_lossy().trim_end_matches('/')));
        *self.initial_input.borrow_mut() = Some(shown);
        let Some(input) = self.read_line_hist(prompt, None, Hist::Minibuffer, None)? else {
            return Ok(None);
        };
        Ok(Some(expand(&input, dir)))
    }

    /// magit-read-branch-or-commit with the branches EXCLUDE excluded.
    fn read_branch_or_commit_excluding(&mut self, prompt: &str, exclude: &[String]) -> Result<Option<String>> {
        let current = self.git.current_branch().filter(|b| !exclude.contains(b));
        let at_point = self.branch_at_point().filter(|b| !exclude.contains(b));
        let commit = self.commit_at_point().map(|c| self.git.rev_short(&c));
        let mut choices: Vec<String> = self.git.refnames().into_iter().filter(|r| !exclude.contains(r)).collect();
        if let (Some(c), None) = (&commit, &at_point) {
            choices.insert(0, c.clone());
        }
        let default = at_point.or(commit).or(current);
        let table = Completion { candidates: choices, require_match: false, multiple: false };
        Ok(self.completing_read(prompt, default.as_deref(), Hist::Revision, Some(table))?.filter(|s| !s.is_empty()))
    }

    /// magit-diff-visit-directory: the status of the repository of
    /// DIRECTORY (Dired for the current one).
    fn visit_directory(&mut self, directory: &std::path::Path) {
        let top = self.git.git_string(&["-C", &directory.to_string_lossy(), "rev-parse", "--show-toplevel"]).ok();
        let top = top.map(|t| std::path::PathBuf::from(t.trim()));
        let same = top.as_ref().is_some_and(|t| {
            let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            canon(t) == canon(&self.git.toplevel)
        });
        if same {
            self.set_msg(&format!("{} is the current worktree (no Dired)", directory.display()));
            return;
        }
        match Git::discover(directory) {
            Ok(git) => {
                self.switch_repository(git);
                if self.mode == BufferMode::Status {
                    self.refresh();
                } else {
                    self.setup_buffer(BufferMode::Status, Vec::new());
                    self.buried.clear();
                }
            }
            Err(e) => self.set_msg(&format!("{e:#}")),
        }
    }

    /// The suffixes of magit-worktree (and of magit-branch: w and W).
    pub(super) fn worktree_command(&mut self, cmd: &str) -> Result<()> {
        match cmd {
            // magit-worktree-checkout: not a branch checked out elsewhere
            "worktree-checkout" => {
                let checked_out: Vec<String> = self.list_worktrees().into_iter().filter_map(|w| w.branch).collect();
                let Some(commit) = self.read_branch_or_commit_excluding("In new worktree; checkout", &checked_out)? else {
                    return Ok(());
                };
                let Some(dir) = self.read_worktree_directory(&commit)? else { return Ok(()) };
                let d = dir.to_string_lossy().into_owned();
                match self.git.git_interactive(&["worktree", "add", &d, &commit]) {
                    Ok(()) => self.visit_directory(&dir),
                    Err(e) => self.done("", Err(e)),
                }
            }
            // magit-worktree-branch
            "worktree-branch" => {
                let Some((branch, start)) = self.branch_read_args("In new worktree; checkout new branch")? else {
                    return Ok(());
                };
                let Some(dir) = self.read_worktree_directory(&branch)? else { return Ok(()) };
                let d = dir.to_string_lossy().into_owned();
                match self.git.git_interactive(&["worktree", "add", "-b", &branch, &d, &start]) {
                    Ok(()) => self.visit_directory(&dir),
                    Err(e) => self.done("", Err(e)),
                }
            }
            // magit-worktree-move: the main worktree can't be moved; this
            // one moved, its status follows it
            "worktree-move" => {
                let choices: Vec<String> = self.list_worktrees().into_iter().skip(1).map(|w| w.path).collect();
                let Some(worktree) = self.read_choice("Move worktree", &choices, None)? else { return Ok(()) };
                let top = self.git.toplevel.clone();
                let Some(dir) = self.read_directory_name("Move worktree to: ", &top, "")? else { return Ok(()) };
                if std::path::Path::new(&worktree).join(".git").is_dir() {
                    self.set_msg("You may not move the main working tree");
                    return Ok(());
                }
                let preexisting = dir.is_dir();
                let d = dir.to_string_lossy().into_owned();
                match self.git.git_interactive(&["worktree", "move", &worktree, &d]) {
                    Ok(()) if !top.exists() && self.mode == BufferMode::Status => {
                        let base = std::path::Path::new(&worktree).file_name().map(std::path::PathBuf::from);
                        let target = match base {
                            Some(b) if preexisting => dir.join(b),
                            _ => dir,
                        };
                        self.visit_directory(&target);
                    }
                    Ok(()) => self.refresh(),
                    Err(e) => self.done("", Err(e)),
                }
            }
            "worktree-delete" => self.worktree_delete()?,
            // magit-worktree-status: another worktree
            "worktree-status" => {
                let top = self.git.toplevel.to_string_lossy().into_owned();
                let choices: Vec<String> =
                    self.list_worktrees().into_iter().map(|w| w.path).filter(|p| p.trim_end_matches('/') != top).collect();
                let Some(worktree) = self.read_choice("Show status for worktree", &choices, None)? else {
                    return Ok(());
                };
                self.visit_directory(std::path::Path::new(&worktree));
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(worktree_prefix(std::path::Path::new("/a/xagit_feature")), "xagit_");
        assert_eq!(worktree_prefix(std::path::Path::new("/a/xagit")), "xagit_");
        let d = std::path::Path::new("/a/b");
        assert_eq!(expand("/x/y/", d), std::path::PathBuf::from("/x/y"));
        assert_eq!(expand("c", d), std::path::PathBuf::from("/a/b/c"));
        assert_eq!(expand("../c", d), std::path::PathBuf::from("/a/c"));
    }
}
