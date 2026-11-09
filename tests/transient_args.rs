//! Transient options (transient-option): values read when the infix is
//! set, shown in the popup, and passed to git by the suffix commands.

mod common;

use common::*;
use ngit_lib::prelude::*;
use ngit_lib::ui::app::App;

fn press_all(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

/// master: initial, then a merge of "side" (adds g.txt).
fn merged_repo() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    g.git_interactive(&["checkout", "-q", "-b", "side"]).unwrap();
    write(&g, "g.txt", "g\n");
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", "two"]).unwrap();
    g.git_interactive(&["checkout", "-q", "master"]).unwrap();
    write(&g, "file.txt", "changed\n");
    g.git_interactive(&["commit", "-q", "-am", "three"]).unwrap();
    g.git_interactive(&["merge", "-q", "--no-edit", "side"]).unwrap();
    (td, g)
}

#[test]
fn commit_reuse_message() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "new\n");
    g.git_interactive(&["add", "."]).unwrap();
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    app.script_answers(&["HEAD"]);
    press_all(&mut app, &["-", "C"]);
    assert_eq!(app.active_switches(), ["--reuse-message=HEAD"]);
    app.press("c").unwrap();
    assert_eq!(g.git_string(&["log", "--format=%s"]).unwrap().trim(), "initial\ninitial");
}

#[test]
fn merge_strategy() {
    let (_td, g, _) = repo_with_commit();
    g.git_interactive(&["checkout", "-q", "-b", "side"]).unwrap();
    write(&g, "g.txt", "g\n");
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", "two"]).unwrap();
    g.git_interactive(&["checkout", "-q", "master"]).unwrap();
    write(&g, "file.txt", "changed\n");
    g.git_interactive(&["commit", "-q", "-am", "three"]).unwrap();
    let mut app = App::new(g.clone());
    app.press("m").unwrap();
    // a choice that is not offered changes nothing
    app.script_answers(&["fast"]);
    press_all(&mut app, &["-", "s"]);
    assert_eq!(app.message(), "fast is not one of resolve, recursive, octopus, ours, subtree");
    assert!(app.active_switches().is_empty());
    app.script_answers(&["ou"]);
    press_all(&mut app, &["-", "s"]);
    assert_eq!(app.active_switches(), ["--strategy=ours"]);
    app.script_answers(&["side"]);
    app.press("n").unwrap();
    // merge-nocommit: --no-ff and the strategy
    assert!(g.gitdir.join("MERGE_HEAD").exists());
    assert!(!exists(&g, "g.txt"), "the ours strategy keeps our tree");
}

#[test]
fn revert_merge_relative_to_mainline() {
    let (_td, g) = merged_repo();
    let merge = g.git_string(&["rev-parse", "--short", "HEAD"]).unwrap();
    let mut app = App::new(g.clone());
    app.press("V").unwrap();
    app.script_answers(&["0"]);
    press_all(&mut app, &["-", "m"]);
    assert_eq!(app.message(), "Please enter a positive integer");
    app.script_answers(&["1"]);
    press_all(&mut app, &["-", "m"]);
    assert_eq!(app.active_switches(), ["--mainline=1"]);
    app.script_answers(&[merge.as_str()]);
    app.press("v").unwrap();
    assert!(!exists(&g, "g.txt"), "reverting the merge relative to master drops side's file");
}

#[test]
fn annotated_tag() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    app.press("t").unwrap();
    press_all(&mut app, &["-", "a"]);
    app.script_answers(&["v2", "HEAD", "release two"]);
    app.press("t").unwrap();
    assert_eq!(g.git_string(&["cat-file", "-t", "v2"]).unwrap().trim(), "tag");
    assert_eq!(g.git_string(&["tag", "-l", "--format=%(contents:subject)", "v2"]).unwrap().trim(), "release two");
}
