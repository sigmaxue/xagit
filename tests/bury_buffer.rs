//! magit-mode-bury-buffer (q): burying keeps the buffer around (J and
//! setting up a buffer of its mode show it again), C-u q kills it.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};

fn texts(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

fn goto(app: &mut App, text: &str) {
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-p").unwrap();
    }
    for _ in 0..n {
        if app.point_text().starts_with(text) {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no line {text:?} in {:#?}", texts(app));
}

/// file.txt changed in the worktree, two commits.
fn repo() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    write(&g, "file.txt", "hello\nworld\n");
    g.git_interactive(&["commit", "-q", "-am", "two"]).unwrap();
    write(&g, "file.txt", "hello\nworld\nmore\n");
    (td, g)
}

fn name(g: &Git, mode: &str) -> String {
    format!("{mode}: {}", g.toplevel.file_name().unwrap().to_string_lossy())
}

/// J: display the buffer NAME.
fn display(app: &mut App, name: &str) {
    app.script_answers(&[name]);
    app.press("J").unwrap();
}

#[test]
fn bury_keeps_the_buffer() {
    let (_td, g) = repo();
    let mut app = App::new(g.clone());
    press(&mut app, &["d", "u"]);
    assert!(matches!(app.mode(), BufferMode::Diff(_)));
    goto(&mut app, "@@");
    let at = app.point_text().to_string();
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    // J shows it again, as it was
    display(&mut app, &name(&g, "magit-diff"));
    assert!(matches!(app.mode(), BufferMode::Diff(_)), "{}", app.message());
    assert_eq!(app.point_text(), at);
    // below it is the status buffer again
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    // the same diff set up again reuses the buffer
    press(&mut app, &["d", "u"]);
    assert_eq!(app.point_text(), at);
    // a killed buffer is gone
    press(&mut app, &["C-u", "q"]);
    assert_eq!(app.mode(), &BufferMode::Status);
    display(&mut app, &name(&g, "magit-diff"));
    assert_eq!(app.mode(), &BufferMode::Status);
    assert_eq!(app.message(), "[No match]");
    press(&mut app, &["d", "u"]);
    assert!(app.point_text() != at, "a new buffer starts at the top");
}

#[test]
fn bury_several() {
    let (_td, g) = repo();
    let mut app = App::new(g.clone());
    press(&mut app, &["l", "l"]);
    goto(&mut app, &g.git_string(&["rev-parse", "--short", "HEAD"]).unwrap());
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { .. }));
    press(&mut app, &["q", "q"]);
    assert_eq!(app.mode(), &BufferMode::Status);
    assert!(!app.quitting());
    display(&mut app, &name(&g, "magit-revision"));
    assert!(matches!(app.mode(), BufferMode::Revision { .. }), "{}", app.message());
    display(&mut app, &name(&g, "magit-log"));
    assert!(matches!(app.mode(), BufferMode::Log { .. }), "{}", app.message());
    // q shows the buffer below, the revision buffer
    app.press("q").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { .. }));
    press(&mut app, &["q", "q"]);
    assert_eq!(app.mode(), &BufferMode::Status);
    // the status buffer stays; C-x C-c quits
    app.press("q").unwrap();
    assert!(!app.quitting());
    assert_eq!(app.mode(), &BufferMode::Status);
    assert_eq!(app.message(), "Type C-x C-c to quit xagit");
    press(&mut app, &["C-x", "C-g"]);
    assert!(!app.quitting());
    press(&mut app, &["C-x", "C-c"]);
    assert!(app.quitting());
}

#[test]
fn blobs_are_killed() {
    let (_td, g) = repo();
    let mut app = App::new(g.clone());
    goto(&mut app, "modified   file.txt");
    app.press("RET").unwrap();
    app.press("p").unwrap();
    assert!(app.header_line().contains("file.txt"), "{}", app.header_line());
    let blob = app.header_line().to_string();
    // magit-blob-mode-map: q is magit-kill-this-buffer
    app.press("q").unwrap();
    display(&mut app, &blob);
    assert!(!matches!(app.mode(), BufferMode::File { rev, .. } if rev != "{worktree}"), "{:?}", app.mode());
}
