//! git/query.rs — in-process answers to the read-only git queries the
//! buffers repeat on every refresh (magit uses the git CLI for these).
//!
//! A libgit2 `Repository` is opened once per refresh in the thread that
//! builds the buffer (`Scope`), so refs and config are always the ones on
//! disk at the beginning of that refresh. Only an exact whitelist of
//! argument shapes is answered; anything else — diff, log, reflog text,
//! writes, commands with an environment — still goes to the git CLI. A
//! query libgit2 cannot answer falls back to the CLI as well, so a bare
//! repository, an unusual config or a new git feature cannot change what
//! the buffers show.

use git2::{DescribeFormatOptions, DescribeOptions, Oid, Repository};
use std::cell::RefCell;
use std::path::Path;
use std::process::Output;

thread_local! {
    /// The repository of the refresh running on this thread, if any.
    static REPO: RefCell<Option<Repository>> = const { RefCell::new(None) };
}

/// Keeps the in-process repository for one refresh. Nested scopes keep
/// the outer one (a buffer builder may call another builder).
pub struct Scope {
    installed: bool,
}

/// Open DIR's repository for the queries of the current thread. When
/// libgit2 cannot open it, every query falls back to the git CLI.
pub fn enter(dir: &Path) -> Scope {
    let mut installed = false;
    REPO.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            if let Some(repo) = Repository::discover(dir).ok().filter(|r| r.workdir().is_some()) {
                *slot = Some(repo);
                installed = true;
            }
        }
    });
    Scope { installed }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if self.installed {
            REPO.with(|slot| *slot.borrow_mut() = None);
        }
    }
}

/// The result of a query: use OUT, report a non-zero exit, or run the CLI.
pub enum Answer {
    Ok(String),
    Failed,
    Fallback,
}

impl Answer {
    fn some(s: Option<String>) -> Self {
        match s {
            Some(s) => Self::Ok(s),
            None => Self::Failed,
        }
    }

