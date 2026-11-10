//! The suffixes of magit-patch, magit-patch-create, magit-patch-apply
//! (magit-patch.el) and magit-am (magit-sequence.el).

use super::worktree::{abbreviate, expand};
use super::*;

/// RANGE of magit-patch-create: a single commit C is "C^..C".
fn patch_range(range: &str) -> String {
    if range.contains("..") {
        range.to_string()
    } else {
        format!("{range}^..{range}")
    }
}

/// The file of the cover letter: "[v<N>-]0000-cover-letter.patch" in the
/// --output-directory= (relative to TOPDIR), else in TOPDIR.
fn cover_letter(args: &[String], topdir: &std::path::Path) -> std::path::PathBuf {
    let value = |a: &str| args.iter().find_map(|x| x.strip_prefix(a)).filter(|v| !v.is_empty());
    let name = match value("--reroll-count=") {
        Some(n) => format!("v{n}-0000-cover-letter.patch"),
        None => "0000-cover-letter.patch".to_string(),
    };
    match value("--output-directory=") {
        Some(dir) => expand(dir, topdir).join(name),
        None => topdir.join(name),
    }
}

/// The arguments of ARGS other than "-- FILES", and the FILES.
fn split_files(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut rest = Vec::new();
    let mut files = Vec::new();
    for a in args {
        match a.strip_prefix("-- ") {
            Some(f) => files.extend(f.split(' ').filter(|f| !f.is_empty()).map(str::to_string)),
            None => rest.push(a.clone()),
        }
    }
    (rest, files)
}

impl App {
    /// read-file-name in default-directory: the minibuffer starts with
    /// the directory and INITIAL; DEFAULT when that is left unchanged.
    fn read_file_name_in(&mut self, prompt: &str, default: Option<&str>, initial: Option<&str>) -> Result<Option<std::path::PathBuf>> {
        let dir = self.cwd.clone();
        let shown = format!("{}/", abbreviate(dir.to_string_lossy().trim_end_matches('/')));
        *self.initial_input.borrow_mut() = Some(format!("{shown}{}", initial.unwrap_or("")));
        let Some(input) = self.read_line_hist(prompt, None, Hist::Minibuffer, None)? else {
            return Ok(None);
        };
        let input = input.trim();
        Ok(Some(match default {
            Some(d) if input.is_empty() || input == shown => expand(d, &dir),
            _ => expand(input, &dir),
        }))
    }

    /// magit-read-branch-or-commit with SECONDARY-DEFAULT (before the
    /// current branch).
    fn read_branch_or_commit_default(&self, prompt: &str, secondary: Option<&str>) -> Result<Option<String>> {
        let default = self
            .branch_at_point()
            .or_else(|| self.commit_at_point().map(|c| self.git.rev_short(&c)))
            .or_else(|| secondary.map(str::to_string))
            .or_else(|| self.git.current_branch());
        self.read_rev(prompt, default.as_deref())
    }

    /// magit-read-range-or-commit: "PROMPT: ", the default the range of
    /// the selected commits and branches, else the branch or commit at
    /// point, else the current branch.
    fn read_range_or_commit(&mut self, prompt: &str) -> Result<Option<String>> {
        let default = self
            .region_revs()
            .map(|r| format!("{}..{}", r[r.len() - 1], r[0]))
            .or_else(|| self.branch_or_commit_at_point())
            .or_else(|| self.git.current_branch());
        let table = Completion { candidates: self.git.refnames(), ..Default::default() };
        self.read_any(&format!("{prompt}: "), default.as_deref(), Hist::Revision, table)
    }

