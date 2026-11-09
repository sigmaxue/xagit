//! git/exec.rs — running git commands (mirrors magit-git.el behavior)

use anyhow::{anyhow, bail, Context, Result};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// Global arguments Magit passes to every git invocation (magit-git-global-arguments).
const GLOBAL_ARGS: [&str; 4] = ["-c", "core.editor=true", "-c", "color.ui=false"];

/// Recent side-effecting git invocations, shown by `$` (magit-process-buffer).
/// Like Magit, plain queries (status, rev-parse, ...) are not recorded.
static PROCESS_LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const PROCESS_LOG_MAX: usize = 200;

pub(crate) fn record(dir: &Path, args: &[&str], ok: bool, code: Option<i32>, stderr: &str) {
    let mut entry = format!(
        "{} git {}  [{}]",
        if ok { " " } else { "!" },
        args.join(" "),
        match code {
            Some(c) => format!("exit {c}"),
            None => "signal".into(),
        }
    );
    entry.push_str(&format!("\n    in {}", dir.display()));
    for l in stderr.lines().filter(|l| !l.trim().is_empty()).take(8) {
        entry.push_str(&format!("\n    {l}"));
    }
    if let Ok(mut log) = PROCESS_LOG.lock() {
        if log.len() >= PROCESS_LOG_MAX {
            log.pop_front();
        }
        log.push_back(entry);
    }
}

/// Snapshot of the process log, oldest first.
pub fn process_log() -> Vec<String> {
    PROCESS_LOG
        .lock()
        .map(|l| l.iter().cloned().collect())
        .unwrap_or_default()
}

/// Which program git may start to edit a todo list or a commit message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editor<'a> {
    /// accept the file unchanged (`true`)
    Accept,
    /// the user's configured editor; the caller must hand over the terminal
    User,
    /// a non-interactive shell command, e.g. Magit's perl todo editors
    Command(&'a str),
}

#[derive(Debug, Clone)]
pub struct Git {
    /// Repository top-level worktree path (magit-toplevel)
    pub toplevel: PathBuf,
    /// .git dir path
    pub gitdir: PathBuf,
}

fn env_git() -> &'static str {
    "git"
}

impl Git {
    /// Find the toplevel of the repository containing `dir` (like magit-toplevel).
    pub fn discover(dir: &Path) -> Result<Git> {
        let out = Command::new(env_git())
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(dir)
            .output()
            .context("failed to run git")?;
        if !out.status.success() {
            bail!("not inside a git repository: {}", dir.display());
        }
        let top = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if top.is_empty() {
            bail!("bare repository or no toplevel");
        }
        let toplevel = PathBuf::from(top);
        let gitdir = Self::gitdir_of(&toplevel)?;
        Ok(Git { toplevel, gitdir })
    }

    fn gitdir_of(toplevel: &Path) -> Result<PathBuf> {
        let dotgit = toplevel.join(".git");
        if dotgit.is_dir() {
            return Ok(dotgit);
        }
        if dotgit.is_file() {
            // gitfile: "gitdir: /path/to/real/gitdir"
            let txt = std::fs::read_to_string(&dotgit)?;
            if let Some(p) = txt.strip_prefix("gitdir:") {
                return Ok(PathBuf::from(p.trim()));
            }
        }
        bail!("cannot locate .git directory");
    }

    /// True when the repo has no commits yet (magit-no-commit-p).
    pub fn no_commit(&self) -> bool {
        self.rev_parse("HEAD").is_err()
    }

    /// `git rev-parse --verify <rev>` -> hash
    pub fn rev_parse(&self, rev: &str) -> Result<String> {
        self.git_string(&["rev-parse", "--verify", rev])
            .map(|s| s.trim().to_string())
            .and_then(|s| if s.is_empty() { bail!("no such ref: {rev}") } else { Ok(s) })
    }

