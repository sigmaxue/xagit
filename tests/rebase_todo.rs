//! git-rebase-mode: `r i` edits the todo list in xagit (git-rebase.el):
//! the keys change the actions, move and comment lines, C-c C-c rebases
//! and C-c C-k aborts without touching the repository.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn subjects(g: &Git) -> Vec<String> {
    g.git_string(&["log", "--format=%s"]).unwrap().lines().map(str::to_string).collect()
}

/// c1..c4, each adding its own file.
fn repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    for i in 1..=4 {
        write(&g, &format!("f{i}"), &format!("{i}\n"));
        git(&g, &["add", "."]);
        git(&g, &["commit", "-q", "-m", &format!("c{i}")]);
    }
    (td, g)
}

/// Point on the commit line of SUBJECT in the status buffer.
fn goto_commit(app: &mut App, subject: &str) {
    // the commit sections are folded: show everything
    app.press("M-4").unwrap();
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-p").unwrap();
    }
    for _ in 0..n {
        if app.point_text().ends_with(&format!(" {subject}")) {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no commit {subject}: {:?}", app.visible_lines().iter().map(|l| l.text.clone()).collect::<Vec<_>>());
}

/// `r i` on c2: the todo of c2..c4 in the git-rebase-mode buffer.
fn open_todo(g: &Git) -> App {
    let mut app = App::new(g.clone());
    goto_commit(&mut app, "c2");
    press(&mut app, &["r", "i"]);
    assert_eq!(app.mode(), &BufferMode::RebaseTodo, "{}", app.message());
    app
}

fn actions(app: &App) -> Vec<String> {
    app.rebase_todo_lines()
        .unwrap()
        .into_iter()
        .take_while(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut w = l.split(' ');
            let a = w.next().unwrap_or("").to_string();
            let _hash = w.next();
            let rest: Vec<&str> = w.collect();
            // git >= 2.50 writes `pick <hash> # <subject>`, older git omits `#`
            let subject = if rest.first() == Some(&"#") { &rest[1..] } else { &rest[..] };
            format!("{a} {}", subject.join(" "))
        })
        .collect()
}

#[test]
fn opens_without_touching_the_repository() {
    let (_td, g) = repo();
    let head = g.rev_parse("HEAD").unwrap();
    let app = open_todo(&g);
    assert_eq!(actions(&app), ["pick c2", "pick c3", "pick c4"]);
    // the instructions follow, as in git-rebase-mode
    assert!(app.rebase_todo_lines().unwrap().iter().any(|l| l.starts_with("# Rebase ")));
    assert!(!g.rebase_in_progress());
    assert_eq!(g.rev_parse("HEAD").unwrap(), head);
}

#[test]
fn action_keys_advance_and_finish_rebases() {
    let (_td, g) = repo();
    let mut app = open_todo(&g);
    // r: reword, then point is on the next line; d drops c3; s squashes c4
    press(&mut app, &["r"]);
    assert_eq!(app.rebase_todo_cursor(), Some(1));
    press(&mut app, &["d", "f"]);
    assert_eq!(actions(&app), ["reword c2", "drop c3", "fixup c4"]);
    press(&mut app, &["C-c", "C-c"]);
    assert_ne!(app.mode(), &BufferMode::RebaseTodo);
    // git stops at the reworded commit: xagit's text buffer edits the
    // message (instead of an external editor), C-c C-c lets rebase go on
    assert_eq!(app.mode(), &BufferMode::TextEdit, "{}", app.message());
    app.text_edit_set("reworded\n");
    press(&mut app, &["C-c", "C-c"]);
    assert!(!g.rebase_in_progress(), "{}", app.message());
    assert_eq!(subjects(&g), ["reworded", "c1"], "{}", app.message());
    assert!(!exists(&g, "f3"), "c3 dropped");
    assert!(exists(&g, "f4"), "c4 folded into c2");
}

#[test]
fn all_commit_actions() {
    let (_td, g) = repo();
    let mut app = open_todo(&g);
    let keys = [
        ("c", "pick"),
        ("w", "reword"),
        ("e", "edit"),
        ("m", "edit"),
        ("s", "squash"),
        ("S", "fixup -c"),
        ("f", "fixup"),
        ("F", "fixup -C"),
        ("A", "fixup -C"),
        ("d", "drop"),
    ];
    for (key, action) in keys {
        press(&mut app, &["<"]);
        app.press(key).unwrap();
        let first = app.rebase_todo_lines().unwrap()[0].clone();
        assert!(first.starts_with(&format!("{action} ")), "{key}: {first}");
        assert!(first.ends_with(" c2"), "{key}: {first}");
    }
}

