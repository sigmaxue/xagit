//! magit-file-dispatch (C-c M-g) in file and blob buffers and elsewhere:
//! the file commands (stage, unstage, untrack, rename, delete, checkout),
//! diff / log / trace of the visited file, navigation and editing the
//! commit of a line.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};

fn commit(g: &Git, content: &str, msg: &str) {
    write(g, "f.txt", content);
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", msg]).unwrap();
}

/// f.txt: "one" a b c, "two" b -> B, "three" adds d; the worktree
/// changes c to C and adds e.
fn file_repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    commit(&g, "a\nb\nc\n", "one");
    commit(&g, "a\nB\nc\n", "two");
    commit(&g, "a\nB\nc\nd\n", "three");
    write(&g, "f.txt", "a\nB\nC\nd\ne\n");
    (td, g)
}

fn goto(app: &mut App, text: &str) {
    for _ in 0..app.visible_lines().len() {
        app.press("C-p").unwrap();
    }
    for _ in 0..app.visible_lines().len() {
        if app.point_text() == text {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no line {text:?}");
}

/// Visit the worktree file f.txt from the status buffer, at LINE.
fn visit(app: &mut App, line: usize) {
    goto(app, "modified   f.txt");
    app.press("RET").unwrap();
    assert_eq!(app.header_line(), "f.txt");
    for _ in 1..line {
        app.press("C-n").unwrap();
    }
    assert_eq!(app.file_line(), Some(line));
}

fn dispatch(app: &mut App, keys: &[&str]) {
    press(app, &["C-c", "M-g"]);
    assert_eq!(app.current_menu(), Some("file-dispatch"));
    press(app, keys);
}

fn staged(g: &Git) -> Vec<String> {
    g.git_lines(&["diff", "--cached", "--name-only", "-z"])
}

#[test]
fn menus() {
    let (_td, g) = file_repo();
    let mut app = App::new(g);
    press(&mut app, &["C-c", "M-g"]);
    assert_eq!(app.menu_keys(), ["s", "u", "x", "r", "k", "c", "G"], "no g in the status buffer");
    app.press("C-g").unwrap();
    visit(&mut app, 1);
    press(&mut app, &["C-c", "M-g"]);
    let keys = app.menu_keys();
    for k in [", x", ", r", ", k", ", c", "d", "l", "t", "B", "b", "p", "n", "v", "V", "g", "G", "c", "e"] {
        assert!(keys.contains(&k.to_string()), "{k} in {keys:?}");
    }
    assert!(!keys.contains(&"q".to_string()), "q only while blaming");
    app.press("q").unwrap();
    assert_eq!(app.current_menu(), None);
    // magit-blame: r and f only in blob buffers, m only in file buffers,
    // q and c while blaming
    app.press("B").unwrap();
    assert_eq!(app.menu_keys(), ["-w", "-r", "-P", "-M", "-C", "b", "m"]);
    app.press("b").unwrap();
    press(&mut app, &["C-c", "M-g"]);
    assert!(app.menu_keys().contains(&"q".to_string()));
    app.press("q").unwrap();
    assert_eq!(app.current_menu(), None);
    assert_eq!(app.visible_lines().len(), 5, "blaming was turned off");
    app.press("p").unwrap();
    app.press("B").unwrap();
    assert_eq!(app.menu_keys(), ["-w", "-r", "-P", "-M", "-C", "b", "r", "f"]);
}

#[test]
fn stage_and_unstage() {
    let (_td, g) = file_repo();
    write(&g, "g.txt", "new\n");
    let mut app = App::new(g.clone());
    visit(&mut app, 1);
    dispatch(&mut app, &["s"]);
    assert_eq!(staged(&g), ["f.txt"]);
    dispatch(&mut app, &["u"]);
    assert!(staged(&g).is_empty());
    // a blob is not a visited file
    app.press("p").unwrap();
    dispatch(&mut app, &["s"]);
    assert_eq!(app.message(), "Not visiting a file");
    // elsewhere, magit-stage-files / -unstage-files read files
    press(&mut app, &["q", "q"]);
    assert_eq!(app.mode(), &BufferMode::Status);
    app.script_answers(&["f.txt, g.txt"]);
    dispatch(&mut app, &["s"]);
    assert_eq!(staged(&g), ["f.txt", "g.txt"]);
    app.script_answers(&["g.txt"]);
    dispatch(&mut app, &["u"]);
    assert_eq!(staged(&g), ["f.txt"]);
    app.script_answers(&["nope.txt"]);
    dispatch(&mut app, &["s"]);
    assert_eq!(app.message(), "no match");
}

#[test]
fn rename_delete_checkout_untrack() {
    let (_td, g) = file_repo();
    write(&g, "g.txt", "new\n");
    let mut app = App::new(g.clone());
    visit(&mut app, 2);
    app.script_answers(&["", "sub/h.txt"]);
    dispatch(&mut app, &[",", "r"]);
    assert!(app.message().starts_with("Destination directory "), "{}", app.message());
    assert!(app.message().ends_with("/sub does not exist"), "{}", app.message());
    app.script_answers(&["", "g.txt"]);
    dispatch(&mut app, &[",", "r"]);
    assert_eq!(app.message(), "g.txt already exists");
    // the visited file is renamed with git mv; the buffer follows
    app.script_answers(&["", "h.txt"]);
    dispatch(&mut app, &[",", "r"]);
    assert_eq!(app.header_line(), "h.txt");
    assert_eq!(app.file_line(), Some(2));
    assert!(staged(&g).contains(&"h.txt".to_string()));
    // an untracked file is deleted from the disk
    app.script_answers(&["g.txt"]);
    dispatch(&mut app, &[",", "k"]);
    assert!(!exists(&g, "g.txt"));
    // the file from a revision
    app.script_answers(&["HEAD~", "f.txt"]);
    dispatch(&mut app, &[",", "c"]);
    assert_eq!(read(&g, "f.txt"), "a\nB\nc\n");
    // a tracked file is untracked
    g.git_interactive(&["add", "h.txt"]).unwrap();
    app.script_answers(&[""]);
    dispatch(&mut app, &[",", "x"]);
    assert!(!g.git_ok(&["ls-files", "--error-unmatch", "h.txt"]));
    assert!(exists(&g, "h.txt"));
    // magit-file-untrack reads tracked files only
    app.script_answers(&["h.txt"]);
    dispatch(&mut app, &[",", "x"]);
    assert_eq!(app.message(), "[No match]");
    // magit-file-delete also untracked ones
    app.script_answers(&[""]);
    dispatch(&mut app, &[",", "k"]);
    assert!(!exists(&g, "h.txt"));
}

#[test]
fn diff_and_log_of_the_visited_file() {
    let (_td, g) = file_repo();
    write(&g, "other.txt", "x\n");
    g.git_interactive(&["add", "other.txt"]).unwrap();
    let mut app = App::new(g.clone());
    visit(&mut app, 3);
    // the changes since the current branch, at the visited line
    dispatch(&mut app, &["d"]);
    match app.mode() {
        BufferMode::Diff(spec) => {
            assert_eq!(spec.range.as_deref(), Some("master"));
            assert_eq!(spec.files, ["f.txt"]);
        }
        m => panic!("{m:?}"),
    }
    assert_eq!(app.point_text(), "+C");
    app.press("q").unwrap();
    // a blob: its commit, limited to the file
    app.press("p").unwrap();
    dispatch(&mut app, &["d"]);
    assert!(matches!(app.mode(), BufferMode::Revision { files, .. } if files == &["f.txt"]), "{:?}", app.mode());
    app.press("q").unwrap();
    dispatch(&mut app, &["l"]);
    let head = g.git_string(&["rev-parse", "--short", "HEAD"]).unwrap();
    assert_eq!(app.mode(), &BufferMode::Log { revs: vec![head], files: vec!["f.txt".into()] });
    app.press("q").unwrap();
    // with a region, the log of its lines
    press(&mut app, &["C-p", "C-SPC", "C-n"]);
    assert_eq!(app.file_line(), Some(3));
    dispatch(&mut app, &["l"]);
    assert!(matches!(app.mode(), BufferMode::Log { files, .. } if files.is_empty()), "{:?}", app.mode());
    assert!(app.header_line().contains("-L2,3:f.txt"), "{}", app.header_line());
    let t: Vec<String> = app.visible_lines().iter().map(|l| l.text.clone()).collect();
    assert!(t.iter().any(|l| l.ends_with(" two")) && t.iter().any(|l| l.ends_with(" one")), "{t:#?}");
}

#[test]
fn trace_definition() {
    let (_td, g) = scratch_repo();
    write(&g, "lib.rs", "use x;\n\nfn foo() {\n    1\n}\n");
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", "add foo"]).unwrap();
    write(&g, "lib.rs", "use x;\n\nfn foo() {\n    2\n}\n");
    g.git_interactive(&["commit", "-q", "-am", "change foo"]).unwrap();
    write(&g, "lib.rs", "use y;\n\nfn foo() {\n    2\n}\n");
    let mut app = App::new(g.clone());
    goto(&mut app, "modified   lib.rs");
    app.press("RET").unwrap();
    dispatch(&mut app, &["t"]);
    assert_eq!(app.message(), "No function at point found");
    press(&mut app, &["C-n", "C-n", "C-n"]);
    dispatch(&mut app, &["t"]);
    assert!(app.header_line().contains("-L:foo:lib.rs"), "{}", app.header_line());
    let t: Vec<String> = app.visible_lines().iter().map(|l| l.text.clone()).collect();
    assert!(t.iter().any(|l| l.ends_with(" change foo")) && t.iter().any(|l| l.ends_with(" add foo")), "{t:#?}");
}

#[test]
fn navigation() {
    let (_td, g) = file_repo();
    let mut app = App::new(g.clone());
    visit(&mut app, 3);
    // magit-status-here: the visited line in the diff of the status buffer
    dispatch(&mut app, &["g"]);
    assert_eq!(app.mode(), &BufferMode::Status);
    assert_eq!(app.point_text(), "+C");
    // magit-display-repository-buffer
    app.script_answers(&["f.txt"]);
    dispatch(&mut app, &["G"]);
    assert_eq!(app.header_line(), "f.txt");
    assert_eq!(app.file_line(), Some(3));
    app.script_answers(&["magit: nope"]);
    dispatch(&mut app, &["G"]);
    assert_eq!(app.message(), "[No match]");
    // magit-find-file keeps the line of the same file
    app.script_answers(&["HEAD~", ""]);
    dispatch(&mut app, &["v"]);
    assert_eq!(app.header_line(), "f.txt.~HEAD~~");
    assert_eq!(app.file_line(), Some(3));
    // magit-blob-visit-file
    app.press("C-p").unwrap();
    dispatch(&mut app, &["V"]);
    assert_eq!(app.header_line(), "f.txt");
    assert_eq!(app.file_line(), Some(2));
    dispatch(&mut app, &["p"]);
    assert!(app.header_line().starts_with("f.txt.~"), "{}", app.header_line());
    dispatch(&mut app, &["n"]);
    assert_eq!(app.header_line(), "f.txt");
}

#[test]
fn edit_line_commit() {
    let (_td, g) = file_repo();
    let mut app = App::new(g.clone());
    visit(&mut app, 5);
    dispatch(&mut app, &["e"]);
    assert_eq!(app.message(), "This line has not been committed yet");
    // B was added by "two": the rebase stops there
    press(&mut app, &["C-p", "C-p", "C-p"]);
    assert_eq!(app.file_line(), Some(2));
    g.git_interactive(&["stash", "-q"]).unwrap();
    dispatch(&mut app, &["e"]);
    assert!(g.gitdir.join("rebase-merge").exists(), "{}", app.message());
    assert_eq!(g.git_string(&["log", "-1", "--format=%s"]).unwrap(), "two");
    assert_eq!(app.header_line(), "f.txt");
}
