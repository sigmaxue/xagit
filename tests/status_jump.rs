//! magit-status-jump (j in the status buffer), imenu and magit-dispatch's
//! "j".

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

/// Untracked file n, unstaged change of a and a staged file s.
fn repo() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    write(&g, "n", "n\n");
    write(&g, "s", "s\n");
    g.git_interactive(&["add", "s"]).unwrap();
    write(&g, "file.txt", "changed\n");
    (td, g)
}

#[test]
fn menu_and_inapt_jumpers() {
    let (_td, g) = repo();
    let mut app = App::new(g);
    app.press("j").unwrap();
    assert_eq!(app.current_menu(), Some("status-jump-menu"));
    assert_eq!(app.menu_keys(), ["z", "n", "u", "s", "fu", "fp", "pu", "pp", "j"]);
    let p = app.transient_popup().join("\n");
    // no stashes and no upstream: inapt
    assert!(p.contains("(z Stashes)") && p.contains("n Untracked files"), "{p}");
    assert!(p.contains("(fu Unpulled from @{upstream})") && p.contains("(pu Recent commits)"), "{p}");
    app.press("z").unwrap();
    assert_eq!(app.message(), "Inapt command: `z' (Use `C-g' to abort, `?' for help) [magit-jump-to-stashes]");
    assert_eq!(app.current_menu(), Some("status-jump-menu"));
}

#[test]
fn jump_to_sections() {
    let (_td, g) = repo();
    let mut app = App::new(g);
    press(&mut app, &["j", "s"]);
    assert_eq!(app.current_menu(), None);
    assert!(app.point_text().starts_with("Staged changes"), "{}", app.point_text());
    press(&mut app, &["j", "n"]);
    assert!(app.point_text().starts_with("Untracked files"), "{}", app.point_text());
    // two keys
    press(&mut app, &["j", "f"]);
    assert_eq!(app.current_menu(), Some("status-jump-menu"));
}

#[test]
fn prefix_argument_expands() {
    let (_td, g) = repo();
    g.git_interactive(&["stash", "push", "-q", "--", "s"]).unwrap();
    let mut app = App::new(g);
    // Stashes is collapsed by default
    press(&mut app, &["j", "C-u", "z"]);
    assert!(app.point_text().starts_with("Stashes"), "{}", app.point_text());
    let lines: Vec<String> = app.visible_lines().iter().map(|l| l.text.clone()).collect();
    let i = lines.iter().position(|l| l.starts_with("Stashes")).unwrap();
    assert!(lines[i + 1].starts_with("stash@{0}"), "{lines:?}");
}

#[test]
fn imenu() {
    let (_td, g) = repo();
    let mut app = App::new(g);
    app.script_answers(&["Unstaged changes", "modified   file.txt"]);
    press(&mut app, &["j", "j"]);
    assert_eq!(app.prompts(), ["Index item: ", "Index item: "]);
    let file = app.point_text().to_string();
    assert_eq!(file, "modified   file.txt", "{}", app.message());
}

#[test]
fn dispatch_j() {
    let (_td, g) = repo();
    let mut app = App::new(g);
    app.press("h").unwrap();
    assert!(app.transient_popup().join("\n").contains("j Jump to section"));
    press(&mut app, &["C-g", "l", "l"]);
    assert!(matches!(app.mode(), BufferMode::Log { .. }));
    app.press("h").unwrap();
    assert!(app.transient_popup().join("\n").contains("j Display status"));
    app.press("j").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
}