#[test]
fn move_lines_and_region() {
    let (_td, g) = repo();
    let mut app = open_todo(&g);
    // M-n moves c2 below c3, M-p back up
    press(&mut app, &["M-n"]);
    assert_eq!(actions(&app), ["pick c3", "pick c2", "pick c4"]);
    assert_eq!(app.rebase_todo_cursor(), Some(1));
    press(&mut app, &["M-p"]);
    assert_eq!(actions(&app), ["pick c2", "pick c3", "pick c4"]);
    assert_eq!(app.rebase_todo_cursor(), Some(0));
    // the top line can't move up
    press(&mut app, &["M-p"]);
    assert_eq!(app.message(), "Cannot move further");
    // a region of two lines: the action applies to both
    press(&mut app, &["C-SPC", "n", "r"]);
    assert_eq!(actions(&app), ["reword c2", "reword c3", "pick c4"]);
    // reorder and rebase: c4 first
    press(&mut app, &["<", "c", "c", "M-p", "M-p"]);
    assert_eq!(actions(&app), ["pick c4", "pick c2", "pick c3"]);
    press(&mut app, &["C-c", "C-c"]);
    assert_eq!(subjects(&g), ["c3", "c2", "c4", "c1"], "{}", app.message());
}

#[test]
fn kill_comments_and_undo() {
    let (_td, g) = repo();
    let mut app = open_todo(&g);
    press(&mut app, &["n", "k"]);
    assert!(app.rebase_todo_lines().unwrap()[1].starts_with("# pick "));
    // k again on a commented line uncomments it
    press(&mut app, &["p", "k"]);
    assert!(app.rebase_todo_lines().unwrap()[1].starts_with("pick "));
    press(&mut app, &["p", "k"]);
    assert!(app.rebase_todo_lines().unwrap()[1].starts_with("# pick "));
    press(&mut app, &["<", "k"]);
    assert!(app.rebase_todo_lines().unwrap()[0].starts_with("# pick "));
    press(&mut app, &["C-/"]);
    assert!(app.rebase_todo_lines().unwrap()[0].starts_with("pick "));
    assert!(app.rebase_todo_lines().unwrap()[1].starts_with("# pick "));
    // a commented commit is not rebased (git drops it)
    press(&mut app, &["C-c", "C-c"]);
    assert_eq!(subjects(&g), ["c4", "c2", "c1"], "{}", app.message());
}

#[test]
fn exec_break_and_insert() {
    let (_td, g) = repo();
    let mut app = open_todo(&g);
    app.script_answers(&["touch ran"]);
    press(&mut app, &["x"]);
    assert_eq!(app.prompts().last().unwrap(), "Execute: ");
    assert_eq!(app.rebase_todo_lines().unwrap()[1], "exec touch ran");
    // point is on c3 now: break is inserted before it
    press(&mut app, &["b"]);
    assert_eq!(app.rebase_todo_lines().unwrap()[2], "break");
    assert_eq!(app.rebase_todo_cursor(), Some(3));
    // y inserts a pick of another commit below point
    app.script_answers(&["HEAD~3"]);
    press(&mut app, &["y"]);
    let l = app.rebase_todo_lines().unwrap()[4].clone();
    assert!(l.starts_with("pick ") && l.ends_with(" c1"), "{l}");
    // undo the insert, and comment the break
    press(&mut app, &["C-/", "p", "k"]);
    assert!(app.rebase_todo_lines().unwrap()[2].starts_with("# break"));
    assert!(!app.rebase_todo_lines().unwrap().iter().any(|l| l.ends_with(" c1")));
    press(&mut app, &["C-c", "C-c"]);
    assert!(exists(&g, "ran"), "exec ran: {}", app.message());
    assert!(!g.rebase_in_progress(), "{}", app.message());
}

#[test]
fn cancel_aborts_without_changes() {
    let (_td, g) = repo();
    let head = g.rev_parse("HEAD").unwrap();
    // unmodified: no question
    let mut app = open_todo(&g);
    press(&mut app, &["C-c", "C-k"]);
    assert_eq!(app.mode(), &BufferMode::Status);
    assert_eq!(app.message(), "Rebase aborted");
    // modified: confirmed (abort-rebase); declining keeps editing
    let mut app = open_todo(&g);
    press(&mut app, &["d"]);
    app.script_answers(&["n"]);
    press(&mut app, &["C-c", "C-k"]);
    assert_eq!(app.mode(), &BufferMode::RebaseTodo);
    app.script_answers(&["y"]);
    press(&mut app, &["C-c", "C-k"]);
    assert_eq!(app.mode(), &BufferMode::Status);
    assert_eq!(g.rev_parse("HEAD").unwrap(), head);
    assert!(!g.rebase_in_progress());
    assert_eq!(subjects(&g), ["c4", "c3", "c2", "c1"]);
}

#[test]
fn edit_stops_then_r_e_edits_the_rest() {
    let (_td, g) = repo();
    let mut app = open_todo(&g);
    // stop at c2
    press(&mut app, &["e", "C-c", "C-c"]);
    assert!(g.rebase_in_progress(), "{}", app.message());
    assert_eq!(app.mode(), &BufferMode::Status);
    // r e: the rest of the todo (c3, c4) in the buffer
    press(&mut app, &["r", "e"]);
    assert_eq!(app.mode(), &BufferMode::RebaseTodo, "{}", app.message());
    assert_eq!(actions(&app), ["pick c3", "pick c4"]);
    press(&mut app, &["d", "C-c", "C-c"]);
    assert!(g.rebase_in_progress());
    // r r continues with the edited todo
    press(&mut app, &["r", "r"]);
    assert!(!g.rebase_in_progress(), "{}", app.message());
    assert_eq!(subjects(&g), ["c4", "c2", "c1"]);
}

