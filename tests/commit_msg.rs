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

#[test]
fn block_movement_and_the_point_position() {
    let (_td, g) = repo();
    staged_change(&g);
    let mut app = App::new(g.clone());
    press(&mut app, &["c", "c"]);
    assert_eq!(app.mode(), &BufferMode::Commit, "{}", app.message());
    // line:column/total lines, starting on the empty message line
    assert!(app.commit_msg_position().unwrap().starts_with("1:1/"), "{}", app.commit_msg_position().unwrap());
    type_line(&mut app, "subject");
    assert!(app.commit_msg_position().unwrap().starts_with("1:8/"), "{}", app.commit_msg_position().unwrap());
    // the instructions are not a non-comment block: M-n stays in the message
    app.press("M-n").unwrap();
    assert_eq!(app.commit_msg_point().unwrap().0, 0);
    // walk into the comment lines, then M-p returns to the message block
    press(&mut app, &["C-n", "C-n"]);
    assert_eq!(app.commit_msg_point().unwrap().0, 2);
    app.press("M-p").unwrap();
    assert_eq!(app.commit_msg_point(), Some((0, 7)));
    assert!(app.commit_msg_position().unwrap().starts_with("1:8/"), "{}", app.commit_msg_position().unwrap());
    press(&mut app, &["C-c", "C-c"]);
    assert_eq!(subjects(&g)[0], "subject");
}

#[test]
fn c_k_kills_the_line_and_m_k_the_block() {
    let (_td, g) = repo();
    staged_change(&g);
    let mut app = App::new(g.clone());
    press(&mut app, &["c", "c"]);
    type_line(&mut app, "subject");
    press(&mut app, &["enter", "enter"]);
    type_line(&mut app, "body");
    // point in the middle of the subject: C-k still kills the whole line
    press(&mut app, &["C-p", "C-p", "C-a", "C-f", "C-f"]);
    assert_eq!(app.commit_msg_point(), Some((0, 2)));
    app.press("C-k").unwrap();
    let text = app.commit_msg_text().unwrap();
    assert!(!text.contains("subject") && text.contains("body"), "{text}");
    assert_eq!(app.commit_msg_point(), Some((0, 0)));
    // M-k kills the rest of the message block; git's comments stay
    app.press("M-k").unwrap();
    let text = app.commit_msg_text().unwrap();
    assert!(!text.contains("body") && text.contains("Please enter"), "{text}");
    assert_eq!(app.commit_msg_point(), Some((0, 0)));
}

#[test]
fn undo_restores_typing_and_kills() {
    let (_td, g) = repo();
    staged_change(&g);
    let mut app = App::new(g.clone());
    press(&mut app, &["c", "c"]);
    type_line(&mut app, "subject");
    app.press("C-k").unwrap();
    assert!(!app.commit_msg_text().unwrap().contains("subject"));
    // C-_ puts the line back, with point where it was
    app.press("C-_").unwrap();
    assert_eq!(app.message(), "Undo");
    assert!(app.commit_msg_text().unwrap().contains("subject"));
    assert_eq!(app.commit_msg_point(), Some((0, 7)));
    // C-/ undoes the whole run of typed characters in one step
    app.press("C-/").unwrap();
    assert!(!app.commit_msg_text().unwrap().contains("subject"));
    assert_eq!(app.commit_msg_point(), Some((0, 0)));
    app.press("C-/").unwrap();
    assert_eq!(app.message(), "No further undo information");
    // a killed block is undoable too
    type_line(&mut app, "again");
    app.press("M-k").unwrap();
    assert!(!app.commit_msg_text().unwrap().contains("again"), "{}", app.commit_msg_text().unwrap());
    app.press("C-/").unwrap();
    assert!(app.commit_msg_text().unwrap().contains("again"), "{}", app.commit_msg_text().unwrap());
}
