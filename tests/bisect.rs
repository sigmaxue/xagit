//! magit-bisect (B).

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;
use xagit_lib::ui::options::Options;

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

/// Commits writing 1..=5 to f; returns their hashes.
fn repo() -> (TempDir, Git, Vec<String>) {
    let (td, g) = scratch_repo();
    let mut hashes = Vec::new();
    for i in 1..=5 {
        write(&g, "f", &format!("{i}\n"));
        g.git_interactive(&["add", "f"]).unwrap();
        g.commit(Some(&format!("c{i}")), &[]).unwrap();
        hashes.push(g.rev_parse("HEAD").unwrap());
    }
    (td, g, hashes)
}

fn output(g: &Git) -> String {
    std::fs::read_to_string(g.gitdir.join("BISECT_CMD_OUTPUT")).unwrap_or_default()
}

fn texts(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

#[test]
fn menus() {
    let (_td, g, _) = repo();
    let mut app = App::new(g.clone());
    app.press("B").unwrap();
    // --term-old= / --term-new= and Mark are at level 6
    assert_eq!(app.menu_keys(), ["-n", "-p", "B", "s"]);
    app.press("C-g").unwrap();
    let (o, _) = Options::parse("transient-level magit-bisect 6");
    app.set_options(o);
    app.press("B").unwrap();
    assert_eq!(app.menu_keys(), ["-n", "-p", "=o", "=n", "B", "s"]);
}

#[test]
fn start_mark_and_reset() {
    let (_td, g, h) = repo();
    let mut app = App::new(g.clone());
    app.script_answers(&["HEAD", &h[0]]);
    press(&mut app, &["B", "B"]);
    assert_eq!(app.prompts(), ["Start bisect with bad revision", "Good revision"]);
    assert!(g.bisect_in_progress(), "{}", app.message());
    assert_eq!(app.message(), "Bisecting...done");
    // the output is shown in the status buffer
    assert!(output(&g).starts_with("Bisecting:"), "{}", output(&g));
    assert!(texts(&app).iter().any(|l| l.starts_with("Bisecting:")));
    // B / g mark HEAD (no revision is read)
    let n = app.prompts().len();
    press(&mut app, &["B", "g"]);
    assert_eq!(app.prompts().len(), n);
    assert!(g.git_string(&["bisect", "log"]).unwrap().contains("# good:"));
    app.script_answers(&["y"]);
    press(&mut app, &["B", "r"]);
    assert!(!g.bisect_in_progress());
    assert!(!g.gitdir.join("BISECT_CMD_OUTPUT").exists());
}

#[test]
fn run_before_bisecting() {
    let (_td, g, h) = repo();
    let mut app = App::new(g.clone());
    app.script_answers(&["HEAD", &h[0], "test $(cat f) -lt 4"]);
    press(&mut app, &["B", "s"]);
    assert_eq!(app.prompts(), ["Start bisect with bad revision", "Good revision", "Bisect shell command: "]);
    // "is the first bad commit" ('bad' quoted by newer Git)
    let out = output(&g);
    let first = format!("{} is the first", h[3]);
    assert!(out.contains(&first), "{}\n{out}", app.message());
    // the output of `git bisect run` in the status buffer
    let line = out.lines().next().unwrap();
    assert!(texts(&app).iter().any(|l| l == line), "{line}");
}

#[test]
fn alternate_terms() {
    let (_td, g, h) = repo();
    let mut app = App::new(g.clone());
    let (o, _) = Options::parse("transient-level magit-bisect 6");
    app.set_options(o);
    app.script_answers(&["fast", "slow", "HEAD", &h[0]]);
    press(&mut app, &["B", "=", "o", "=", "n", "B"]);
    assert_eq!(app.prompts()[2..], ["Start bisect with slow revision", "fast revision"]);
    assert_eq!(std::fs::read_to_string(g.gitdir.join("BISECT_TERMS")).unwrap(), "slow\nfast\n");
    app.script_keys(&["o"]);
    press(&mut app, &["B", "m"]);
    assert_eq!(app.prompts().last().unwrap(), "Mark HEAD as slow ([n]ew) or fast ([o]ld)");
    assert!(g.git_string(&["bisect", "log"]).unwrap().contains("# fast:"), "{}", app.message());
}

#[test]
fn uncommitted_changes() {
    let (_td, g, h) = repo();
    write(&g, "f", "dirty\n");
    let mut app = App::new(g.clone());
    app.script_answers(&["HEAD", &h[0]]);
    press(&mut app, &["B", "B"]);
    assert_eq!(app.message(), "Cannot bisect with uncommitted changes");
    assert!(!g.bisect_in_progress());
}
