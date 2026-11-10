//! The suffixes of magit-gitignore (magit-gitignore.el): adding ignore
//! rules to the .gitignore files, $GIT_DIR/info/exclude and
//! core.excludesFile, and the skip-worktree and assume-unchanged bits.

use super::*;

/// file-name-extension: None for a name without a dot (or only a
/// leading one).
fn file_name_extension(file: &str) -> Option<&str> {
    let name = file.rsplit('/').next().unwrap_or(file);
    let stem = name.strip_prefix('.').unwrap_or(name);
    stem.rfind('.').map(|i| &stem[i + 1..])
}

/// file-name-directory: the directory part with its slash.
fn file_name_directory(file: &str) -> Option<&str> {
    file.rfind('/').map(|i| &file[..=i])
}

/// The rule as written by magit--gitignore: each run of backslashes
/// doubled.
fn quote_backslashes(rule: &str) -> String {
    rule.replace('\\', "\\\\")
}

impl App {
    /// The suffixes of magit-gitignore.
    pub(super) fn gitignore_command(&mut self, cmd: &str) -> Result<()> {
        match cmd {
            // magit-gitignore-in-topdir
            "ignore" => {
                let Some(rule) = self.gitignore_read_pattern(None)? else { return Ok(()) };
                let file = self.git.toplevel.join(".gitignore");
                self.gitignore(&rule, &file, true)?;
            }
            // magit-gitignore-in-subdir
            "ignore-subdir" => {
                let top = self.git.toplevel.clone();
                let start = match self.current_file_name().as_deref().and_then(file_name_directory) {
                    Some(d) => top.join(d),
                    None => top.clone(),
                };
                *self.initial_input.borrow_mut() = Some(format!("{}/", start.display().to_string().trim_end_matches('/')));
                let Some(dir) = self.read_line_hist("Limit rule to files in: ", None, Hist::Minibuffer, None)? else {
                    return Ok(());
                };
                let dir = match dir.strip_prefix('~') {
                    Some(rest) => format!("{}{rest}", std::env::var("HOME").unwrap_or_default()),
                    None => dir,
                };
                let dir = top.join(dir.trim());
                let rel = dir.strip_prefix(&top).map(|r| r.to_string_lossy().into_owned()).unwrap_or_default();
                let rel = if rel.is_empty() { None } else { Some(format!("{}/", rel.trim_end_matches('/'))) };
                let Some(rule) = self.gitignore_read_pattern(rel.as_deref())? else { return Ok(()) };
                self.gitignore(&rule, &dir.join(".gitignore"), true)?;
            }
            // magit-gitignore-in-gitdir: the common git directory
            "ignore-gitdir" => {
                let Some(rule) = self.gitignore_read_pattern(None)? else { return Ok(()) };
                let common = self.git.git_string(&["rev-parse", "--git-common-dir"]).unwrap_or_default();
                let common = self.git.toplevel.join(common.trim());
                self.gitignore(&rule, &common.join("info").join("exclude"), false)?;
            }
            // magit-gitignore-on-system (inapt without core.excludesFile)
            "ignore-system" => {
                let Some(file) = self.config("core.excludesFile") else {
                    self.set_msg("Variable `core.excludesFile' isn't set");
                    return Ok(());
                };
                let Some(rule) = self.gitignore_read_pattern(None)? else { return Ok(()) };
                let file = match file.strip_prefix("~/") {
                    Some(rest) => std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
                    None => self.git.toplevel.join(file),
                };
                self.gitignore(&rule, &file, false)?;
            }
            "skip-worktree" | "no-skip-worktree" | "assume-unchanged" | "no-assume-unchanged" => {
                let (flag, letters, all): (&str, &[char], bool) = match cmd {
                    "skip-worktree" => ("--skip-worktree", &['S'], true),
                    "no-skip-worktree" => ("--no-skip-worktree", &['S'], false),
                    "assume-unchanged" => ("--assume-unchanged", &['h', 's', 'm', 'r', 'c', 'k'], true),
                    _ => ("--no-assume-unchanged", &['h', 's', 'm', 'r', 'c', 'k'], false),
                };
                let listing = if letters == ['S'] { "-t" } else { "-v" };
                // magit-skip-worktree-files / magit-assume-unchanged-files
                let marked: Vec<String> = self
                    .git
                    .git_lines(&["ls-files", "-z", listing])
                    .into_iter()
                    .filter(|l| l.chars().next().is_some_and(|c| letters.contains(&c)))
                    .filter_map(|l| l.get(2..).map(str::to_string))
                    .collect();
                let files = if all {
                    self.git.git_lines(&["ls-files", "-z"]).into_iter().filter(|f| !marked.contains(f)).collect()
                } else {
                    marked
                };
                let prompt = match cmd {
                    "skip-worktree" => "Skip worktree for",
                    "no-skip-worktree" => "Do not skip worktree for",
                    "assume-unchanged" => "Assume file to be unchanged",
                    _ => "Do not assume file to be unchanged",
                };
                let Some(file) = self.read_file_choice(prompt, &files)? else { return Ok(()) };
                let res = self.git.git_interactive(&["update-index", flag, "--", &file]);
                self.done("", res.map(|()| String::new()));
            }
            _ => {}
        }
        Ok(())
    }

