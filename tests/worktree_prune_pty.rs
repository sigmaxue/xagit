//! Regression: an interactive git command whose stderr is captured through
//! a pty must not hang the caller.  If the parent keeps the pty slave fd
//! open, the stderr reader never sees EOF and `join()` blocks forever —
//! this is what made `git worktree prune` freeze the TUI.

mod common;

use common::*;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn interactive_worktree_prune_does_not_hang() {
    let (_td, g, _head) = repo_with_commit();
    // a stale worktree entry makes `git worktree prune` do real work
    let wt = _td.path().join("wt");
    g.git_interactive(&["worktree", "add", "-q", "-b", "wt", &wt.to_string_lossy()]).unwrap();
    // remove the directory behind git's back so prune has something to do
    let _ = std::fs::remove_dir_all(&wt);

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = g.git_interactive(&["worktree", "prune"]);
        let _ = tx.send(r.is_ok());
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(ok) => assert!(ok, "git worktree prune failed"),
        Err(e) => panic!("git_interactive hung (pty stderr reader blocked): {e}"),
    }
}

#[test]
fn interactive_command_failure_still_includes_stderr_after_pty_fix() {
    let (_td, g, _head) = repo_with_commit();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = g.git_interactive(&["worktree", "prune", "--bogus-option"]);
        let _ = tx.send(r.err().map(|e| format!("{e:#}")).unwrap_or_default());
    });
    let detail = rx.recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|e| panic!("git_interactive hung: {e}"));
    assert!(!detail.is_empty(), "expected a detailed failure");
    assert!(detail.contains("stderr:") || detail.contains("worktree"), "{detail}");
}
