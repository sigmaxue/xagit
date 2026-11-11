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

/// The full refname of `branch@{u}` / `branch@{upstream}`. Without a
/// branch prefix git uses the upstream of the current branch.
fn upstream_ref(repo: &Repository, rev: &str) -> Option<String> {
    let b = rev.strip_suffix("@{u}").or_else(|| rev.strip_suffix("@{upstream}"))?;
    let branch = if b.is_empty() {
        let head = repo.head().ok()?;
        if !head.is_branch() {
            return None;
        }
        head.shorthand().ok()?.to_string()
    } else {
        b.to_string()
    };
    let full = repo.branch_upstream_name(&format!("refs/heads/{branch}")).ok()?;
    full.as_str().ok().map(str::to_string)
}

/// Resolve REV to a commit, including `branch@{upstream}` (which
/// libgit2's revparse does not understand).
fn oid(repo: &Repository, rev: &str) -> Option<Oid> {
    if let Some(full) = upstream_ref(repo, rev) {
        return repo.refname_to_id(&full).ok();
    }
    // libgit2's revparse has no `^{commit}` operator; peeling is explicit.
    let rev = rev.strip_suffix("^{commit}").unwrap_or(rev);
    let obj = repo.revparse_single(rev).ok()?;
    Some(obj.peel_to_commit().ok()?.id())
}

/// Whether `branch@{u}` certainly has no upstream, so git would fail: the
/// branch exists and has no `branch.<name>.merge`, or (without a branch
/// prefix) HEAD is detached. Anything less certain is the CLI's to judge.
fn no_upstream(repo: &Repository, rev: &str) -> bool {
    let Some(b) = rev.strip_suffix("@{u}").or_else(|| rev.strip_suffix("@{upstream}")) else {
        return false;
    };
    let branch = if b.is_empty() {
        let Ok(head) = repo.head() else { return false }; // unborn: the CLI's case
        if !head.is_branch() {
            return true;
        }
        match head.shorthand() {
            Ok(s) => s.to_string(),
            Err(_) => return false,
        }
    } else {
        b.to_string()
    };
    if repo.find_reference(&format!("refs/heads/{branch}")).is_err() {
        return false;
    }
    let Ok(cfg) = repo.config() else { return false };
    let missing = matches!(cfg.get_entry(&format!("branch.{branch}.merge")), Err(e) if e.code() == git2::ErrorCode::NotFound);
    missing
}

/// Whether the repository has no tag at all, so `git describe --tags` and
/// `git describe --contains` cannot name anything.
fn no_tags(repo: &Repository) -> bool {
    repo.references_glob("refs/tags/*").is_ok_and(|mut refs| refs.next().is_none())
}

