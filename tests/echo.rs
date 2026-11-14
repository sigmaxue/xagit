//! Echo area: a key that runs a side-effecting git command shows a short
//! label of that command in the mode line (e.g. `$ git add -- new.txt`),
//! without replacing the command's message.

mod common;

use common::*;
use xagit_lib::ui::app::App;

fn goto(app: &mut App, prefix: &str) {
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-p").unwrap();
    }
    for _ in 0..n {
        if app.point_text().starts_with(prefix) {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no visible line starting with {prefix:?}");
}

/// Move the point onto an untracked file (its section starts folded).
fn goto_untracked(app: &mut App, file: &str) {
    goto(app, "Untracked files");
    app.press("TAB").unwrap(); // unfold the section (a pure UI action)
    goto(app, file);
}

#[test]
fn staging_echoes_the_git_command() {
    let (_td, g, _h) = repo_with_commit();
    write(&g, "new.txt", "new\n");
    let mut app = App::new(g.clone());
    assert_eq!(app.echo(), None, "no command has run yet");
    // moving / unfolding is not a git command: no echo
    goto_untracked(&mut app, "new.txt");
    assert_eq!(app.echo(), None, "movement must not echo");
    // stage the file at point: the mode line echoes what git ran
    app.press("s").unwrap();
    let echo = app.echo().unwrap_or("").to_string();
    assert!(echo.starts_with("git add"), "{echo:?}");
    assert!(echo.contains("new.txt"), "{echo:?}");
    // the echo lives in its own field: the command's message is untouched
    assert!(!app.message().contains("git add"), "echo must not be folded into message: {}", app.message());
    // and it is actually drawn in the mode line
    let line = app.mode_line_text(200);
    assert!(line.contains("$ git add") && line.contains("new.txt"), "{line:?}");
    // a plain movement produces no NEW echo (the last one stays)
    app.press("C-n").unwrap();
    assert_eq!(app.echo(), Some(echo.as_str()));
}

#[test]
fn echo_is_per_repository() {
    let (_td1, g1, _h) = repo_with_commit();
    let (_td2, g2, _h2) = repo_with_commit();
    write(&g1, "a.txt", "a\n");
    write(&g2, "b.txt", "b\n");
    let mut app1 = App::new(g1.clone());
    let mut app2 = App::new(g2.clone());
    goto_untracked(&mut app1, "a.txt");
    app1.press("s").unwrap();
    assert!(app1.echo().unwrap().contains("a.txt"), "{}", app1.echo().unwrap());
    // the other repository does not see app1's command
    assert_eq!(app2.echo(), None);
    goto_untracked(&mut app2, "b.txt");
    app2.press("s").unwrap();
    assert!(app2.echo().unwrap().contains("b.txt"), "{}", app2.echo().unwrap());
}

#[test]
fn commit_echoes_git_commit_without_the_message() {
    let (_td, g, _h) = repo_with_commit();
    write(&g, "f.txt", "change\n");
    g.git_interactive(&["add", "f.txt"]).unwrap();
    let mut app = App::new(g.clone());
    press(&mut app, &["c", "c"]); // opens the built-in COMMIT_EDITMSG buffer
    for ch in "my subject".chars() {
        app.press(&ch.to_string()).unwrap();
    }
    press(&mut app, &["C-c", "C-c"]); // commit
    let echo = app.echo().unwrap_or("").to_string();
    assert!(echo.starts_with("git commit"), "{echo:?}");
    assert!(!echo.contains("my subject"), "the message is not echoed: {echo:?}");
    assert_eq!(g.git_string(&["log", "-1", "--format=%s"]).unwrap(), "my subject", "{}", app.message());
}
