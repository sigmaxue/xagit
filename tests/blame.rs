//! File and blob buffers (magit-find-file, magit-blob-mode) and
//! magit-blame: the styles, chunk navigation, recursive blaming, blaming
//! removals and the blame transient.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};
use xagit_lib::ui::blame::SEPARATOR;

fn commit(g: &Git, content: &str, msg: &str) {
    write(g, "f.txt", content);
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", msg]).unwrap();
}

/// f.txt: "one" a b c, "two" b -> B, "three" adds d; the worktree adds e.
fn blame_repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    g.git_interactive(&["config", "user.name", "Ann Author"]).unwrap();
    commit(&g, "a\nb\nc\n", "one");
    commit(&g, "a\nB\nc\n", "two");
    commit(&g, "a\nB\nc\nd\n", "three");
    write(&g, "f.txt", "a\nB\nc\nd\ne\n");
    (td, g)
}

fn texts(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

/// Visit the worktree file from the status buffer.
fn visit_file(app: &mut App) {
    for _ in 0..app.visible_lines().len() {
        if app.point_text() == "modified   f.txt" {
            app.press("RET").unwrap();
            assert_eq!(app.header_line(), "f.txt");
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no unstaged f.txt");
}

fn short(g: &Git, rev: &str) -> String {
    g.git_string(&["rev-parse", "--short", rev]).unwrap()
}

#[test]
fn blame_headings() {
    let (_td, g) = blame_repo();
    let mut app = App::new(g.clone());
    visit_file(&mut app);
    assert_eq!(texts(&app), ["a", "B", "c", "d", "e"]);
    app.press("b").unwrap();
    let t = texts(&app);
    assert_eq!(t.len(), 10, "{t:#?}");
    for (i, summary) in [(0, "one"), (2, "two"), (4, "one"), (6, "three")] {
        assert!(t[i].starts_with("Ann Author           "), "{t:#?}");
        assert!(t[i].ends_with(&format!(" {summary}")), "{t:#?}");
    }
    assert_eq!(t[8], "Not Yet Committed");
    assert_eq!([&t[1], &t[3], &t[5], &t[7], &t[9]], ["a", "B", "c", "d", "e"]);
    assert_eq!(app.file_line(), Some(1));
    // the chunks
    app.press("n").unwrap();
    assert_eq!(app.file_line(), Some(2));
    assert_eq!(app.point_text(), t[2]);
    press(&mut app, &["p", "N"]);
    assert_eq!(app.file_line(), Some(3), "the next chunk of \"one\"");
    app.press("N").unwrap();
    assert_eq!(app.message(), "No more chunks from same commit");
    press(&mut app, &["n", "n"]);
    assert_eq!(app.file_line(), Some(5));
    app.press("n").unwrap();
    assert_eq!(app.message(), "No more chunks");
    // RET shows the commit of the chunk
    press(&mut app, &["p", "p", "p"]);
    assert_eq!(app.file_line(), Some(2));
    app.press("M-w").unwrap();
    assert_eq!(app.message(), g.git_string(&["rev-parse", "HEAD~"]).unwrap());
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { .. }), "{:?}", app.mode());
    app.press("q").unwrap();
    assert_eq!(texts(&app).len(), 10, "still blaming");
    // q turns blaming off, then buries the buffer
    app.press("q").unwrap();
    assert_eq!(texts(&app), ["a", "B", "c", "d", "e"]);
    assert_eq!(app.file_line(), Some(2));
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
}

#[test]
fn blame_styles() {
    let (_td, g) = blame_repo();
    let mut app = App::new(g);
    visit_file(&mut app);
    press(&mut app, &["C-n", "b"]);
    app.press("c").unwrap();
    // highlight: no headings
    assert_eq!(texts(&app), ["a", "B", "c", "d", "e"]);
    assert_eq!(app.file_line(), Some(2));
    // lines: a separator before each chunk, the summary in the echo area
    app.press("c").unwrap();
    let t = texts(&app);
    assert_eq!(t, [SEPARATOR, "a", SEPARATOR, "B", SEPARATOR, "c", SEPARATOR, "d", SEPARATOR, "e"]);
    app.press("C-n").unwrap();
    assert_eq!(app.message(), "\"one\"");
    // headings again, from the blame transient (where c is transient)
    press(&mut app, &["B", "c"]);
    assert_eq!(app.current_menu(), Some("blame-menu"));
    assert_eq!(texts(&app).len(), 10);
    // q: Quit blaming
    app.press("q").unwrap();
    assert_eq!(app.current_menu(), None);
    assert_eq!(texts(&app), ["a", "B", "c", "d", "e"]);
}

#[test]
fn recursive_blame() {
    let (_td, g) = blame_repo();
    let mut app = App::new(g.clone());
    visit_file(&mut app);
    app.press("b").unwrap();
    app.press("b").unwrap();
    assert_eq!(app.message(), "Chunk has no further history");
    // B was changed by "two": blame the file as it was before
    app.press("n").unwrap();
    app.press("b").unwrap();
    let parent = g.git_string(&["rev-parse", "HEAD~2"]).unwrap();
    assert_eq!(app.header_line(), format!("f.txt.~{parent}~"));
    assert_eq!(app.file_line(), Some(2));
    assert_eq!(app.point_text(), "b");
    let t = texts(&app);
    assert_eq!(t[1..], ["a", "b", "c"], "one chunk of \"one\": {t:#?}");
    // q kills the buffer of a recursive blame
    app.press("q").unwrap();
    assert_eq!(app.header_line(), "f.txt");
    assert_eq!(texts(&app).len(), 10);
}

#[test]
fn blobs_and_reverse_blame() {
    let (_td, g) = blame_repo();
    let mut app = App::new(g.clone());
    visit_file(&mut app);
    app.press("r").unwrap();
    assert_eq!(app.message(), "Only blob buffers can be blamed in reverse");
    // p: HEAD, then the blobs that changed the file
    app.press("p").unwrap();
    assert_eq!(app.header_line(), format!("f.txt.~{}~", short(&g, "HEAD")));
    assert!(app.message().starts_with("three ("), "{}", app.message());
    press(&mut app, &["p", "p"]);
    assert_eq!(app.header_line(), format!("f.txt.~{}~", short(&g, "HEAD~2")));
    assert_eq!(texts(&app), ["a", "b", "c"]);
    app.press("p").unwrap();
    assert_eq!(app.message(), "You have reached the beginning of time");
    // the commits that removed the lines
    press(&mut app, &["C-n", "r"]);
    let t = texts(&app);
    assert_eq!(t.len(), 6, "{t:#?}");
    assert!(t[2].ends_with(" two"), "b was removed by \"two\": {t:#?}");
    assert_eq!(app.file_line(), Some(2));
    // the last commits that still have them
    app.press("f").unwrap();
    let t = texts(&app);
    assert!(t[0].ends_with(" three"), "{t:#?}");
    assert!(t[2].ends_with(" one"), "{t:#?}");
    // n: the next blobs, up to the worktree file
    press(&mut app, &["q", "n"]);
    assert_eq!(app.header_line(), format!("f.txt.~{}~", short(&g, "HEAD~")));
    press(&mut app, &["n", "n"]);
    assert_eq!(app.header_line(), "f.txt");
    app.press("n").unwrap();
    assert_eq!(app.message(), "You have reached the end of time");
}

#[test]
fn blame_transient() {
    let (_td, g) = blame_repo();
    write(&g, "f.txt", "a\nB\n  c\nd\n");
    let mut app = App::new(g);
    visit_file(&mut app);
    app.press("B").unwrap();
    assert_eq!(app.active_switches(), ["-w"]);
    app.script_answers(&[""]);
    press(&mut app, &["-", "M"]);
    assert_eq!(app.active_switches(), ["-M", "-w"]);
    app.press("b").unwrap();
    assert_eq!(app.current_menu(), None);
    let t = texts(&app);
    // -w: the re-indented line is still from "one"
    assert!(t[4].ends_with(" one"), "{t:#?}");
    // without -w it is not committed
    press(&mut app, &["B", "-", "w", "b"]);
    assert_eq!(app.message(), "Chunk has no further history");
    press(&mut app, &["q", "B", "-", "w", "b"]);
    let t = texts(&app);
    assert_eq!(t[4], "Not Yet Committed", "{t:#?}");
    // file buffers have no Magit bindings
    app.press("s").unwrap();
    assert_eq!(app.message(), "s is undefined");
}

#[test]
fn blame_echo() {
    let (_td, g) = blame_repo();
    let mut app = App::new(g.clone());
    visit_file(&mut app);
    // the lines style, the summary of the chunk at point echoed
    press(&mut app, &["C-n", "B", "m"]);
    assert_eq!(app.current_menu(), None);
    let t = texts(&app);
    assert_eq!(t, [SEPARATOR, "a", SEPARATOR, "B", SEPARATOR, "c", SEPARATOR, "d", SEPARATOR, "e"]);
    assert_eq!(app.file_line(), Some(2));
    assert_eq!(app.message(), "\"two\"");
    // only when entering another chunk
    app.press("C-g").unwrap();
    press(&mut app, &["C-n", "C-n"]);
    assert_eq!(app.file_line(), Some(3));
    assert_eq!(app.message(), "\"one\"");
    app.press("C-g").unwrap();
    app.press("C-n").unwrap();
    assert_eq!(app.point_text(), SEPARATOR);
    assert_eq!(app.message(), "\"three\"", "the separator is the start of the chunk");
    press(&mut app, &["C-g", "C-n"]);
    assert_eq!(app.point_text(), "d");
    assert_eq!(app.message(), "Quit", "the same chunk");
    // without magit-blame-read-only-mode
    app.press("c").unwrap();
    assert_eq!(app.message(), "c is undefined");
    // m only while not blaming or read-only; c cycles from the transient
    app.press("B").unwrap();
    let keys = app.menu_keys();
    assert!(!keys.contains(&"m".to_string()) && keys.contains(&"q".to_string()), "{keys:?}");
    app.press("c").unwrap();
    assert_eq!(texts(&app).len(), 10, "headings");
    app.press("C-g").unwrap();
    press(&mut app, &["C-c", "M-g"]);
    assert!(!app.menu_keys().contains(&"m".to_string()));
    app.press("C-g").unwrap();
    // C-c C-q: magit-blame-quit
    press(&mut app, &["C-c", "C-q"]);
    assert_eq!(texts(&app), ["a", "B", "c", "d", "e"]);
    // while blaming read-only: turn that off, in the lines style
    press(&mut app, &["b", "B", "m"]);
    assert_eq!(texts(&app)[0], SEPARATOR);
    app.press("c").unwrap();
    assert_eq!(app.message(), "c is undefined");
    press(&mut app, &["C-c", "C-q"]);
    // not in blob buffers
    app.press("p").unwrap();
    assert!(app.header_line().starts_with("f.txt.~"));
    app.press("B").unwrap();
    assert!(!app.menu_keys().contains(&"m".to_string()));
    app.press("C-g").unwrap();
    press(&mut app, &["C-c", "M-g"]);
    let keys = app.menu_keys();
    assert!(!keys.contains(&"m".to_string()) && keys.contains(&"r".to_string()), "{keys:?}");
}

#[test]
fn blame_from_file_dispatch() {
    let (_td, g) = blame_repo();
    write(&g, "f.txt", "a\nB\n  c\nd\n");
    let mut app = App::new(g);
    visit_file(&mut app);
    // r and f only in blob buffers
    press(&mut app, &["C-c", "M-g"]);
    let keys = app.menu_keys();
    assert!(!keys.contains(&"r".to_string()) && keys.contains(&"m".to_string()), "{keys:?}");
    // magit-blame-arguments: the value of magit-blame (-w)
    app.press("b").unwrap();
    let t = texts(&app);
    assert!(t[4].ends_with(" one"), "{t:#?}");
}
