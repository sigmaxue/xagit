//! magit-discard on the hunk of a conflicted file (magit-smerge-keep-
//! current) and the smerge commands of hunks (C-c ^ RET/u/b/l/a,
//! magit-hunk-section-smerge-map).

mod common;

use common::*;
use ngit_lib::prelude::*;
use ngit_lib::ui::app::App;

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn commit(g: &Git, file: &str, content: &str, msg: &str) {
    write(g, file, content);
    git(g, &["add", "."]);
    git(g, &["commit", "-q", "-m", msg]);
}

fn lines(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

fn goto(app: &mut App, pred: impl Fn(&str) -> bool) {
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-p").unwrap();
    }
    for _ in 0..n {
        if pred(app.point_text()) {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no such line: {:?}", lines(app));
}

/// f.txt changed to "ours" on master and to "theirs" on other, merged:
/// the conflict is at lines 4-8. With DIFF3 merge.conflictStyle diff3.
fn conflicted(diff3: bool) -> (tempfile::TempDir, Git, App) {
    let (td, g, _) = repo_with_commit();
    if diff3 {
        git(&g, &["config", "merge.conflictStyle", "diff3"]);
    }
    commit(&g, "f.txt", "a\nb\nc\nd\ne\nf\ng\n", "f");
    git(&g, &["checkout", "-q", "-b", "other"]);
    commit(&g, "f.txt", "a\nb\nc\ntheirs\ne\nf\ng\n", "theirs");
    git(&g, &["checkout", "-q", "master"]);
    commit(&g, "f.txt", "a\nb\nc\nours\ne\nf\ng\n", "ours");
    assert!(g.git_interactive(&["merge", "-q", "other"]).is_err());
    let mut app = App::new(g.clone());
    goto(&mut app, |t| t.contains("f.txt"));
    app.press("tab").unwrap();
    (td, g, app)
}

const OURS: &str = "a\nb\nc\nours\ne\nf\ng\n";
const THEIRS: &str = "a\nb\nc\ntheirs\ne\nf\ng\n";

fn status(g: &Git) -> String {
    g.git_string(&["status", "--porcelain"]).unwrap()
}

#[test]
fn discard_keeps_the_side_at_point() {
    let (_td, g, mut app) = conflicted(false);
    goto(&mut app, |t| t.trim_start() == "+ theirs");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts(), ["Discard hunk? (y or n) "]);
    assert_eq!(read(&g, "f.txt"), THEIRS);
    assert_eq!(app.message(), format!("Wrote {}", g.toplevel.join("f.txt").display()));
    assert_eq!(status(&g), "UU f.txt", "still to be staged");
    // ours
    git(&g, &["checkout", "--merge", "--", "f.txt"]);
    app.refresh();
    goto(&mut app, |t| t.trim_start() == "+ours");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(read(&g, "f.txt"), OURS);
    // the hunk heading visits the line after the first changed one,
    // the first line of ours here
    git(&g, &["checkout", "--merge", "--", "f.txt"]);
    app.refresh();
    goto(&mut app, |t| t.starts_with("@@@"));
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(read(&g, "f.txt"), OURS);
}

#[test]
fn discard_outside_a_version() {
    let (_td, g, mut app) = conflicted(false);
    let conflict = read(&g, "f.txt");
    // a conflict marker: smerge-keep-current fails
    goto(&mut app, |t| t.trim_start().starts_with("++<<<<<<<"));
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.message(), "Not inside a version");
    assert_eq!(read(&g, "f.txt"), conflict);
    // declined
    goto(&mut app, |t| t.trim_start() == "+ours");
    app.script_answers(&["n"]);
    app.press("k").unwrap();
    assert_eq!(app.message(), "Abort");
    assert_eq!(read(&g, "f.txt"), conflict);
    // before the conflict: the side to keep of the next one is read
    goto(&mut app, |t| t.trim() == "a");
    app.script_answers(&["y", "x", "t"]);
    app.press("k").unwrap();
    let p = "Keep side: [o]urs/upper, [b]ase, [t]heirs/lower, or [C-g] to abort ";
    assert_eq!(app.prompts()[app.prompts().len() - 2..], [p, p]);
    assert_eq!(read(&g, "f.txt"), THEIRS);
    // no base
    git(&g, &["checkout", "--merge", "--", "f.txt"]);
    app.refresh();
    let conflict = read(&g, "f.txt");
    goto(&mut app, |t| t.trim() == "b");
    app.script_answers(&["y", "b"]);
    app.press("k").unwrap();
    assert_eq!(app.message(), "No `base'");
    assert_eq!(read(&g, "f.txt"), conflict);
}

#[test]
fn smerge_map() {
    let (_td, g, mut app) = conflicted(true);
    let conflict = read(&g, "f.txt");
    assert!(conflict.contains("|||||||"), "{conflict}");
    let again = |app: &mut App| {
        git(&g, &["checkout", "--merge", "--", "f.txt"]);
        app.refresh();
    };
    goto(&mut app, |t| t.trim_start() == "+ours");
    for k in ["C-c", "^", "b"] {
        app.press(k).unwrap();
    }
    assert_eq!(read(&g, "f.txt"), "a\nb\nc\nd\ne\nf\ng\n");
    again(&mut app);
    // the base at point
    goto(&mut app, |t| t.trim_start() == "++d");
    for k in ["C-c", "^", "enter"] {
        app.press(k).unwrap();
    }
    assert_eq!(read(&g, "f.txt"), "a\nb\nc\nd\ne\nf\ng\n");
    again(&mut app);
    // outside a conflict: the next one, but not for RET
    goto(&mut app, |t| t.trim() == "a");
    for k in ["C-c", "^", "enter"] {
        app.press(k).unwrap();
    }
    assert_eq!(app.message(), "Point not in conflict region");
    for k in ["C-c", "^", "l"] {
        app.press(k).unwrap();
    }
    assert_eq!(read(&g, "f.txt"), THEIRS);
    again(&mut app);
    goto(&mut app, |t| t.trim() == "a");
    for k in ["C-c", "^", "a"] {
        app.press(k).unwrap();
    }
    assert_eq!(read(&g, "f.txt"), "a\nb\nc\nours\nd\ntheirs\ne\nf\ng\n");
    again(&mut app);
    goto(&mut app, |t| t.trim() == "a");
    for k in ["C-c", "^", "x"] {
        app.press(k).unwrap();
    }
    assert_eq!(app.message(), "C-c ^ x is undefined");
    for k in ["C-c", "^", "u"] {
        app.press(k).unwrap();
    }
    assert_eq!(read(&g, "f.txt"), OURS);
    // only on hunks
    goto(&mut app, |t| t.starts_with("Head:"));
    for k in ["C-c", "^"] {
        app.press(k).unwrap();
    }
    assert_eq!(app.message(), "C-c ^ is undefined");
}
