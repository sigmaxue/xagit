//! magit-log-select-mode: commands that act on a commit select it from
//! the log when there is none at point (rebase) or always (fixup/squash).

mod common;

use common::*;
use ngit_lib::prelude::*;
use ngit_lib::ui::app::{App, BufferMode, SelectAction};

/// initial, two (two.txt), fixup! two (two.txt changed again).
fn repo() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    commit_file(&g, "two.txt", "2\n", "two");
    commit_file(&g, "three.txt", "3\n", "three");
    (td, g)
}

fn commit_file(g: &Git, file: &str, content: &str, msg: &str) {
    write(g, file, content);
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

fn is_select(app: &App) -> bool {
    matches!(app.mode(), BufferMode::LogSelect { .. })
}

#[test]
fn fixup_selects_the_target_from_the_log() {
    let (_td, g) = repo();
    write(&g, "two.txt", "two\n");
    g.git_interactive(&["add", "."]).unwrap();
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    app.press("f").unwrap();
    assert!(is_select(&app), "{:?}", app.mode());
    let usage = "Type C-c C-c on a commit to fixup= into it, or C-c C-k to abort";
    assert_eq!(app.header_line(), usage);
    assert_eq!(app.message(), usage);
    assert!(app.point_text().ends_with(" three"), "{}", app.point_text());
    goto(&mut app, " two");
    app.press("C-c").unwrap();
    app.press("C-c").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    assert_eq!(subjects(&g), ["fixup! two", "three", "two", "initial"]);
}

#[test]
fn quitting_selects_nothing() {
    let (_td, g) = repo();
    write(&g, "two.txt", "two\n");
    g.git_interactive(&["add", "."]).unwrap();
    let mut app = App::new(g.clone());
    for quit in [&["q"][..], &["C-c", "C-k"][..]] {
        app.press("c").unwrap();
        app.press("s").unwrap();
        assert!(is_select(&app));
        for k in quit {
            app.press(k).unwrap();
        }
        assert_eq!(app.mode(), &BufferMode::Status);
    }
    assert_eq!(subjects(&g), ["three", "two", "initial"]);
}

#[test]
fn nothing_to_fixup() {
    let (_td, g) = repo();
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    app.press("f").unwrap();
    assert_eq!(app.message(), "Nothing staged (or unstaged)");
    assert_eq!(app.mode(), &BufferMode::Status);
    // unstaged changes: offer to commit them all
    write(&g, "two.txt", "two\n");
    app.script_answers(&["y"]);
    app.press("c").unwrap();
    app.press("f").unwrap();
    match app.mode() {
        BufferMode::LogSelect { action: SelectAction::CommitSquash { args, .. }, .. } => {
            assert_eq!(args, &["--all".to_string(), "--verbose".to_string()])
        }
        m => panic!("{m:?}"),
    }
}

#[test]
fn select_skips_fixups_and_starts_at_point() {
    let (_td, g) = repo();
    write(&g, "two.txt", "two\n");
    g.git_interactive(&["commit", "-q", "-am", "fixup! two"]).unwrap();
    write(&g, "three.txt", "three\n");
    g.git_interactive(&["add", "."]).unwrap();
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    app.press("s").unwrap();
    assert!(app.point_text().ends_with(" three"), "{}", app.point_text());
    app.press("q").unwrap();
    // from a log buffer: the commit at point
    app.press("l").unwrap();
    app.press("l").unwrap();
    goto(&mut app, " initial");
    app.press("c").unwrap();
    app.press("s").unwrap();
    assert!(is_select(&app));
    assert!(app.point_text().ends_with(" initial"), "{}", app.point_text());
    // q returns to the log buffer
    app.press("q").unwrap();
    assert!(matches!(app.mode(), BufferMode::Log { .. }));
}

#[test]
fn instant_fixup_squashes_right_away() {
    let (_td, g) = repo();
    write(&g, "two.txt", "two\n");
    g.git_interactive(&["add", "."]).unwrap();
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    app.press("F").unwrap();
    goto(&mut app, " two");
    app.press(".").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    assert_eq!(subjects(&g), ["three", "two", "initial"]);
    assert_eq!(g.git_string(&["show", "HEAD~1:two.txt"]).unwrap(), "two");
    assert!(!g.rebase_in_progress());
}

#[test]
fn rebase_uses_the_commit_at_point_or_selects_one() {
    let (_td, g) = repo();
    let mut app = App::new(g.clone());
    // nothing at point: select
    app.press("r").unwrap();
    app.press("k").unwrap();
    assert!(is_select(&app));
    assert_eq!(app.header_line(), "Type C-c C-c on a commit to remove it, or C-c C-k to abort");
    goto(&mut app, " two");
    app.press("e").unwrap();
    assert_eq!(subjects(&g), ["three", "initial"]);
    assert_eq!(app.mode(), &BufferMode::Status);
    // a commit at point is used as is
    app.press("l").unwrap();
    app.press("l").unwrap();
    goto(&mut app, " three");
    app.press("r").unwrap();
    app.press("k").unwrap();
    assert!(matches!(app.mode(), BufferMode::Log { .. }));
    assert_eq!(subjects(&g), ["initial"]);
}

#[test]
fn rebase_subset_and_autosquash_select() {
    let (_td, g) = repo();
    g.git_interactive(&["branch", "-q", "base", "HEAD~2"]).unwrap();
    commit_file(&g, "base.txt", "b\n", "base");
    g.git_interactive(&["checkout", "-q", "base"]).unwrap();
    commit_file(&g, "other.txt", "o\n", "other");
    g.git_interactive(&["checkout", "-q", "master"]).unwrap();
    let mut app = App::new(g.clone());
    // r s: rebase "three" and "base" onto base
    app.script_answers(&["base"]);
    app.press("r").unwrap();
    app.press("s").unwrap();
    assert!(is_select(&app));
    assert_eq!(
        app.header_line(),
        "Type C-c C-c on a commit to rebase it and commits above it onto base, or C-c C-k to abort"
    );
    goto(&mut app, " three");
    app.press(".").unwrap();
    assert_eq!(subjects(&g), ["base", "three", "other", "initial"]);

    // r f without an upstream: select the commit to squash into and above
    write(&g, "three.txt", "three\n");
    g.git_interactive(&["commit", "-q", "-am", "fixup! three"]).unwrap();
    app.press("r").unwrap();
    app.press("f").unwrap();
    assert!(is_select(&app));
    assert!(app.point_text().ends_with(" base"), "{}", app.point_text());
    goto(&mut app, " other");
    app.press(".").unwrap();
    assert_eq!(subjects(&g), ["base", "three", "other", "initial"]);
    assert_eq!(g.git_string(&["show", "HEAD~1:three.txt"]).unwrap(), "three");
}

#[test]
fn log_select_is_a_log_buffer() {
    let (_td, g) = repo();
    let mut app = App::new(g.clone());
    app.press("r").unwrap();
    app.press("i").unwrap();
    assert!(is_select(&app));
    // log arguments and margin of log buffers
    assert!(app.visible_lines()[0].text.contains('*'));
    assert!(!app.visible_lines()[0].margin.is_empty());
    app.press("L").unwrap();
    assert_eq!(app.current_menu(), Some("log-refresh-menu"));
    assert!(app.active_switches().contains(&"-n256".to_string()));
    app.press("C-g").unwrap();
    // RET shows the commit; q comes back
    goto(&mut app, " two");
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { .. }));
    app.press("q").unwrap();
    assert!(is_select(&app));
    app.press("C-c").unwrap();
    app.press("C-b").unwrap();
    assert_eq!(app.message(), "C-c C-b is undefined");
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
}