    /// Short hash of rev (magit-rev-parse --short).
    pub fn rev_short(&self, rev: &str) -> String {
        self.git_string(&["rev-parse", "--short", rev])
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    pub fn current_branch(&self) -> Option<String> {
        let s = self
            .git_string(&["symbolic-ref", "--short", "HEAD"])
            .ok()?;
        let s = s.trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }

    /// HEAD line for the status header: "branch-name" or "detached at abc1234"
    #[allow(dead_code)]
    pub fn head_desc(&self) -> String {
        match self.current_branch() {
            Some(b) => b,
            None => format!(
                "detached at {}",
                self.rev_short("HEAD")
            ),
        }
    }

    pub fn upstream_shortname(&self) -> Option<String> {
        let b = self.current_branch()?;
        let s = self
            .git_string(&["rev-parse", "--abbrev-ref", &format!("{b}@{{u}}")])
            .ok()?
            .trim()
            .to_string();
        if s.is_empty() || s.ends_with("@{u}") { None } else { Some(s) }
    }

    /// "N ahead, M behind" counts vs upstream.
    pub fn ahead_behind(&self) -> Option<(u32, u32)> {
        let up = self.current_branch()?;
        let out = self
            .git_string(&[
                "rev-list",
                "--left-right",
                "--count",
                &format!("HEAD...{up}@{{u}}"),
            ])
            .ok()?;
        let mut it = out.split_whitespace();
        let a: u32 = it.next()?.parse().ok()?;
        let b: u32 = it.next()?.parse().ok()?;
        Some((a, b))
    }

    pub fn remotes(&self) -> Vec<String> {
        self.git_lines(&["remote"])
    }

    pub fn branches(&self) -> Vec<String> {
        let mut v = self.git_lines(&["for-each-ref", "--format=%(refname:short)", "refs/heads/"]);
        v.sort();
        v
    }

    pub fn tags(&self) -> Vec<String> {
        let mut v = self.git_lines(&["for-each-ref", "--format=%(refname:short)", "refs/tags/"]);
        v.sort();
        v
    }

    pub fn stashes(&self) -> Vec<(String, String)> {
        // (ref name "stash@{0}", message) — use '|' separator (git < 2.44 lacks %X)
        self.git_lines(&[
                "stash",
                "list",
                "--format=%gd|%gs",
            ])
            .into_iter()
            .filter_map(|l| {
                let (a, b) = l.split_once('|')?;
                Some((a.trim().to_string(), b.trim().to_string()))
            })
            .collect()
    }

    pub fn staged_files(&self) -> Vec<String> {
        self.git_lines(&["diff", "--cached", "--name-only", "-z"])
    }

    pub fn unstaged_files(&self) -> Vec<String> {
        self.git_lines(&["diff", "--name-only", "-z"])
    }

    pub fn untracked_files(&self) -> Vec<String> {
        self.git_lines(&["ls-files", "--others", "--exclude-standard", "-z"])
    }

    /// Run git and return stdout as String. Err on non-zero exit.
    pub fn git_string(&self, args: &[&str]) -> Result<String> {
        let out = self.git_output(args)?;
        if !out.status.success() {
            bail!(
                "git {} failed: {}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    /// Run git quietly and report only whether it exited successfully.
    pub fn git_ok(&self, args: &[&str]) -> bool {
        self.git_output(args).map(|o| o.status.success()).unwrap_or(false)
    }

    /// Run git and split stdout on NUL (for -z listings); skips empty entries.
    /// Falls back to newline splitting when the output contains no NUL bytes.
    pub fn git_lines(&self, args: &[&str]) -> Vec<String> {
        let out = Command::new(env_git())
            .args(GLOBAL_ARGS)
            .args(args)
            .current_dir(&self.toplevel)
            .output();
        match out {
            Ok(o) if o.status.success() => {
                let s = String::from_utf8_lossy(&o.stdout);
                let mut parts: Vec<&str> = if s.contains('\0') {
                    s.split('\0').collect()
                } else {
                    s.split('\n').collect()
                };
                // drop trailing empty piece after final separator
                if parts.last() == Some(&"") {
                    parts.pop();
                }
                parts
                    .into_iter()
                    .map(|x| x.trim_end_matches('\n').to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn git_output(&self, args: &[&str]) -> Result<std::process::Output> {
        Command::new(env_git())
            .args(GLOBAL_ARGS)
            .args(args)
            .current_dir(&self.toplevel)
            .output()
            .map_err(|e| anyhow!("failed to spawn git: {e}"))
    }

    /// Like `git_string`, but run in `dir` instead of the toplevel
    /// (magit-git-command vs magit-git-command-topdir).
    pub fn git_string_in(&self, dir: &Path, args: &[&str]) -> Result<String> {
        let out = Command::new(env_git())
            .args(GLOBAL_ARGS)
            .args(args)
            .current_dir(dir)
            .output()
            .map_err(|e| anyhow!("failed to spawn git: {e}"))?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        record(dir, args, out.status.success(), out.status.code(), &stderr);
        if !out.status.success() {
            bail!(
                "git {} failed: {}",
                args.first().unwrap_or(&""),
                stderr.trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    /// Run git with `input` on stdin (e.g. `git apply -`).
    pub fn git_with_input(&self, args: &[&str], input: &[u8]) -> Result<String> {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = Command::new(env_git())
            .args(GLOBAL_ARGS)
            .args(args)
            .current_dir(&self.toplevel)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("failed to spawn git: {e}"))?;
        child
            .stdin
            .take()
            .context("git stdin unavailable")?
            .write_all(input)?;
        let out = child.wait_with_output()?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        record(&self.toplevel, args, out.status.success(), out.status.code(), &stderr);
        if !out.status.success() {
            bail!(
                "git {} failed: {}",
                args.first().unwrap_or(&""),
                stderr.trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    /// Run git, inherit stdio so editors/hooks can run (magit-call-git).
    pub fn git_interactive(&self, args: &[&str]) -> Result<()> {
        let st = Command::new(env_git())
            .args(GLOBAL_ARGS)
            .args(args)
            .current_dir(&self.toplevel)
            .status()
            .map_err(|e| anyhow!("failed to spawn git: {e}"))?;
        record(&self.toplevel, args, st.success(), st.code(), "");
        if !st.success() {
            bail!(
                "git {} failed: {}",
                args.first().unwrap_or(&""),
                st
            );
        }
        Ok(())
    }

    /// Run a sequencer command (rebase -i, --edit-todo, ...) with explicit
    /// editors, like Magit's `GIT_SEQUENCE_EDITOR` / with-editor handling.
    /// The environment variables are set so they win over the user's own
    /// `GIT_SEQUENCE_EDITOR` / `GIT_EDITOR` unless `Editor::User` is asked for.
    pub fn git_with_editors(&self, todo: Editor, message: Editor, args: &[&str]) -> Result<()> {
        let mut cmd = Command::new(env_git());
        cmd.args(["-c", "color.ui=false"]).current_dir(&self.toplevel);
        match todo {
            Editor::Accept => {
                cmd.env("GIT_SEQUENCE_EDITOR", "true");
            }
            Editor::Command(c) => {
                cmd.env("GIT_SEQUENCE_EDITOR", c);
            }
            Editor::User => {}
        }
        match message {
            Editor::Accept => {
                cmd.env("GIT_EDITOR", "true");
            }
            Editor::Command(c) => {
                cmd.env("GIT_EDITOR", c);
            }
            Editor::User => {}
        }
        let st = cmd
            .args(args)
            .status()
            .map_err(|e| anyhow!("failed to spawn git: {e}"))?;
        record(&self.toplevel, args, st.success(), st.code(), "");
        if !st.success() {
            bail!("git {} failed: {}", args.first().unwrap_or(&""), st);
        }
        Ok(())
    }

    /// Run a side-effecting git command and return its stdout (recorded in the
    /// process log, unlike `git_string` which is used for queries).
    pub fn git_fmt(&self, args: &[String]) -> Result<String> {
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = self.git_output(&refs)?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        record(&self.toplevel, &refs, out.status.success(), out.status.code(), &stderr);
        if !out.status.success() {
            bail!(
                "git {} failed: {}",
                refs.first().unwrap_or(&""),
                stderr.trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    /// Anything modified/staged/untracked? (magit-anything-modified-p)
    pub fn anything_modified(&self) -> bool {
        !self.staged_files().is_empty()
            || !self.unstaged_files().is_empty()
            || !self.untracked_files().is_empty()
    }

    pub fn anything_staged(&self) -> bool {
        !self.staged_files().is_empty()
    }

    /// Rebase / merge / revert / cherry-pick in progress?
    pub fn merge_in_progress(&self) -> bool {
        self.gitdir.join("MERGE_HEAD").exists()
    }
    /// magit-rebase-in-progress-p: `rebase-merge/` or `rebase-apply/onto`
    /// (`git am` also uses `rebase-apply/`, but without `onto`).
    pub fn rebase_in_progress(&self) -> bool {
        let g = &self.gitdir;
        g.join("rebase-merge").exists() || g.join("rebase-apply/onto").exists()
    }
    /// magit-am-in-progress-p
    pub fn am_in_progress(&self) -> bool {
        self.gitdir.join("rebase-apply/applying").exists()
    }
    /// magit-bisect-in-progress-p
    pub fn bisect_in_progress(&self) -> bool {
        self.gitdir.join("BISECT_LOG").exists()
    }
    pub fn revert_in_progress(&self) -> bool {
        self.gitdir.join("REVERT_HEAD").exists()
    }
    pub fn cherry_in_progress(&self) -> bool {
        self.gitdir.join("CHERRY_PICK_HEAD").exists()
    }
    /// magit-sequencer-in-progress-p: a cherry-pick or revert is stopped
    /// (including multi-commit sequences recorded in .git/sequencer).
    pub fn pick_or_revert_in_progress(&self) -> bool {
        self.cherry_in_progress()
            || self.revert_in_progress()
            || self.gitdir.join("sequencer/todo").exists()
    }
    /// Any operation in progress (merge / rebase / revert / cherry-pick).
    pub fn sequencer_in_progress(&self) -> bool {
        self.merge_in_progress()
            || self.rebase_in_progress()
            || self.revert_in_progress()
            || self.cherry_in_progress()
    }
}
