//! git/exec.rs — running git commands (mirrors magit-git.el behavior)

use anyhow::{anyhow, bail, Context, Result};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// Global arguments Magit passes to every git invocation (magit-git-global-arguments).
const GLOBAL_ARGS: [&str; 4] = ["-c", "core.editor=true", "-c", "color.ui=false"];

/// A git command with the standard global arguments and working directory.
/// Callers add their own stdio/environment on top.
fn command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env_git());
    cmd.args(GLOBAL_ARGS).args(args).current_dir(dir);
    cmd
}

/// Recent side-effecting git invocations, shown by `$` (magit-process-buffer).
/// Like Magit, plain queries (status, rev-parse, ...) are not recorded.
static PROCESS_LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const PROCESS_LOG_MAX: usize = 200;

pub(crate) fn record(dir: &Path, args: &[&str], ok: bool, code: Option<i32>, stderr: &str) {
    record_process(dir, &command_label(args), ok, code, stderr);
}

/// Like `record`, for a program that is not git (magit--shell-command).
/// LABEL is the command line as it was typed.
pub fn record_process(dir: &Path, label: &str, ok: bool, code: Option<i32>, stderr: &str) {
    let mut entry = format!(
        "{} {}  [{}]",
        if ok { " " } else { "!" },
        sanitize(label),
        match code {
            Some(c) => format!("exit {c}"),
            None => "signal".into(),
        }
    );
    entry.push_str(&format!("\n    in {}", sanitize(&dir.display().to_string())));
    let stderr = sanitize(stderr);
    for l in stderr.lines().filter(|l| !l.trim().is_empty()) {
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

/// Sanitize diagnostics, including URLs echoed by Git. Never inspect the environment.
pub fn sanitize(text: &str) -> String {
    let mut out = String::new();
    let mut hide_next = false;
    let mut hide_line = false;
    for part in text.split_inclusive(char::is_whitespace) {
        let end = part.trim_end_matches(char::is_whitespace).len();
        let (word, space) = part.split_at(end);
        let lower = word.to_ascii_lowercase();
        let sensitive = ["password", "passwd", "token", "secret", "authorization", "credential", "extraheader", "api_key", "apikey", "private_key"];
        let sensitive_word = sensitive.iter().any(|key| lower.contains(key));
        if sensitive_word || hide_next || hide_line {
            out.push_str("[redacted]");
            hide_next = sensitive_word && !word.contains('=');
            hide_line = hide_line || lower.contains("authorization") || lower.contains("private_key");
        } else if let Some(start) = word.find("://") {
            let authority = start + 3;
            let tail = &word[authority..];
            let stop = tail.find(['/','?','#']).unwrap_or(tail.len());
            out.push_str(&word[..authority]);
            if let Some(at) = tail[..stop].rfind('@') {
                out.push_str("[redacted]@");
                out.push_str(&tail[at + 1..stop]);
            } else {
                out.push_str(&tail[..stop]);
            }
            let rest = &tail[stop..];
            out.push_str(rest.split(['?', '#']).next().unwrap_or(""));
            if rest.contains(['?', '#']) { out.push_str("?[redacted]"); }
        } else {
            out.extend(word.chars().filter(|c| !c.is_control()));
        }
        out.extend(space.chars().filter(|c| matches!(c, '\n' | '\t' | ' ')));
        if space.contains('\n') { hide_next = false; hide_line = false; }
    }
    out
}

pub(crate) fn command_label(args: &[&str]) -> String {
    let mut hide_next = false;
    let mut parts = vec!["git".to_string()];
    for arg in args {
        let hidden = hide_next || arg.starts_with("--message=") || arg.starts_with("--config-env=");
        hide_next = matches!(*arg, "-c" | "-m" | "--message" | "--config-env");
        parts.push(if hidden { "[redacted]".into() } else { format!("{:?}", sanitize(arg)) });
    }
    parts.join(" ")
}

pub(crate) fn failure(dir: &Path, args: &[&str], out: &std::process::Output) -> String {
    let stderr = sanitize(&String::from_utf8_lossy(&out.stderr));
    let stdout = sanitize(&String::from_utf8_lossy(&out.stdout));
    let mut text = format!("{} failed ({})\nin {}\nstderr:\n{}", command_label(args), out.status, sanitize(&dir.display().to_string()), if stderr.trim().is_empty() { "(empty)" } else { stderr.trim() });
    if !stdout.trim().is_empty() { text.push_str(&format!("\nstdout:\n{}", stdout.trim())); }
    let lower = stderr.to_ascii_lowercase();
    let hint = if lower.contains("non-fast-forward") || lower.contains("fetch first") {
        Some("Review remote changes with fetch and compare histories before retrying; do not force-push blindly.")
    } else if lower.contains("authentication failed") || lower.contains("permission denied (publickey)") {
        Some("Check remote access and your credential/SSH configuration; do not paste credentials into diagnostics.")
    } else if lower.contains("would be overwritten") {
        Some("Review local changes; commit or stash the changes you want to keep before retrying.")
    } else if lower.contains("index.lock") && lower.contains("file exists") {
        Some("Check for another running Git process. Only inspect a stale lock after confirming no Git process is using it.")
    } else if lower.contains("conflict") {
        Some("Inspect git status and conflicting files, resolve them, then explicitly continue or abort the operation.")
    } else if lower.contains("could not resolve host") {
        Some("Check the remote hostname, network and DNS settings before retrying.")
    } else { None };
    if let Some(hint) = hint { text.push_str(&format!("\nHint: {hint}")); }
    text
}

/// Whether git (or ssh) failed because it could not ask the user for
/// something on the terminal: credentials, a passphrase, a host key.
pub(crate) fn wants_terminal(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    [
        "terminal prompts disabled",
        "could not read username",
        "could not read password",
        "host key verification failed",
        "read_passphrase",
        "can't open /dev/tty",
        "authentication failed",
        "permission denied (publickey",
    ]
    .iter()
    .any(|p| lower.contains(p))
}

pub(crate) fn spawn_failure(dir: &Path, args: &[&str], e: std::io::Error) -> anyhow::Error {
    anyhow!("{} could not start/wait\nin {}\n{}", command_label(args), sanitize(&dir.display().to_string()), sanitize(&e.to_string()))
}

impl Git {
    /// Find the toplevel of the repository containing `dir` (like magit-toplevel).
    pub fn discover(dir: &Path) -> Result<Git> {
        let out = Command::new(env_git())
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(dir)
            .output()
            .map_err(|e| spawn_failure(dir, &["rev-parse", "--show-toplevel"], e))?;
        if !out.status.success() {
            bail!("{}", failure(dir, &["rev-parse", "--show-toplevel"], &out));
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

    /// magit-list-refnames: the local and remote branches and the tags
    /// (not the symbolic remote HEADs), short names.
    pub fn refnames(&self) -> Vec<String> {
        self.git_lines(&["for-each-ref", "--format=%(refname)", "refs/heads/", "refs/remotes/", "refs/tags/"])
            .into_iter()
            .filter(|r| !r.ends_with("/HEAD"))
            .filter_map(|r| {
                ["refs/heads/", "refs/remotes/", "refs/tags/"]
                    .iter()
                    .find_map(|p| r.strip_prefix(p).map(str::to_string))
            })
            .collect()
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

    /// magit-unmerged-files
    pub fn unmerged_files(&self) -> Vec<String> {
        self.git_lines(&["diff-files", "--name-only", "--diff-filter=U", "-z"])
    }

    pub fn untracked_files(&self) -> Vec<String> {
        self.git_lines(&["ls-files", "--others", "--exclude-standard", "-z"])
    }

    /// Run git and return stdout as String. Err on non-zero exit.
    pub fn git_string(&self, args: &[&str]) -> Result<String> {
        let out = self.git_output(args)?;
        if !out.status.success() {
            bail!("{}", failure(&self.toplevel, args, &out));
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
        if let Some(v) = super::query::answer_lines(args) {
            return v;
        }
        let out = command(&self.toplevel, args).output();
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
        self.git_output_env(&[], args)
    }

    /// `git_output` with the environment variables ENV set.
    pub fn git_output_env(&self, env: &[(&str, &str)], args: &[&str]) -> Result<std::process::Output> {
        // read-only whitelisted queries are answered in-process while a
        // buffer is being refreshed; anything else (and any command with
        // an environment, e.g. GIT_INDEX_FILE) still runs the git CLI
        if env.is_empty() {
            if let Some(out) = super::query::answer(args) {
                return Ok(out);
            }
        }
        command(&self.toplevel, args)
            .envs(env.iter().copied())
            .output()
            .map_err(|e| spawn_failure(&self.toplevel, args, e))
    }

    /// Like `git_string`, but run in `dir` instead of the toplevel
    /// (magit-git-command vs magit-git-command-topdir).
    pub fn git_string_in(&self, dir: &Path, args: &[&str]) -> Result<String> {
        let out = command(dir, args)
            .output()
            .map_err(|e| spawn_failure(dir, args, e))?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        record(dir, args, out.status.success(), out.status.code(), &stderr);
        if !out.status.success() {
            bail!("{}", failure(dir, args, &out));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    /// Run git with `input` on stdin (e.g. `git apply -`).
    pub fn git_with_input(&self, args: &[&str], input: &[u8]) -> Result<String> {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = command(&self.toplevel, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| spawn_failure(&self.toplevel, args, e))?;
        let input_result = child.stdin.take().context("git stdin unavailable")?.write_all(input);
        let out = child.wait_with_output().map_err(|e| spawn_failure(&self.toplevel, args, e))?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        record(&self.toplevel, args, out.status.success(), out.status.code(), &stderr);
        if !out.status.success() {
            bail!("{}", failure(&self.toplevel, args, &out));
        }
        input_result.with_context(|| format!("{}: failed to write Git stdin", command_label(args)))?;
        Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    /// Run git, inherit stdio so editors/hooks can run (magit-call-git).
    pub fn git_interactive(&self, args: &[&str]) -> Result<()> {
        self.git_interactive_env(&[], args)
    }

    /// git_interactive with the environment variables ENV set
    /// (with-environment-variables).
    pub fn git_interactive_env(&self, env: &[(&str, &str)], args: &[&str]) -> Result<()> {
        let mut cmd = command(&self.toplevel, args);
        cmd.envs(env.iter().copied());
        if args.first() == Some(&"checkout") {
            return self.checkout_with_diagnostics(&mut cmd, args);
        }
        let st = cmd.status()
            .map_err(|e| spawn_failure(&self.toplevel, args, e))?;
        record(&self.toplevel, args, st.success(), st.code(), "");
        if !st.success() {
            bail!("{} failed ({st})\nin {}\nOutput was shown in the terminal (interactive editor/hooks).", command_label(args), sanitize(&self.toplevel.display().to_string()));
        }
        Ok(())
    }

    /// Run a long git command (fetch, pull, push …) in the background:
    /// no stdin, its output captured, and away from the terminal the TUI
    /// owns (its own session, no terminal prompts), so it can never write
    /// over the screen or wait for input nobody sees. ON_START gets the
    /// process id (the id of its process group too). The error is the
    /// failure text and whether the command wanted to ask the user
    /// something (a password, a host key) and so should be run again in
    /// the terminal.
    pub fn git_detached(&self, args: &[&str], on_start: impl FnOnce(u32)) -> std::result::Result<(), (String, bool)> {
        use std::process::Stdio;
        let mut cmd = command(&self.toplevel, args);
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // a new session: no controlling terminal for ssh to prompt on
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }
        let child = cmd.spawn().map_err(|e| (format!("{:#}", spawn_failure(&self.toplevel, args, e)), false))?;
        on_start(child.id());
        let out = child.wait_with_output().map_err(|e| (format!("{:#}", spawn_failure(&self.toplevel, args, e)), false))?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        record(&self.toplevel, args, out.status.success(), out.status.code(), &stderr);
        if out.status.success() {
            return Ok(());
        }
        Err((failure(&self.toplevel, args, &out), wants_terminal(&stderr)))
    }

    #[cfg(unix)]
    fn checkout_with_diagnostics(&self, cmd: &mut Command, args: &[&str]) -> Result<()> {
        use std::fs::File;
        use std::io::{Read, Write};
        use std::os::fd::FromRawFd;
        use std::process::Stdio;

        let (mut master, mut slave) = (-1, -1);
        if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) } != 0 {
            return Err(spawn_failure(&self.toplevel, args, std::io::Error::last_os_error()));
        }
        let mut reader = unsafe { File::from_raw_fd(master) };
        let terminal = unsafe { File::from_raw_fd(slave) };
        unsafe {
            libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(slave, libc::F_SETFD, libc::FD_CLOEXEC);
            let mut size: libc::winsize = std::mem::zeroed();
            if libc::ioctl(libc::STDERR_FILENO, libc::TIOCGWINSZ, &mut size) == 0 {
                libc::ioctl(slave, libc::TIOCSWINSZ, &size);
            }
        }
        cmd.stderr(Stdio::from(terminal));
        let child = cmd.spawn();
        cmd.stderr(Stdio::inherit());
        let mut child = child.map_err(|e| spawn_failure(&self.toplevel, args, e))?;
        let capture = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            let mut buf = [0; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        bytes.extend_from_slice(&buf[..n]);
                        let _ = std::io::stderr().write_all(&buf[..n]);
                    }
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
            Ok(bytes)
        });
        let status = child.wait().map_err(|e| spawn_failure(&self.toplevel, args, e))?;
        let stderr = capture.join().map_err(|_| anyhow!("checkout diagnostic reader panicked"))?
            .map_err(|e| spawn_failure(&self.toplevel, args, e))?;
        let out = std::process::Output { status, stdout: Vec::new(), stderr };
        record(&self.toplevel, args, status.success(), status.code(), &String::from_utf8_lossy(&out.stderr));
        if !status.success() {
            bail!("{}", failure(&self.toplevel, args, &out));
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn checkout_with_diagnostics(&self, cmd: &mut Command, args: &[&str]) -> Result<()> {
        cmd.stdin(std::process::Stdio::inherit()).stdout(std::process::Stdio::inherit());
        let out = cmd.output().map_err(|e| spawn_failure(&self.toplevel, args, e))?;
        record(&self.toplevel, args, out.status.success(), out.status.code(), &String::from_utf8_lossy(&out.stderr));
        if !out.status.success() { bail!("{}", failure(&self.toplevel, args, &out)); }
        Ok(())
    }

    /// Run a sequencer command (rebase -i, --edit-todo, ...) with explicit
    /// editors, like Magit's `GIT_SEQUENCE_EDITOR` / with-editor handling.
    /// The environment variables are set so they win over the user's own
    /// `GIT_SEQUENCE_EDITOR` / `GIT_EDITOR` unless `Editor::User` is asked for.
    pub fn git_with_editors(&self, todo: Editor, message: Editor, args: &[&str]) -> Result<()> {
        self.git_with_editors_env(todo, message, &[], args)
    }

    /// git_with_editors with the environment variables ENV set.
    pub fn git_with_editors_env(&self, todo: Editor, message: Editor, env: &[(&str, &str)], args: &[&str]) -> Result<()> {
        let mut cmd = Command::new(env_git());
        cmd.args(["-c", "color.ui=false"]).envs(env.iter().copied()).current_dir(&self.toplevel);
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
        cmd.args(args);
        if args.first() == Some(&"checkout") {
            return self.checkout_with_diagnostics(&mut cmd, args);
        }
        let st = cmd.status()
            .map_err(|e| spawn_failure(&self.toplevel, args, e))?;
        record(&self.toplevel, args, st.success(), st.code(), "");
        if !st.success() {
            bail!("{} failed ({st})\nin {}\nOutput was shown in the terminal (interactive editor/hooks).", command_label(args), sanitize(&self.toplevel.display().to_string()));
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
            bail!("{}", failure(&self.toplevel, &refs, &out));
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

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn empty_stderr_keeps_exit_and_stdout() {
        let out = std::process::Output { status: std::process::ExitStatus::from_raw(7 << 8), stdout: b"hook explanation\nsecond line".to_vec(), stderr: vec![] };
        let text = failure(Path::new("/repo"), &["commit", "-m", "private message"], &out);
        assert!(text.contains("exit status: 7"));
        assert!(text.contains("stderr:\n(empty)"));
        assert!(text.contains("stdout:\nhook explanation\nsecond line"));
        assert!(!text.contains("private message"));
    }

    #[test]
    fn secrets_are_redacted_in_output_and_arguments() {
        let text = sanitize("fatal: https://alice:fakepass@example.com/repo?access=abc\nAuthorization: Bearer fakebearer\ntoken=othersecret\npassword: separatedsecret");
        for secret in ["alice", "fakepass", "access=abc", "fakebearer", "othersecret", "separatedsecret"] { assert!(!text.contains(secret), "{secret}"); }
        assert!(text.contains("example.com/repo"));
        let label = command_label(&["-c", "http.extraHeader=Bearer hidden", "push", "https://u:p@example.com/r"]);
        assert!(!label.contains("hidden"));
        assert!(!label.contains("u:p"));
    }

    #[test]
    fn long_errors_and_original_stderr_survive_with_targeted_hint() {
        let stderr = format!("rejected: non-fast-forward\n{}", "detail\n".repeat(100));
        let out = std::process::Output { status: std::process::ExitStatus::from_raw(1 << 8), stdout: vec![], stderr: stderr.as_bytes().to_vec() };
        let text = failure(Path::new("/repo"), &["push", "origin", "main"], &out);
        assert!(text.contains(stderr.trim()));
        assert!(text.contains("Hint: Review remote changes"));
    }

    #[test]
    fn signal_is_reported() {
        let out = std::process::Output { status: std::process::ExitStatus::from_raw(15), stdout: vec![], stderr: vec![] };
        assert!(failure(Path::new("/repo"), &["fetch"], &out).contains("signal: 15"));
    }

    #[test]
    fn prompts_wanted_are_recognized() {
        assert!(wants_terminal("fatal: could not read Username for 'https://x': terminal prompts disabled"));
        assert!(wants_terminal("Host key verification failed.\nfatal: Could not read from remote repository."));
        assert!(wants_terminal("git@x: Permission denied (publickey).\n"));
        assert!(!wants_terminal("fatal: '/nonexistent' does not appear to be a git repository"));
        assert!(!wants_terminal(" ! [rejected]        main -> main (non-fast-forward)"));
    }
}