    /// magit-read-file-choice: the only file without asking.
    fn read_file_choice(&mut self, prompt: &str, files: &[String]) -> Result<Option<String>> {
        match files {
            [] => {
                self.set_msg("No file choices");
                Ok(None)
            }
            [f] => Ok(Some(f.clone())),
            _ => {
                let default = self.current_file_name().filter(|f| files.contains(f));
                self.read_choice(prompt, files, default.as_deref())
            }
        }
    }

    /// magit--gitignore-patterns: the untracked files and directories
    /// of DIRECTORY (relative to the toplevel, with a slash) with a
    /// leading slash, "*.EXT" and "DIR/*.EXT" of their extensions.
    fn gitignore_patterns(&self, directory: Option<&str>) -> Vec<String> {
        let mut a = vec!["ls-files", "-z", "--others", "--exclude-standard", "--"];
        a.extend(directory);
        let mut files = self.git.git_lines(&a);
        let mut a = vec!["status", "-z", "--porcelain", "--untracked-files=normal", "--"];
        a.extend(directory);
        let mut dirs: Vec<String> = self
            .git
            .git_lines(&a)
            .into_iter()
            .filter(|l| (l.starts_with("??") || l.starts_with("!!")) && l.ends_with('/'))
            .filter_map(|l| l.get(3..).map(str::to_string))
            .collect();
        if let Some(d) = directory {
            let strip = |v: &mut Vec<String>| {
                for f in v.iter_mut() {
                    *f = f.get(d.len()..).unwrap_or_default().to_string();
                }
            };
            strip(&mut files);
            strip(&mut dirs);
        }
        let mut globs: Vec<String> = Vec::new();
        let mut dirglobs: Vec<String> = Vec::new();
        for file in &files {
            if let Some(ext) = file_name_extension(file) {
                let glob = format!("*.{ext}");
                if !globs.contains(&glob) {
                    globs.push(glob);
                }
                if let Some(dir) = file_name_directory(file) {
                    let glob = format!("{dir}*.{ext}");
                    if !dirglobs.contains(&glob) {
                        dirglobs.push(glob);
                    }
                }
            }
        }
        let mut all = globs;
        all.extend(files.into_iter().chain(dirs).chain(dirglobs).map(|f| format!("/{f}")));
        all.sort();
        all
    }

    /// magit-gitignore-read-pattern: the default "/FILE" or "*.EXT" of
    /// the current file, when a choice.
    fn gitignore_read_pattern(&mut self, directory: Option<&str>) -> Result<Option<String>> {
        let choices = self.gitignore_patterns(directory);
        let default = self.current_file_name().and_then(|f| {
            let f = match directory {
                Some(d) => f.get(d.len()..).unwrap_or_default().to_string(),
                None => f,
            };
            let slash = format!("/{f}");
            if choices.contains(&slash) {
                return Some(slash);
            }
            let glob = format!("*.{}", file_name_extension(&slash).unwrap_or_default());
            choices.contains(&glob).then_some(glob)
        });
        let table = Completion { candidates: choices, require_match: false, multiple: false };
        self.read_any("File or pattern to ignore", default.as_deref(), Hist::Minibuffer, table)
    }

    /// magit--gitignore: RULE appended to FILE, which is staged if STAGE.
    fn gitignore(&mut self, rule: &str, file: &std::path::Path, stage: bool) -> Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut content = std::fs::read_to_string(file).unwrap_or_default();
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&quote_backslashes(rule));
        content.push('\n');
        std::fs::write(file, content)?;
        if stage {
            let rel = file.strip_prefix(&self.git.toplevel).unwrap_or(file).to_string_lossy().into_owned();
            let res = self.git.git_interactive(&["add", &rel]);
            self.done("", res.map(|()| String::new()));
        } else {
            self.refresh();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(file_name_extension("a/b.txt"), Some("txt"));
        assert_eq!(file_name_extension(".bashrc"), None);
        assert_eq!(file_name_extension("a/.x.y"), Some("y"));
        assert_eq!(file_name_extension("foo."), Some(""));
        assert_eq!(file_name_extension("Makefile"), None);
        assert_eq!(file_name_directory("a/b/c"), Some("a/b/"));
        assert_eq!(file_name_directory("c"), None);
        assert_eq!(quote_backslashes(r"a\b\\c"), r"a\\b\\\\c");
    }
}