    /// The readers of the magit-patch-create infixes.
    pub(super) fn read_patch_infix(&mut self, reader: &Reader, arg: &str) -> Result<Option<String>> {
        match reader {
            // magit-transient-read-person: magit-list-people
            Reader::Person => {
                let candidates = self
                    .git
                    .git_lines(&["shortlog", "-n", "-s", "-e", "HEAD"])
                    .iter()
                    .filter_map(|l| l.trim_start().split_once('\t').map(|(_, p)| p.to_string()))
                    .collect();
                let table = Completion { candidates, ..Default::default() };
                let r = self.completing_read(arg, None, Hist::Infix(arg), Some(table))?;
                Ok(r.filter(|v| !v.is_empty()))
            }
            // magit-format-patch-select-base
            Reader::PatchBase => {
                let mut candidates = vec!["auto".to_string()];
                candidates.extend(self.git.refnames());
                let table = Completion { candidates, ..Default::default() };
                self.read_any(arg, Some("auto"), Hist::Infix(arg), table)
            }
            // magit-format-patch-select-range-diff
            Reader::RangeOrCommit => self.read_range_or_commit(arg),
            // transient-read-existing-directory
            _ => {
                let dir = self.cwd.clone();
                *self.initial_input.borrow_mut() = Some(format!("{}/", abbreviate(dir.to_string_lossy().trim_end_matches('/'))));
                let Some(input) = self.read_line_hist(arg, None, Hist::Infix(arg), None)? else {
                    return Ok(None);
                };
                let path = expand(&input, &dir);
                if !path.is_dir() {
                    self.set_msg("[No match]");
                    return Ok(None);
                }
                Ok(Some(path.to_string_lossy().into_owned()))
            }
        }
    }

