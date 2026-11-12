//! Rebase conflicts: like Magit's `magit-run-git-sequencer`, a rebase that
//! stops at a commit is a sequence in progress, not a failed command.

mod common;

use common::*;
use xagit_lib::prelude::Git;
use xagit_lib::ui::app::App;

fn ok(g: &Git, args: &[&str]) {
    let out = std::process::Command::new("git")
        .current_dir(&g.toplevel)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// master -> b edits file.txt; a forks from master and edits the same line,
/// so rebasing a onto b stops with a conflict.
fn conflicting_branches() -> (tempfile::TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    ok(&g, &["checkout", "-q", "-b", "b"]);
    write(&g, "file.txt", "b\n");
    ok(&g, &["commit", "-qam", "b work"]);
    ok(&g, &["checkout", "-q", "-b", "a", "master"]);
    write(&g, "file.txt", "a\n");
    ok(&g, &["commit", "-qam", "a work"]);
    (td, g)
}

/// The same two branches without overlapping changes: the rebase applies
/// cleanly and finishes.
fn disjoint_branches() -> (tempfile::TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    ok(&g, &["checkout", "-q", "-b", "b"]);
    write(&g, "b.txt", "b\n");
    ok(&g, &["add", "b.txt"]);
    ok(&g, &["commit", "-qm", "b work"]);
    ok(&g, &["checkout", "-q", "-b", "a", "master"]);
    write(&g, "a.txt", "a\n");
    ok(&g, &["add", "a.txt"]);
    ok(&g, &["commit", "-qm", "a work"]);
    (td, g)
}

fn rebase_onto_b(app: &mut App) {
    app.script_answers(&["b"]);
    press(app, &["r", "e"]);
}

fn status_lines(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

/// master and other both edit file.txt; the merge is not run yet.
fn mergeable_branches() -> (tempfile::TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    ok(&g, &["checkout", "-q", "-b", "other"]);
    write(&g, "file.txt", "other\n");
    ok(&g, &["commit", "-qam", "other work"]);
    ok(&g, &["checkout", "-q", "master"]);
    write(&g, "file.txt", "main\n");
    ok(&g, &["commit", "-qam", "main work"]);
    (td, g)
}

#[test]
fn a_clean_rebase_reports_success() {
    let (_td, g) = disjoint_branches();
    let mut app = App::new(g.clone());
    rebase_onto_b(&mut app);
    assert!(!g.rebase_in_progress(), "{}", app.message());
    assert_eq!(app.message(), "rebased");
    assert_eq!(g.current_branch().as_deref(), Some("a"));
    assert_eq!(g.git_string(&["log", "-1", "--format=%s"]).unwrap(), "a work");
}

#[test]
fn a_conflicting_rebase_stops_the_sequence_instead_of_failing() {
    let (_td, g) = conflicting_branches();
    let mut app = App::new(g.clone());
    rebase_onto_b(&mut app);

    assert!(g.rebase_in_progress(), "{}", app.message());
    let msg = app.message();
    assert!(msg.starts_with("rebase stopped"), "{msg}");
    assert!(!msg.contains("failed"), "{msg}");
    assert!(msg.contains("r r continue"), "{msg}");
    assert!(!msg.contains("details"), "the status buffer stays in front: {msg}");
    let lines = status_lines(&app);
    assert!(lines.iter().any(|l| l == "Rebasing a onto b"), "{lines:?}");
    press(&mut app, &["r"]);
    assert_eq!(app.menu_keys(), ["r", "s", "e", "a"]);
}

#[test]
fn continuing_with_unresolved_conflicts_is_refused() {
    let (_td, g) = conflicting_branches();
    let mut app = App::new(g.clone());
    rebase_onto_b(&mut app);
    press(&mut app, &["r", "r"]);
    assert!(g.rebase_in_progress(), "{}", app.message());
    // magit-rebase-continue refuses while conflicts are unstaged; a
    // stopped sequence is never reported as a failed command.
    assert_eq!(app.message(), "Cannot continue rebase with unstaged changes");
    assert!(!app.message().contains("failed"), "{}", app.message());
}

#[test]
fn resolving_the_conflict_and_continuing_finishes_the_rebase() {
    let (_td, g) = conflicting_branches();
    let mut app = App::new(g.clone());
    rebase_onto_b(&mut app);
    write(&g, "file.txt", "resolved\n");
    ok(&g, &["add", "file.txt"]);
    press(&mut app, &["r", "r"]);
    assert!(!g.rebase_in_progress(), "{}", app.message());
    assert!(app.message().starts_with("rebase continued"), "{}", app.message());
    assert_eq!(g.git_string(&["log", "-1", "--format=%s"]).unwrap(), "a work");
    assert_eq!(read(&g, "file.txt"), "resolved\n");
}

#[test]
fn aborting_the_rebase_restores_the_branch() {
    let (_td, g) = conflicting_branches();
    let mut app = App::new(g.clone());
    rebase_onto_b(&mut app);
    app.script_answers(&["y"]);
    press(&mut app, &["r", "a"]);
    assert!(!g.rebase_in_progress(), "{}", app.message());
    assert_eq!(g.current_branch().as_deref(), Some("a"));
    assert_eq!(read(&g, "file.txt"), "a\n");
}

#[test]
fn merge_conflicts_are_reported_as_a_stopped_state() {
    let (_td, g) = mergeable_branches();
    let mut app = App::new(g.clone());
    app.script_answers(&["other"]);
    press(&mut app, &["m", "m"]);
    assert!(g.merge_in_progress(), "{}", app.message());
    assert!(!g.unmerged_files().is_empty(), "{}", app.message());
    assert!(app.message().starts_with("merge stopped"), "{}", app.message());
    assert!(app.message().contains("m m commit"), "{}", app.message());
    assert!(!app.message().contains("failed"), "{}", app.message());
    assert!(!app.message().contains("details"), "{}", app.message());
    // the built-in resolution view and line-wise refine still work
    press(&mut app, &["e", "*"]);
    assert!(app.resolve_refine() == Some(true), "{}", app.message());
}

#[test]
fn cherry_pick_conflicts_are_reported_as_a_stopped_state() {
    let (_td, g) = mergeable_branches();
    let mut app = App::new(g.clone());
    app.script_answers(&["other"]);
    press(&mut app, &["A", "A"]);
    assert!(g.cherry_in_progress(), "{}", app.message());
    assert!(app.message().starts_with("cherry-pick stopped"), "{}", app.message());
    assert!(app.message().contains("A A continue"), "{}", app.message());
    assert!(!app.message().contains("failed"), "{}", app.message());
}

#[test]
fn stash_conflicts_are_not_silent() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "stashed\n");
    ok(&g, &["stash", "push", "-m", "wip"]);
    write(&g, "file.txt", "current\n");
    ok(&g, &["commit", "-qam", "current"]);
    let mut app = App::new(g.clone());
    app.script_answers(&["stash@{0}"]);
    press(&mut app, &["z", "p"]);
    assert!(!g.unmerged_files().is_empty(), "{}", app.message());
    assert!(app.message().contains("conflicts left in the worktree"), "{}", app.message());
    assert!(!app.message().contains("failed"), "{}", app.message());
}

#[test]
fn rebase_progress_is_not_a_cherry_or_revert_sequence() {
    // sequencer-continue used the over-broad sequencer_in_progress(),
    // which includes a rebase; it must use pick_or_revert_in_progress().
    let (_td, g) = conflicting_branches();
    let mut app = App::new(g.clone());
    rebase_onto_b(&mut app);
    assert!(g.rebase_in_progress(), "{}", app.message());
    assert!(g.sequencer_in_progress());
    assert!(!g.pick_or_revert_in_progress());
}