    fn output(self) -> Option<Output> {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            let (stdout, ok) = match self {
                Self::Ok(s) => (s, true),
                Self::Failed => (String::new(), false),
                Self::Fallback => return None,
            };
            Some(Output {
                status: std::process::ExitStatus::from_raw(if ok { 0 } else { 1 << 8 }),
                stdout: stdout.into_bytes(),
                stderr: Vec::new(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = self;
            None
        }
    }
}

/// Try to answer `git ARGS` in-process. `None` means "run the git CLI".
pub fn answer(args: &[&str]) -> Option<Output> {
    REPO.with(|slot| {
        let slot = slot.borrow();
        slot.as_ref().and_then(|repo| string(repo, args).output())
    })
}

/// Try to answer a `git_lines`-style listing in-process.
pub fn answer_lines(args: &[&str]) -> Option<Vec<String>> {
    REPO.with(|slot| {
        let slot = slot.borrow();
        let repo = slot.as_ref()?;
        lines(repo, args)
    })
}

/// Whether the in-process backend is answering queries on this thread.
pub fn active() -> bool {
    REPO.with(|slot| slot.borrow().is_some())
}

/// Resolve REV to a commit, including `branch@{upstream}` (which
/// libgit2's revparse does not understand).
fn oid(repo: &Repository, rev: &str) -> Option<Oid> {
    if let Some(b) = rev.strip_suffix("@{u}").or_else(|| rev.strip_suffix("@{upstream}")) {
        let full = repo.branch_upstream_name(&format!("refs/heads/{b}")).ok()?;
        let full = full.as_str().ok()?.to_string();
        return repo.refname_to_id(&full).ok();
    }
    let obj = repo.revparse_single(rev).ok()?;
    Some(obj.peel_to_commit().ok()?.id())
}

/// The short name of `branch@{upstream}`, as `rev-parse --abbrev-ref`.
fn upstream(repo: &Repository, rev: &str) -> Option<String> {
    let b = rev.strip_suffix("@{u}").or_else(|| rev.strip_suffix("@{upstream}"))?;
    let full = repo.branch_upstream_name(&format!("refs/heads/{b}")).ok()?;
    Some(short_ref(full.as_str().ok()?).to_string())
}

fn short_ref(name: &str) -> &str {
    name.strip_prefix("refs/remotes/")
        .or_else(|| name.strip_prefix("refs/heads/"))
        .or_else(|| name.strip_prefix("refs/tags/"))
        .unwrap_or(name)
}

/// `git rev-parse --verify` / `--short` / `--abbrev-ref`.
fn rev_parse(repo: &Repository, rev: &str, kind: &str) -> Option<String> {
    if kind == "--abbrev-ref" {
        return upstream(repo, rev);
    }
    if rev.ends_with("^{commit}") {
        return oid(repo, rev).map(|o| o.to_string());
    }
    let obj = repo.revparse_single(rev).ok()?;
    match kind {
        "--short" => obj.short_id().ok().and_then(|b| b.as_str().ok().map(str::to_string)),
        "--verify" => Some(obj.id().to_string()),
        _ => None,
    }
}

/// All the values of a config variable (`git config --get-all VAR`).
fn config_all(repo: &Repository, var: &str) -> Vec<String> {
    let Ok(cfg) = repo.config() else { return Vec::new() };
    let Ok(mut entries) = cfg.entries(None) else { return Vec::new() };
    let mut v = Vec::new();
    while let Some(e) = entries.next() {
        let Ok(e) = e else { continue };
        if e.name().is_ok_and(|n| n.eq_ignore_ascii_case(var)) {
            if let Ok(value) = e.value() {
                v.push(value.to_string());
            }
        }
    }
    v
}

/// `git rev-list --count` / `--left-right --count` of a range.
fn counts(repo: &Repository, range: &str) -> Option<String> {
    if let Some((a, b)) = range.split_once("...") {
        let (ahead, behind) = repo.graph_ahead_behind(oid(repo, a)?, oid(repo, b)?).ok()?;
        return Some(format!("{ahead}\t{behind}"));
    }
    if let Some((a, b)) = range.split_once("..") {
        // `git rev-list --count A..B`: commits in B that are not in A
        let n = repo.graph_ahead_behind(oid(repo, b)?, oid(repo, a)?).ok()?.0;
        return Some(n.to_string());
    }
    None
}

/// The subject / short hash / author of one commit, for the fixed
/// `--format` strings of the header lines and the refs margin.
fn commit(repo: &Repository, rev: &str, format: &str) -> Option<String> {
    let c = repo.revparse_single(rev).ok()?.peel_to_commit().ok()?;
    let short = c.as_object().short_id().ok()?.as_str().ok()?.to_string();
    let summary = c.summary().ok().flatten().unwrap_or("");
    match format {
        "%h%x00%s" => Some(format!("{short}\0{summary}")),
        "%s" => Some(summary.to_string()),
        "%cN%x00%ct" => Some(format!("{}\0{}", c.committer().name().unwrap_or(""), c.time().seconds())),
        "%aN%x00%at" => Some(format!("{}\0{}", c.author().name().unwrap_or(""), c.time().seconds())),
        _ => None,
    }
}

/// `git describe --long --tags`.
fn describe(repo: &Repository) -> Option<String> {
    let mut opts = DescribeOptions::new();
    opts.describe_tags();
    let d = repo.describe(&opts).ok()?;
    let mut fmt = DescribeFormatOptions::new();
    fmt.always_use_long_format(true);
    d.format(Some(&fmt)).ok()
}

/// Ref names of `git for-each-ref`, in the CLI's refname order.
fn for_each_ref(repo: &Repository, args: &[&str]) -> Option<Vec<String>> {
    let mut format = None;
    let mut patterns = Vec::new();
    for a in args.iter().skip(1) {
        if let Some(f) = a.strip_prefix("--format=") {
            format = Some(f);
        } else if a.starts_with('-') {
            // --points-at, --contains, --sort, ...: let the CLI do it
            return None;
        } else {
            patterns.push(*a);
        }
    }
    let short = match format {
        Some("%(refname:short)") => true,
        Some("%(refname)") => false,
        _ => return None,
    };
    if patterns.is_empty() {
        patterns.push("refs");
    }
    let mut v: Vec<String> = Vec::new();
    for p in patterns {
        let glob = format!("{}/*", p.trim_end_matches('/'));
        let refs = repo.references_glob(&glob).ok()?;
        for r in refs {
            let Ok(r) = r else { continue };
            let Ok(name) = r.name() else { continue };
            v.push(if short { short_ref(name).to_string() } else { name.to_string() });
        }
    }
    v.sort();
    v.dedup();
    Some(v)
}

/// Lines of the listing commands (`git_lines`).
fn lines(repo: &Repository, args: &[&str]) -> Option<Vec<String>> {
    match args {
        ["for-each-ref", ..] => for_each_ref(repo, args),
        ["remote"] => {
            let mut v: Vec<String> = repo.remotes().ok()?.iter().filter_map(|s| s.ok().flatten()).map(str::to_string).collect();
            v.sort();
            Some(v)
        }
        ["tag", "--list"] | ["tag", "-l"] => {
            let mut v: Vec<String> = repo.tag_names(None).ok()?.iter().filter_map(|s| s.ok().flatten()).map(str::to_string).collect();
            v.sort();
            Some(v)
        }
        ["config", "--get-all", var] => Some(config_all(repo, var)),
        ["stash", "list", "--format=%gd|%gs"] => stashes(repo),
        _ => None,
    }
}

/// The lines of `git stash list --format=%gd|%gs` (newest first).
fn stashes(repo: &Repository) -> Option<Vec<String>> {
    let log = repo.reflog("refs/stash").ok()?;
    let n = log.len();
    let mut v = Vec::new();
    for i in 0..n {
        // the reflog file is oldest first; stash@{0} is the newest
        let e = log.get(n - 1 - i)?;
        let msg = e.message().ok().flatten().unwrap_or("").lines().next().unwrap_or("");
        v.push(format!("stash@{{{i}}}|{msg}"));
    }
    Some(v)
}

/// Answer a `git_string` / `git_ok` / `git_output` query.
fn string(repo: &Repository, args: &[&str]) -> Answer {
    match args {
        ["symbolic-ref", "--short", "HEAD"] => {
            let Ok(head) = repo.find_reference("HEAD") else { return Answer::Fallback };
            match head.symbolic_target() {
                Ok(Some(t)) => Answer::some(t.strip_prefix("refs/heads/").map(str::to_string)),
                Ok(None) => Answer::Failed, // detached HEAD
                Err(_) => Answer::Fallback,
            }
        }
        ["rev-parse", kind, rev] if matches!(*kind, "--verify" | "--short" | "--abbrev-ref") => {
            match rev_parse(repo, rev, kind) {
                Some(s) => Answer::Ok(s),
                // an unknown rev expression is the CLI's job to resolve
                None if kind == &"--abbrev-ref" || rev.contains('@') || rev.contains("..") => Answer::Fallback,
                None => Answer::Failed,
            }
        }
        ["rev-parse", "--verify", "--quiet", rev] => Answer::some(oid(repo, rev).map(|o| o.to_string())),
        ["config", key] | ["config", "--get", key] => {
            Answer::some(repo.config().ok().and_then(|c| c.get_string(key).ok()))
        }
        ["config", "--type=bool", key] => {
            Answer::some(repo.config().ok().and_then(|c| c.get_bool(key).ok()).map(|b| b.to_string()))
        }
        ["config", "--get-all", var] => Answer::Ok(config_all(repo, var).join("\n")),
        ["rev-list", "--left-right", "--count", range]
        | ["rev-list", "--count", "--left-right", range]
        | ["rev-list", "--count", range] => match counts(repo, range) {
            Some(s) => Answer::Ok(s),
            None => Answer::Fallback,
        },
        ["log", "-1", "--no-walk", fmt, rev, "--"] => match commit(repo, rev, fmt) {
            Some(s) => Answer::Ok(s),
            None => Answer::Fallback,
        },
        ["describe", "--long", "--tags"] => match describe(repo) {
            Some(s) => Answer::Ok(s),
            None => Answer::Fallback,
        },
        ["merge-base", "--is-ancestor", a, b] => {
            let (Some(a), Some(b)) = (oid(repo, a), oid(repo, b)) else { return Answer::Fallback };
            // git merge-base --is-ancestor A A succeeds; libgit2's
            // graph_descendant_of does not count a commit as its own
            // descendant (repo.rs:2720)
            if a == b {
                return Answer::Ok(String::new());
            }
            match repo.graph_descendant_of(b, a) {
                Ok(true) => Answer::Ok(String::new()),
                Ok(false) => Answer::Failed,
                Err(_) => Answer::Fallback,
            }
        }
        // `git_lines` listings that a caller happens to ask for via
        // git_string: keep the same text the CLI would print.
        _ => match lines(repo, args) {
            Some(v) => Answer::Ok(v.join("\n")),
            None => Answer::Fallback,
        },
    }
}
