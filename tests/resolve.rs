//! The built-in conflict resolution view: `e` (ediff dwim) with unmerged
//! conflicts opens a view where a/b/d choose A(ours)/B(theirs)/both, u
//! undecides, n/p move, and C-c C-c writes the file back and stages it.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};
use xagit_lib::ui::resolve::Choice;

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

/// A repository with a real merge conflict: master and topic both changed
/// the same lines of f.txt, then `git rebase master` on topic.
fn conflict_repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    git(&g, &["config", "merge.conflictStyle", "merge"]);
    write(&g, "f.txt", "top\nbase line\nmid\nbottom\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "base"]);
    git(&g, &["checkout", "-q", "-b", "topic"]);
    write(&g, "f.txt", "top\nTOPIC line\nmid\nbottom\n");
    git(&g, &["commit", "-q", "-am", "topic"]);
    git(&g, &["checkout", "-q", "master"]);
    write(&g, "f.txt", "top\nMASTER line\nmid\nbottom\n");
    git(&g, &["commit", "-q", "-am", "master"]);
    git(&g, &["checkout", "-q", "topic"]);
    // rebase topic onto master conflicts in f.txt
    let _ = g.git_string(&["rebase", "master"]);
    (td, g)
}

fn open(app: &mut App) {
    app.press("e").unwrap();
    assert_eq!(app.mode(), &BufferMode::Resolve, "{}", app.message());
}

#[test]
fn e_opens_only_with_conflicts() {
    let (_td, g) = conflict_repo();
    assert!(!g.unmerged_files().is_empty(), "expected a conflict");
    let mut app = App::new(g.clone());
    open(&mut app);
    assert_eq!(app.resolve_choices().unwrap().len(), 1);
    // without conflicts `e` does not open the view
    git(&g, &["rebase", "--abort"]);
    let mut clean = App::new(g);
    clean.press("e").unwrap();
    assert_ne!(clean.mode(), &BufferMode::Resolve);
}

#[test]
fn choose_a_b_both_and_save() {
    let (_td, g) = conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    // conflict markers are still in the worktree file until C-c C-c
    assert!(read(&g, "f.txt").contains("<<<<<<<"));

    // a: keep A (ours = the version of the branch being rebased onto,
    // i.e. the upper side shown in the file)
    press(&mut app, &["a"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::A]);
    let out = app.resolve_result().unwrap();
    assert!(out.contains("MASTER line") && !out.contains("TOPIC line"), "a: {out:?}");

    // u then b: keep B (theirs)
    press(&mut app, &["p", "u"]);
    press(&mut app, &["b"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::B]);
    let out = app.resolve_result().unwrap();
    assert!(out.contains("TOPIC line") && !out.contains("MASTER line"), "b: {out:?}");

    // d: keep both
    press(&mut app, &["p", "d"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::Both]);
    let out = app.resolve_result().unwrap();
    assert!(out.contains("MASTER line") && out.contains("TOPIC line"), "d: {out:?}");

    // C-c C-c writes back and stages the file
    press(&mut app, &["C-c", "C-c"]);
    assert_ne!(app.mode(), &BufferMode::Resolve);
    let body = read(&g, "f.txt");
    assert!(!body.contains("<<<<<<<") && !body.contains("=======") && !body.contains(">>>>>>>"), "markers left: {body:?}");
    assert!(body.contains("MASTER line") && body.contains("TOPIC line"), "both: {body:?}");
    assert!(g.unmerged_files().is_empty(), "file should be staged/resolved: {}", app.message());
}

#[test]
fn multiple_conflicts_and_movement() {
    let (_td, g) = two_conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    assert_eq!(app.resolve_choices().unwrap().len(), 2);
    assert_eq!(app.resolve_cursor(), Some(0));
    press(&mut app, &["a"]);
    // auto-advance went to the next undecided conflict
    assert_eq!(app.resolve_cursor(), Some(1));
    press(&mut app, &["b"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::A, Choice::B]);
    // n/p move between conflicts
    press(&mut app, &["p"]);
    assert_eq!(app.resolve_cursor(), Some(0));
    press(&mut app, &["n"]);
    assert_eq!(app.resolve_cursor(), Some(1));
    press(&mut app, &["C-c", "C-c"]);
    let body = read(&g, "f.txt");
    assert!(!body.contains("<<<<<<<"), "{body:?}");
}

/// Two distant conflicting lines in the same file.
fn two_conflict_repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    write(&g, "f.txt", "1\n2\n3\n4\n5\n6\n7\n8\n9\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "base"]);
    git(&g, &["checkout", "-q", "-b", "topic"]);
    write(&g, "f.txt", "TOPIC-1\n2\n3\n4\n5\n6\n7\n8\nTOPIC-9\n");
    git(&g, &["commit", "-q", "-am", "topic"]);
    git(&g, &["checkout", "-q", "master"]);
    write(&g, "f.txt", "MASTER-1\n2\n3\n4\n5\n6\n7\n8\nMASTER-9\n");
    git(&g, &["commit", "-q", "-am", "master"]);
    git(&g, &["checkout", "-q", "topic"]);
    let _ = g.git_string(&["rebase", "master"]);
    (td, g)
}

#[test]
fn cancel_leaves_file_untouched() {
    let (_td, g) = conflict_repo();
    let before = read(&g, "f.txt");
    let mut app = App::new(g.clone());
    open(&mut app);
    press(&mut app, &["d"]);
    // declining the confirm keeps editing
    app.script_answers(&["n"]);
    press(&mut app, &["C-c", "C-k"]);
    assert_eq!(app.mode(), &BufferMode::Resolve);
    // confirming cancels without writing
    app.script_answers(&["y"]);
    press(&mut app, &["C-c", "C-k"]);
    assert_ne!(app.mode(), &BufferMode::Resolve);
    assert_eq!(read(&g, "f.txt"), before);
    assert!(!g.unmerged_files().is_empty());
}

#[test]
fn undecided_keeps_markers() {
    let (_td, g) = conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    press(&mut app, &["a", "p", "u"]); // decide then undecide
    assert_eq!(app.resolve_choices().unwrap(), [Choice::Undecided]);
    press(&mut app, &["C-c", "C-c"]);
    // markers are retained, file stays unresolved
    assert!(read(&g, "f.txt").contains("<<<<<<<"));
    assert!(!g.unmerged_files().is_empty());
    assert!(app.message().contains("undecided"), "{}", app.message());
}
