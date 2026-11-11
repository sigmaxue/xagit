//! Work done in the background (as in a terminal): refreshes on a
//! worker thread, fetch / pull / push as background processes; the
//! results are shown once they are in.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn rev(g: &Git, r: &str) -> String {
    g.git_string(&["rev-parse", "--verify", "-q", r]).unwrap_or_default().trim().to_string()
}

fn commit(g: &Git, file: &str) -> String {
    write(g, file, file);
    git(g, &["add", file]);
    git(g, &["commit", "-qm", file]);
    rev(g, "HEAD")
}

/// A repository `up` and a clone of it (master tracking origin/master).
fn cloned() -> (TempDir, Git, TempDir, Git) {
    let (utd, up, _) = repo_with_commit();
    let td = TempDir::new().unwrap();
    let dir = td.path().join("clone");
    let ok = std::process::Command::new("git")
        .args(["clone", "-q", &up.toplevel.to_string_lossy(), &dir.to_string_lossy()])
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let g = Git::discover(&dir).unwrap();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    (utd, up, td, g)
}

fn text(app: &App) -> String {
    app.visible_lines().iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn refresh_is_shown_when_done() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    app.set_background(true);
    write(&g, "new.txt", "new\n");
    app.press("g").unwrap();
    app.wait_background();
    assert!(text(&app).contains("Untracked files (1)"), "{}", text(&app));
    // a refresh started after another wins
    write(&g, "other.txt", "other\n");
    app.press("g").unwrap();
    app.refresh();
    app.wait_background();
    assert!(text(&app).contains("Untracked files (2)"), "{}", text(&app));
}

#[test]
fn staging_works_on_the_refreshed_buffer() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "a.txt", "a\n");
    write(&g, "b.txt", "b\n");
    let mut app = App::new(g.clone());
    app.set_background(true);
    let at = app.visible_lines().iter().position(|l| l.text.starts_with("Untracked files")).unwrap();
    while app.point_text() != app.visible_lines()[at].text {
        app.press("C-n").unwrap();
    }
    app.press("s").unwrap();
    app.wait_background();
    let t = text(&app);
    assert!(t.contains("Staged changes (2)"), "{t}");
    assert!(!t.contains("Untracked files"), "{t}");
}

#[test]
fn pull_runs_in_the_background() {
    let (_utd, up, _td, g) = cloned();
    let new = commit(&up, "a.txt");
    let mut app = App::new(g.clone());
    app.set_background(true);
    app.press("F").unwrap();
    app.press("u").unwrap();
    assert!(app.message().starts_with("Running git pull"), "{}", app.message());
    app.wait_background();
    assert_eq!(rev(&g, "HEAD"), new, "{}", app.message());
    assert!(app.message().starts_with("pulled"), "{}", app.message());
    // the buffer shows the commit pulled
    assert!(text(&app).contains(" a.txt"), "{}", text(&app));
}

#[test]
fn failed_fetch_is_reported() {
    let (_utd, _up, _td, g) = cloned();
    git(&g, &["remote", "set-url", "origin", "/nonexistent/repository"]);
    let mut app = App::new(g.clone());
    app.set_background(true);
    app.press("f").unwrap();
    app.press("u").unwrap();
    app.wait_background();
    let (_, details) = app.text_view().expect("the failure is shown");
    assert!(details.contains("fetch"), "{details}");
    assert!(details.contains("/nonexistent/repository"), "{details}");
}

#[test]
fn refresh_done_restores_ui_state() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g);
    app.set_background(true);
    app.press("C-SPC").unwrap();
    assert!(app.region_active(), "mark should be active");
    app.script_answers(&["file"]);
    app.press("C-s").unwrap();
    assert_eq!(app.search_query(), Some("file"));
    let before = app.point_text().to_string();

    app.press("g").unwrap();
    app.wait_background();
    assert!(!app.region_active(), "mark must not survive a rebuild");
    assert_eq!(app.search_query(), Some("file"), "a refresh keeps the query");
    assert_eq!(app.point_text(), before, "point is restored by its anchor");

    app.press("y").unwrap();
    assert_eq!(app.search_query(), None, "leaving the buffer clears the query");
}
