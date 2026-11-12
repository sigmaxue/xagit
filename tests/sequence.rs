//! magit-sequence / magit-commit: prefix default values, commits selected
//! by the region, magit--cherry-pick's merge handling and commits whose
//! message is written in the user's editor.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;

fn commit_file(g: &Git, file: &str, msg: &str) {
    write(g, file, &format!("{msg}\n"));
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", msg]).unwrap();
}

fn subjects(g: &Git) -> Vec<String> {
    g.git_string(&["log", "--format=%s"]).unwrap().lines().map(String::from).collect()
}

fn goto(app: &mut App, suffix: &str) {
    for _ in 0..app.visible_lines().len() {
        if app.point_text().ends_with(suffix) {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no line ending with {suffix:?}");
}

/// master: initial, a, b; "other" at initial (checked out).
fn picks_repo() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    g.git_interactive(&["branch", "other"]).unwrap();
    commit_file(&g, "a.txt", "a");
    commit_file(&g, "b.txt", "b");
    g.git_interactive(&["checkout", "-q", "other"]).unwrap();
    (td, g)
}

fn log_master(app: &mut App) {
    app.script_answers(&["master"]);
    app.press("l").unwrap();
    app.press("o").unwrap();
    assert_eq!(app.header_line(), "Commits in master");
}

#[test]
fn prefix_values() {
    let (_td, g, _) = repo_with_commit();
    for (key, value) in [
        ("c", &["--verbose"][..]),
        ("A", &["--ff"][..]),
        ("V", &["--edit"][..]),
        ("r", &["--autostash"][..]),
        ("M", &["-f"][..]),
        ("w", &["--3way"][..]),
    ] {
        let mut app = App::new(g.clone());
        app.press(key).unwrap();
        assert_eq!(app.active_switches(), value, "{key}");
    }
}

#[test]
fn cherry_pick_region_oldest_first() {
    let (_td, g) = picks_repo();
    let mut app = App::new(g.clone());
    log_master(&mut app);
    goto(&mut app, " b");
    app.press("C-SPC").unwrap();
    goto(&mut app, " a");
    app.press("A").unwrap();
    app.press("A").unwrap();
    assert_eq!(subjects(&g), ["b", "a", "initial"]);
}

#[test]
fn cherry_pick_defaults_to_commit_at_point() {
    let (_td, g) = picks_repo();
    let mut app = App::new(g.clone());
    log_master(&mut app);
    goto(&mut app, " a");
    app.press("A").unwrap();
    app.script_answers(&[""]);
    app.press("A").unwrap();
    assert_eq!(subjects(&g), ["a", "initial"]);
}

#[test]
fn cherry_apply_does_not_commit() {
    let (_td, g) = picks_repo();
    let mut app = App::new(g.clone());
    app.press("A").unwrap();
    app.script_answers(&["master~1..master"]);
    app.press("a").unwrap();
    assert_eq!(subjects(&g), ["initial"]);
    // magit--cherry-pick splits "A..B" into the commits A and B
    assert_eq!(read(&g, "a.txt"), "a\n");
    assert_eq!(read(&g, "b.txt"), "b\n");
}

#[test]
fn revert_region() {
    let (_td, g) = picks_repo();
    g.git_interactive(&["checkout", "-q", "master"]).unwrap();
    let mut app = App::new(g.clone());
    log_master(&mut app);
    goto(&mut app, " b");
    app.press("C-SPC").unwrap();
    goto(&mut app, " a");
    app.press("V").unwrap();
    app.press("V").unwrap();
    assert_eq!(subjects(&g), ["Revert \"a\"", "Revert \"b\"", "b", "a", "initial"]);
}

#[test]
fn merge_and_non_merge_commits() {
    let (_td, g) = picks_repo();
    g.git_interactive(&["checkout", "-q", "master"]).unwrap();
    g.git_interactive(&["checkout", "-q", "-b", "side", "HEAD~2"]).unwrap();
    commit_file(&g, "s.txt", "s");
    g.git_interactive(&["checkout", "-q", "master"]).unwrap();
    g.git_interactive(&["merge", "-q", "--no-edit", "side"]).unwrap();
    let mut app = App::new(g.clone());
    app.press("V").unwrap();
    app.script_answers(&["HEAD~1..HEAD"]);
    app.press("V").unwrap();
    assert_eq!(app.message(), "Cannot revert merge and non-merge commits at once");
    // a merge without --mainline reads the parent
    app.press("V").unwrap();
    app.script_answers(&["HEAD", "x", "1"]);
    app.press("V").unwrap();
    assert_eq!(subjects(&g)[0], "Revert \"Merge branch 'side'\"");
    assert!(!exists(&g, "s.txt"));
}

#[test]
fn commit_create_uses_the_builtin_buffer() {
    let (_td, g, _) = repo_with_commit();
    // an external editor is configured, but xagit edits the message itself
    g.git_interactive(&["config", "core.editor", "f() { echo edited > \"$1\"; }; f"]).unwrap();
    write(&g, "file.txt", "new\n");
    g.git_interactive(&["add", "."]).unwrap();
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    app.press("c").unwrap();
    assert_eq!(app.mode(), &xagit_lib::ui::app::BufferMode::Commit);
    for ch in "edited".chars() {
        app.press(&ch.to_string()).unwrap();
    }
    app.press("C-c").unwrap();
    app.press("C-c").unwrap();
    assert_eq!(subjects(&g), ["edited", "initial"]);
}