    /// find-file of FILE: the user's editor (`git var GIT_EDITOR`).
    fn edit_file(&mut self, file: &std::path::Path) {
        // not git_string, which overrides core.editor
        let editor = std::process::Command::new("git")
            .args(["var", "GIT_EDITOR"])
            .current_dir(&self.git.toplevel)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| "vi".into());
        let file = file.to_path_buf();
        let res = self.with_terminal(|g| {
            std::process::Command::new("sh")
                .args(["-c", &format!("{editor} \"$@\""), &editor])
                .arg(&file)
                .current_dir(&g.toplevel)
                .status()
        });
        match res {
            Ok(st) if !st.success() => self.set_msg(&format!("{editor} failed ({st})")),
            Err(e) => self.set_msg(&format!("Cannot run {editor}: {e}")),
            _ => {}
        }
    }

    /// magit-am-in-progress-p
    fn am_in_progress(&self) -> bool {
        self.git.gitdir.join("rebase-apply/applying").exists()
    }

    /// magit-run-git-sequencer "am" ARGS.
    fn run_am(&mut self, args: &[String]) {
        let mut a: Vec<String> = vec!["am".into()];
        a.extend(args.iter().cloned());
        let res = self.git.git_fmt(&a);
        self.done("", res);
    }

    /// The suffixes of magit-patch, magit-patch-create, magit-patch-apply
    /// and magit-am.
    pub(super) fn patch_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        match cmd {
            "patch-create" => {
                let range = match self.region_sections(false).and_then(|secs| {
                    secs.iter()
                        .map(|&s| match &self.buf.all[self.sec(s).start].kind {
                            LineKind::Commit { hash, .. } => Some(hash.clone()),
                            _ => None,
                        })
                        .collect::<Option<Vec<String>>>()
                }) {
                    Some(revs) if revs.len() > 1 => format!("{}^..{}", revs[revs.len() - 1], revs[0]),
                    Some(revs) if revs.len() == 1 => patch_range(&revs[0]),
                    _ => match self.read_range_or_commit("Create patches for range or commit")? {
                        Some(r) => patch_range(&r),
                        None => return Ok(()),
                    },
                };
                let (rest, files) = split_files(args);
                let mut a: Vec<String> = vec!["format-patch".into(), range];
                a.extend(rest);
                a.push("--".into());
                a.extend(files);
                let res = self.git.git_fmt(&a);
                let ok = res.is_ok();
                self.done("", res);
                if ok && args.iter().any(|a| a == "--cover-letter") {
                    let file = cover_letter(args, &self.git.toplevel.clone());
                    self.edit_file(&file);
                }
            }
            "patch-apply" => {
                let initial = self.current_file().map(|(_, p)| p);
                let Some(file) = self.read_file_name_in("Apply patch: ", None, initial.as_deref())? else {
                    return Ok(());
                };
                let mut a: Vec<String> = vec!["apply".into()];
                a.extend(args.iter().cloned());
                a.push("--".into());
                a.push(file.to_string_lossy().into_owned());
                let res = self.git.git_fmt(&a);
                self.done("", res);
            }
            "patches-apply" => {
                let files = match self.region_files() {
                    Some(f) => f.iter().map(|f| self.git.toplevel.join(f)).collect(),
                    None => {
                        let default = self.current_file().map(|(_, p)| p);
                        let prompt = match &default {
                            Some(d) => format!("Apply patch ({d}): "),
                            None => "Apply patch: ".to_string(),
                        };
                        match self.read_file_name_in(&prompt, default.as_deref(), None)? {
                            Some(f) => vec![f],
                            None => return Ok(()),
                        }
                    }
                };
                let mut a = args.to_vec();
                a.push("--".into());
                a.extend(files.iter().map(|f| f.to_string_lossy().into_owned()));
                self.run_am(&a);
            }
            "patches-apply-maildir" => {
                let Some(dir) = self.read_file_name_in("Apply mbox or Maildir: ", None, None)? else {
                    return Ok(());
                };
                let mut a = args.to_vec();
                a.push(dir.to_string_lossy().into_owned());
                self.run_am(&a);
            }
            "patches-continue" | "patches-skip" | "patches-abort" => {
                if !self.am_in_progress() {
                    self.set_msg("Not applying any patches");
                    return Ok(());
                }
                if cmd == "patches-continue" && !self.git.git_ok(&["diff", "--quiet", "--ignore-submodules"]) {
                    self.set_msg("Cannot continue due to unstaged changes");
                    return Ok(());
                }
                let arg = format!("--{}", cmd.trim_start_matches("patches-"));
                self.run_am(&[arg]);
            }
            "patch-save" => self.patch_save()?,
            _ => self.request_pull()?,
        }
        Ok(())
    }

    /// magit-patch-save with magit-patch-save-arguments '(exclude
    /// "--stat"): the arguments of the buffer except --stat (all of them
    /// with a prefix argument).
    fn patch_save(&mut self) -> Result<()> {
        let (range, typearg, files) = match &self.mode {
            BufferMode::Diff(s) => (s.range.clone(), s.typearg.clone(), s.files.clone()),
            BufferMode::Revision { rev, files } => (Some(format!("{rev}^..{rev}")), None, files.clone()),
            BufferMode::Stash { stash } => (Some(format!("{stash}^..{stash}")), None, Vec::new()),
            _ => {
                self.set_msg("Only diff buffers can be saved as patches");
                return Ok(());
            }
        };
        let Some(file) = self.read_file_name_in("Write patch file: ", None, None)? else {
            return Ok(());
        };
        let all = self.prefix_arg.is_some();
        let mut a: Vec<String> = vec!["diff".into()];
        a.extend(range);
        a.push("-p".into());
        a.extend(typearg);
        a.extend(self.buf_args.iter().filter(|x| all || *x != "--stat").cloned());
        a.push("--".into());
        a.extend(files);
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        let out = self.git.git_output(&a)?;
        std::fs::write(&file, out.stdout)?;
        self.refresh();
        Ok(())
    }

    /// magit-request-pull: the url of a remote, the start (the upstream
    /// the secondary default) and the end; the message shown.
    fn request_pull(&mut self) -> Result<()> {
        let Some(remote) = self.read_remote("Remote")? else {
            return Ok(());
        };
        let url = self.config(&format!("remote.{remote}.url")).unwrap_or_default();
        let upstream = self.git.upstream_shortname();
        let Some(start) = self.read_branch_or_commit_default("Start", upstream.as_deref())? else {
            return Ok(());
        };
        let Some(end) = self.read_branch_or_commit_default("End", None)? else {
            return Ok(());
        };
        let res = self.git.git_string(&["request-pull", &start, &url, &end]);
        match res {
            Ok(text) => self.view_text = Some(("request-pull".into(), text)),
            Err(e) => self.set_msg(&format!("{e:#}")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(patch_range("abc"), "abc^..abc");
        assert_eq!(patch_range("a..b"), "a..b");
    }

    #[test]
    fn cover_letters() {
        let top = std::path::Path::new("/r");
        assert_eq!(cover_letter(&[], top), top.join("0000-cover-letter.patch"));
        let args = ["--reroll-count=2".to_string(), "--output-directory=out".to_string()];
        assert_eq!(cover_letter(&args, top), top.join("out/v2-0000-cover-letter.patch"));
    }

    #[test]
    fn files() {
        let args = ["-U3".to_string(), "-- a b".to_string()];
        assert_eq!(split_files(&args), (vec!["-U3".to_string()], vec!["a".to_string(), "b".to_string()]));
    }
}