#[test]
fn dirty_worktree_is_reported() {
    let (_td, g) = repo();
    write(&g, "f1", "dirty\n");
    let mut app = App::new(g.clone());
    goto_commit(&mut app, "c2");
    // without --autostash git refuses before offering a todo
    press(&mut app, &["r", "-", "A", "i"]);
    assert_eq!(app.mode(), &BufferMode::Status);
    assert!(app.message().contains("rebase"), "{}", app.message());
    assert_eq!(read(&g, "f1"), "dirty\n");
    // with --autostash (the default) the todo is offered and the change
    // stays in the worktree
    press(&mut app, &["r", "i"]);
    assert_eq!(app.mode(), &BufferMode::RebaseTodo, "{}", app.message());
    assert_eq!(read(&g, "f1"), "dirty\n");
    press(&mut app, &["d", "C-c", "C-c"]);
    assert_eq!(subjects(&g), ["c4", "c3", "c1"], "{}", app.message());
    assert_eq!(read(&g, "f1"), "dirty\n");
}

/// Two `reword` commits ask git to open its editor twice: xagit answers
/// both requests from the built-in text buffer, one after the other.
#[test]
fn two_reword_messages_use_the_builtin_buffer() {
    let (_td, g) = repo();
    let mut app = open_todo(&g);
    press(&mut app, &["r", "r"]); // reword c2 and c3
    assert_eq!(actions(&app), ["reword c2", "reword c3", "pick c4"]);
    press(&mut app, &["C-c", "C-c"]);
    // first message (c2)
    assert_eq!(app.mode(), &BufferMode::TextEdit, "{}", app.message());
    assert!(app.text_edit_text().unwrap().contains("c2"));
    app.text_edit_set("c2 new\n");
    press(&mut app, &["C-c", "C-c"]);
    // second message (c3)
    assert_eq!(app.mode(), &BufferMode::TextEdit, "{}", app.message());
    assert!(app.text_edit_text().unwrap().contains("c3"));
    app.text_edit_set("c3 new\n");
    press(&mut app, &["C-c", "C-c"]);
    assert!(!g.rebase_in_progress(), "{}", app.message());
    assert_eq!(subjects(&g), ["c4", "c3 new", "c2 new", "c1"], "{}", app.message());
}

/// The commit message git prepares after the todo starts with a reword:
/// xagit puts the point in the message (not in git's comment instructions),
/// shows where it is, M-p/M-n move over the non-comment blocks and
/// M-k / C-k kill a whole block / line regardless of the column.
#[test]
fn reword_message_starts_in_the_message_block() {
    let (_td, g) = repo();
    let mut app = open_todo(&g);
    press(&mut app, &["r", "C-c", "C-c"]);
    assert_eq!(app.mode(), &BufferMode::TextEdit, "{}", app.message());
    let text = app.text_edit_text().unwrap();
    let subject = text.lines().next().unwrap();
    let n = text.lines().count();
    assert_eq!(app.text_edit_point(), Some((0, subject.chars().count())), "{text}");
    assert_eq!(app.text_edit_position(), Some(format!("1:{}/{n}", subject.chars().count() + 1)));
    // a second non-comment block, separated by git-style instruction lines
    app.text_edit_set("subject\n\nbody\n# instruction\n# another\nsecond block\n");
    app.press("M-n").unwrap();
    assert_eq!(app.text_edit_point(), Some((5, 12)), "{}", app.text_edit_text().unwrap());
    app.press("M-p").unwrap();
    assert_eq!(app.text_edit_point(), Some((2, 4)));
    // M-k kills the whole block at point, comments and the next block stay
    app.press("M-k").unwrap();
    let text = app.text_edit_text().unwrap();
    assert!(!text.contains("subject") && !text.contains("body"), "{text}");
    assert!(text.contains("# instruction") && text.contains("second block"), "{text}");
    // C-k kills the whole line even when point is in its middle
    app.press("M-n").unwrap();
    app.press("C-b").unwrap();
    app.press("C-k").unwrap();
    assert!(!app.text_edit_text().unwrap().contains("second block"), "{}", app.text_edit_text().unwrap());
    // C-/ restores the killed line in the sequencer's message buffer too
    app.press("C-/").unwrap();
    assert!(app.text_edit_text().unwrap().contains("second block"), "{}", app.text_edit_text().unwrap());
    app.text_edit_set("reworded\n");
    press(&mut app, &["C-c", "C-c"]);
    assert!(!g.rebase_in_progress(), "{}", app.message());
}