/// The short name of `branch@{upstream}`, as `rev-parse --abbrev-ref`.
fn upstream(repo: &Repository, rev: &str) -> Option<String> {
    Some(short_ref(&upstream_ref(repo, rev)?).to_string())
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
        "%cN%x00%ct" => Some(format!("{}\0{}", c.committer().name().unwrap_or(""), c.committer().when().seconds())),
        "%aN%x00%at" => Some(format!("{}\0{}", c.author().name().unwrap_or(""), c.author().when().seconds())),
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
        } else if a.starts_with('-') || a.contains('*') {
            // --points-at, --contains, --sort, glob patterns: the CLI's job
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
    let prefixes: Vec<String> = if patterns.is_empty() {
        vec!["refs/".to_string()]
    } else {
        patterns
            .iter()
            .map(|p| format!("{}/", p.trim_end_matches('/')))
            .collect()
    };
    let mut names: Vec<String> = repo
        .references()
        .ok()?
        .filter_map(|r| r.ok())
        .filter_map(|r| r.name().ok().map(str::to_string))
        .filter(|name| prefixes.iter().any(|p| name.starts_with(p)))
        .collect();
    // git sorts by full refname even when it prints the short one
    names.sort();
    names.dedup();
    Some(names.into_iter().map(|n| if short { short_ref(&n).to_string() } else { n }).collect())
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
        // -q only silences the message of a detached HEAD
        ["symbolic-ref", "--short", "HEAD"]
        | ["symbolic-ref", "--short", "-q", "HEAD"]
        | ["symbolic-ref", "-q", "--short", "HEAD"] => {
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
                // the status buffer asks this on every refresh; most local
                // branches have no upstream
                None if kind == &"--abbrev-ref" && no_upstream(repo, rev) => Answer::Failed,
                // an unknown rev expression is the CLI's job to resolve
                None if kind == &"--abbrev-ref" || rev.contains('@') || rev.contains("..") => Answer::Fallback,
                None => Answer::Failed,
            }
        }
        ["rev-parse", "--verify", "--quiet", rev] | ["rev-parse", "--verify", "-q", rev] => {
            Answer::some(oid(repo, rev).map(|o| o.to_string()))
        }
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
        ["log", "-1", "--no-walk", fmt, rev, "--"] => {
            let Some(fmt) = fmt.strip_prefix("--format=") else { return Answer::Fallback };
            match commit(repo, rev, fmt) {
                Some(s) => Answer::Ok(s),
                None => Answer::Fallback,
            }
        }
        ["describe", "--long", "--tags"] => match describe(repo) {
            Some(s) => Answer::Ok(s),
            None if no_tags(repo) => Answer::Failed,
            None => Answer::Fallback,
        },
        // name-rev's walk stays with the CLI; only "no tag at all" is known
        ["describe", "--contains", _] if no_tags(repo) => Answer::Failed,
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::git::exec::Git;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// The real git CLI, with the same global arguments `Git` uses.
    fn cli(dir: &Path, args: &[&str]) -> (bool, String) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "core.editor=true", "-c", "color.ui=false"])
            .args(args)
            .output()
            .unwrap();
        (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string())
    }

    fn must(dir: &Path, args: &[&str]) {
        let (ok, out) = cli(dir, args);
        assert!(ok, "git {args:?} failed: {out}");
    }

    struct Fixture {
        _td: tempfile::TempDir,
        dir: PathBuf,
    }

    impl Fixture {
        fn path(&self) -> &Path {
            &self.dir
        }
    }

    /// Two commits (with distinct author and committer times), a tag on
    /// the first, an upstream, a nested branch and a multi-valued config.
    fn fixture() -> Fixture {
        let td = tempfile::tempdir().unwrap();
        let dir = td.path().join("repo");
        let bare = td.path().join("origin.git");
        fs::create_dir(&dir).unwrap();
        must(&dir, &["init", "-q"]);
        must(&dir, &["config", "user.name", "Tester"]);
        must(&dir, &["config", "user.email", "tester@example.com"]);
        must(&dir, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        fs::write(dir.join("a.txt"), "one\n").unwrap();
        must(&dir, &["add", "a.txt"]);
        must(&dir, &["commit", "-qm", "first"]);
        must(&dir, &["tag", "v1"]);
        fs::write(dir.join("b.txt"), "two\n").unwrap();
        must(&dir, &["add", "b.txt"]);
        let out = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["commit", "-qm", "second commit"])
            .env("GIT_AUTHOR_DATE", "2001-02-03T04:05:06 +0000")
            .env("GIT_COMMITTER_DATE", "2002-03-04T05:06:07 +0000")
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        must(td.path(), &["init", "-q", "--bare", bare.to_str().unwrap()]);
        must(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
        must(&dir, &["push", "-q", "-u", "origin", "main"]);
        must(&dir, &["branch", "feature/nested"]);
        must(&dir, &["config", "--add", "xagit.test", "one"]);
        must(&dir, &["config", "--add", "xagit.test", "two"]);
        Fixture { _td: td, dir }
    }

    /// An in-process answer must be byte-for-byte what the CLI prints,
    /// including the exit status (queries like --is-ancestor fail).
    fn same_as_cli(dir: &Path, args: &[&str]) {
        let (ok, text) = cli(dir, args);
        let out = answer(args)
            .unwrap_or_else(|| panic!("expected an in-process answer for git {args:?}"));
        assert_eq!(out.status.success(), ok, "exit status for git {args:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim_end_matches('\n'),
            text.trim_end_matches('\n'),
            "stdout for git {args:?}"
        );
    }

    fn cli_lines(dir: &Path, args: &[&str]) -> Vec<String> {
        let (ok, text) = cli(dir, args);
        assert!(ok, "git {args:?} failed");
        text.trim_end_matches('\n').split('\n').map(str::to_string).collect()
    }

    #[test]
    fn scope_is_thread_local_and_reentrant() {
        assert!(!active());
        assert!(answer(&["symbolic-ref", "--short", "HEAD"]).is_none());
        let f = fixture();
        let outer = enter(f.path());
        assert!(active());
        {
            let inner = enter(f.path());
            assert!(active());
            drop(inner);
        }
        assert!(active(), "a nested scope must not uninstall the outer one");
        drop(outer);
        assert!(!active());
        assert!(answer(&["symbolic-ref", "--short", "HEAD"]).is_none());
    }

    #[test]
    fn whitelisted_queries_match_the_cli() {
        let f = fixture();
        let _scope = enter(f.path());
        for args in [
            vec!["symbolic-ref", "--short", "HEAD"],
            vec!["symbolic-ref", "--short", "-q", "HEAD"],
            vec!["rev-parse", "--verify", "HEAD"],
            vec!["rev-parse", "--short", "HEAD"],
            vec!["rev-parse", "--verify", "HEAD^{commit}"],
            vec!["rev-parse", "--verify", "HEAD~1^{commit}"],
            vec!["rev-parse", "--verify", "--quiet", "HEAD"],
            vec!["rev-parse", "--verify", "-q", "HEAD"],
            vec!["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
            vec!["rev-parse", "--abbrev-ref", "@{u}"],
            vec!["rev-parse", "--abbrev-ref", "main@{u}"],
            vec!["config", "user.name"],
            vec!["config", "--get", "user.email"],
            vec!["config", "--type=bool", "core.bare"],
            vec!["config", "--get-all", "xagit.test"],
            vec!["rev-list", "--count", "HEAD~1..HEAD"],
            vec!["rev-list", "--count", "@{u}..HEAD"],
            vec!["rev-list", "--left-right", "--count", "HEAD...main@{u}"],
            vec!["describe", "--long", "--tags"],
            vec!["merge-base", "--is-ancestor", "HEAD", "HEAD"],
            vec!["merge-base", "--is-ancestor", "HEAD~1", "HEAD"],
            vec!["merge-base", "--is-ancestor", "HEAD", "HEAD~1"],
            vec!["merge-base", "--is-ancestor", "HEAD", "main@{u}"],
            vec!["log", "-1", "--no-walk", "--format=%h%x00%s", "HEAD", "--"],
            vec!["log", "-1", "--no-walk", "--format=%s", "HEAD", "--"],
            vec!["log", "-1", "--no-walk", "--format=%cN%x00%ct", "HEAD", "--"],
            vec!["log", "-1", "--no-walk", "--format=%aN%x00%at", "HEAD", "--"],
            vec!["remote"],
            vec!["tag", "--list"],
        ] {
            same_as_cli(f.path(), &args);
        }
    }

    #[test]
    fn failed_queries_keep_the_cli_exit_status() {
        let f = fixture();
        let scope = enter(f.path());
        same_as_cli(f.path(), &["rev-parse", "--verify", "no-such-ref"]);
        same_as_cli(f.path(), &["merge-base", "--is-ancestor", "HEAD", "HEAD~1"]);
        drop(scope);
        must(f.path(), &["checkout", "-q", "--detach", "HEAD"]);
        let _detached = enter(f.path());
        same_as_cli(f.path(), &["symbolic-ref", "--short", "HEAD"]);
        same_as_cli(f.path(), &["symbolic-ref", "--short", "-q", "HEAD"]);
    }

    /// A branch without upstream and a repository without tags: the
    /// status buffer asks these on every refresh, and the answer "no" must
    /// not need the CLI.
    #[test]
    fn missing_upstream_and_tags_fail_in_process() {
        let td = tempfile::tempdir().unwrap();
        let dir = td.path().to_path_buf();
        must(&dir, &["init", "-q"]);
        must(&dir, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        must(&dir, &["-c", "user.name=T", "-c", "user.email=t@e", "commit", "-q", "--allow-empty", "-m", "one"]);
        let queries = [
            vec!["rev-parse", "--abbrev-ref", "@{u}"],
            vec!["rev-parse", "--abbrev-ref", "main@{u}"],
            vec!["rev-parse", "--abbrev-ref", "main@{upstream}"],
            vec!["describe", "--long", "--tags"],
            vec!["describe", "--contains", "HEAD"],
        ];
        let scope = enter(&dir);
        for args in &queries {
            same_as_cli(&dir, args);
        }
        // a branch that does not exist stays with the CLI
        assert!(answer(&["rev-parse", "--abbrev-ref", "nope@{u}"]).is_none());
        drop(scope);

        must(&dir, &["checkout", "-q", "--detach", "HEAD"]);
        let scope = enter(&dir);
        same_as_cli(&dir, &["rev-parse", "--abbrev-ref", "@{u}"]);
        drop(scope);

        // with a tag, describe --contains is the CLI's again
        must(&dir, &["tag", "v1"]);
        let _scope = enter(&dir);
        assert!(answer(&["describe", "--contains", "HEAD"]).is_none());
        same_as_cli(&dir, &["describe", "--long", "--tags"]);
    }

    #[test]
    fn listing_queries_match_the_cli() {
        let f = fixture();
        let _scope = enter(f.path());
        for args in [
            vec!["for-each-ref", "--format=%(refname)", "refs/heads/", "refs/remotes/", "refs/tags/"],
            vec!["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
            vec!["for-each-ref", "--format=%(refname)", "refs"],
            vec!["remote"],
            vec!["tag", "--list"],
            vec!["config", "--get-all", "xagit.test"],
        ] {
            let expected = cli_lines(f.path(), &args);
            let got = answer_lines(&args)
                .unwrap_or_else(|| panic!("expected in-process lines for git {args:?}"));
            assert_eq!(got, expected, "git {args:?}");
        }
    }

    #[test]
    fn stash_lines_match_the_cli() {
        let f = fixture();
        fs::write(f.path().join("a.txt"), "changed\n").unwrap();
        must(f.path(), &["stash", "push", "-qm", "wip"]);
        let _scope = enter(f.path());
        let args = ["stash", "list", "--format=%gd|%gs"];
        assert_eq!(answer_lines(&args).unwrap(), cli_lines(f.path(), &args));
    }

    #[test]
    fn unknown_queries_fall_back_to_the_cli() {
        let f = fixture();
        let _scope = enter(f.path());
        for args in [
            vec!["status", "--porcelain=v1", "-z"],
            vec!["diff", "--stat"],
            vec!["log", "--oneline", "-5"],
            vec!["branch", "-a"],
            vec!["rev-parse", "--verify", "--symbolic-full-name", "HEAD"],
            vec!["for-each-ref", "--format=%(refname:short)", "--contains", "HEAD"],
            vec!["for-each-ref", "--format=%(refname:short)", "--points-at", "HEAD", "refs/heads"],
            vec!["for-each-ref", "--format=%(refname:short)", "refs/heads/*"],
            vec!["describe", "--tags"],
        ] {
            assert!(answer(&args).is_none(), "git {args:?} must run the CLI");
            assert!(answer_lines(&args).is_none(), "git {args:?} must run the CLI");
        }
    }

    #[test]
    fn unusable_repositories_never_answer() {
        let td = tempfile::tempdir().unwrap();
        let plain = td.path().join("not-a-repo");
        fs::create_dir(&plain).unwrap();
        let no_scope = enter(&plain);
        assert!(!active());
        assert!(answer(&["symbolic-ref", "--short", "HEAD"]).is_none());
        drop(no_scope);

        let bare = td.path().join("bare.git");
        must(td.path(), &["init", "-q", "--bare", bare.to_str().unwrap()]);
        let _bare_scope = enter(&bare);
        assert!(!active(), "bare repos have no worktree, so stay on the CLI");
        assert!(answer(&["symbolic-ref", "--short", "HEAD"]).is_none());
    }

    #[test]
    fn environment_queries_are_not_intercepted() {
        let f = fixture();
        let _scope = enter(f.path());
        let g = Git::discover(f.path()).unwrap();
        let env = [
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "user.name"),
            ("GIT_CONFIG_VALUE_0", "from-env"),
        ];
        let with_env = g.git_output_env(&env, &["config", "user.name"]).unwrap();
        assert_eq!(String::from_utf8_lossy(&with_env.stdout).trim(), "from-env");
        let in_process = g.git_output(&["config", "user.name"]).unwrap();
        assert_eq!(String::from_utf8_lossy(&in_process.stdout).trim(), "Tester");
    }
}
