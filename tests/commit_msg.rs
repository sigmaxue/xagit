//! The built-in COMMIT_EDITMSG buffer: the `c` menu edits the message in
//! the TUI, C-n/C-p move, C-c C-c commits, C-c C-k cancels.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

fn subjects(g: &Git) -> Vec<String> {
    g.git_string(&["log", "--format=%s"]).unwrap().lines().map(str::to_string).collect()
}

fn repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    write(&g, "f.txt", "one\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "initial"]);
    (td, g)
}

fn staged_change(g: &Git) {
    write(g, "f.txt", "two\n");
    git(g, &["add", "f.txt"]);
}

/// Type a single-line message (letters, no spaces needed in the first test).
fn type_line(app: &mut App, s: &str) {
    for ch in s.chars() {
        app.press(&ch.to_string()).unwrap();
    }
}

#[test]
fn create_edits_and_commits() {
    let (_td, g) = repo();
    staged_change(&g);
    let mut app = App::new(g.clone());
    press(&mut app, &["c", "c"]);
    assert_eq!(app.mode(), &BufferMode::Commit, "{}", app.message());
    // the buffer is pre-filled with the standard comment instructions
    assert!(app.commit_msg_text().unwrap().contains("Please enter the commit message"));
    assert_eq!(app.commit_msg_point().unwrap().0, 0, "point starts on the empty message line");
    // type the subject, a blank line and a body
    type_line(&mut app, "my subject");
    app.press("enter").unwrap();
    app.press("enter").unwrap();
    type_line(&mut app, "the body");
    // C-p / C-n move between lines
    let before = app.commit_msg_point().unwrap().0;
    app.press("C-p").unwrap();
    assert_eq!(app.commit_msg_point().unwrap().0, before - 1);
    app.press("C-n").unwrap();
    assert_eq!(app.commit_msg_point().unwrap().0, before);
    app.press("C-c").unwrap();
    app.press("C-c").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status, "{}", app.message());
    assert_eq!(subjects(&g)[0], "my subject");
    let body = g.git_string(&["log", "-1", "--format=%B"]).unwrap();
    assert!(body.contains("my subject") && body.contains("the body"), "{body}");
    assert!(!body.contains("Please enter"), "comment leaked into the message: {body}");
}

#[test]
fn cancel_does_not_commit() {
    let (_td, g) = repo();
    staged_change(&g);
    let mut app = App::new(g.clone());
    press(&mut app, &["c", "c"]);
    type_line(&mut app, "discarded");
    app.press("C-c").unwrap();
    app.press("C-k").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    assert_eq!(subjects(&g), ["initial"]);
    assert_eq!(app.message(), "Commit aborted");
}

#[test]
fn empty_message_is_rejected() {
    let (_td, g) = repo();
    staged_change(&g);
    let mut app = App::new(g.clone());
    press(&mut app, &["c", "c"]);
    app.press("C-c").unwrap();
    app.press("C-c").unwrap();
    assert_eq!(subjects(&g), ["initial"]);
    assert!(app.message().contains("empty commit message"), "{}", app.message());
}

#[test]
fn amend_prefills_head_message() {
    let (_td, g) = repo();
    staged_change(&g);
    let mut app = App::new(g.clone());
    press(&mut app, &["c", "a"]); // amend
    assert_eq!(app.mode(), &BufferMode::Commit, "{}", app.message());
    assert!(app.commit_msg_text().unwrap().starts_with("initial\n"), "{:?}", app.commit_msg_text());
    // the point is on the (empty) second line; go up and append to the subject
    app.press("C-e").unwrap();
    type_line(&mut app, " amended");
    app.press("C-c").unwrap();
    app.press("C-c").unwrap();
    assert_eq!(subjects(&g)[0], "initial amended");
}

#[test]
fn editing_keys() {
    let (_td, g) = repo();
    staged_change(&g);
    let mut app = App::new(g.clone());
    press(&mut app, &["c", "c"]);
    type_line(&mut app, "abcd");
    // C-b / C-f move one character; C-d deletes under the point
    app.press("C-a").unwrap();
    app.press("C-f").unwrap();
    app.press("C-d").unwrap(); // delete 'b' -> "acd"
    app.press("C-e").unwrap();
    app.press("backspace").unwrap(); // delete 'd' -> "ac"
    app.press("C-a").unwrap();
    type_line(&mut app, "X"); // "Xac"
    app.press("C-e").unwrap();
    type_line(&mut app, "Y"); // "XacY"
    app.press("C-c").unwrap();
    app.press("C-c").unwrap();
    assert_eq!(subjects(&g)[0], "XacY", "{}", app.message());
}

#[test]
fn reword_keeps_the_tree() {
    let (_td, g) = repo();
    let tree_before = g.git_string(&["rev-parse", "HEAD^{tree}"]).unwrap();
    let mut app = App::new(g.clone());
    press(&mut app, &["c", "w"]); // reword
    assert_eq!(app.mode(), &BufferMode::Commit, "{}", app.message());
    assert!(app.commit_msg_text().unwrap().starts_with("initial\n"));
    app.press("C-e").unwrap();
    type_line(&mut app, " reworded");
    app.press("C-c").unwrap();
    app.press("C-c").unwrap();
    assert_eq!(subjects(&g)[0], "initial reworded");
    assert_eq!(g.git_string(&["rev-parse", "HEAD^{tree}"]).unwrap(), tree_before);
    assert_eq!(read(&g, "f.txt"), "one\n");
}
