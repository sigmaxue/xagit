//! Magit-parity fixes ported onto xagit 1.0.0:
//!   * `A` on a stash section is remapped to `magit-stash-pop`
//!   * `K` uses the full `magit-file-untrack` semantics (minibuffer with
//!     region, confirmation, "Already untracked", prefix --force)

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;

/// Ignore the user's git configuration, like the other 1.0.0 tests.
fn isolate() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("xagit-fixes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gitconfig");
        std::fs::write(&file, "[user]\n\tname = t\n\temail = t@t\n").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &file);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    });
}

fn repo() -> (TempDir, Git) {
    isolate();
    let (td, g, _) = repo_with_commit();
    (td, g)
}

/// Move point to the first visible line matching PRED (from the top).
fn goto_match(app: &mut App, pred: impl Fn(&str) -> bool) {
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
    panic!("no such line: {:?}", app.visible_lines().iter().map(|l| l.text.clone()).collect::<Vec<_>>());
}

fn tracked_files(g: &Git) -> Vec<String> {
    g.git_lines(&["ls-files", "-z"])
}

// --------------------------------------------------------------- `K` / untrack

/// magit-file-untrack with no region reads a tracked file in the minibuffer
/// (default = the file at point) and runs `git rm --cached` without a
/// separate confirmation (magit-files.el:666).
#[test]
fn untrack_reads_file_and_can_be_aborted() {
    let (_td, g) = repo();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g.clone());

    // C-g / empty at the minibuffer aborts, leaving the file tracked.
    goto_match(&mut app, |t| t.contains("file.txt"));
    app.script_keys(&["C-g"]);
    app.press("K").unwrap();
    assert!(app.prompts().last().unwrap().starts_with("Untrack file"), "{:?}", app.prompts());
    assert!(tracked_files(&g).contains(&"file.txt".to_string()));

    // Accepting the default (file at point) untracks but keeps the worktree.
    goto_match(&mut app, |t| t.contains("file.txt"));
    app.script_answers(&["file.txt"]);
    app.press("K").unwrap();
    assert!(!tracked_files(&g).contains(&"file.txt".to_string()));
    assert_eq!(read(&g, "file.txt"), "changed\n");
    assert!(g.untracked_files().contains(&"file.txt".to_string()));
}

/// With a region, magit-file-untrack confirms and refuses untracked files.
#[test]
fn untrack_region_confirms_and_can_be_declined() {
    let (_td, g) = repo();
    write(&g, "a.txt", "a\n");
    write(&g, "b.txt", "b\n");
    g.git_interactive(&["add", "a.txt", "b.txt"]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", "add a b"]).unwrap();
    write(&g, "a.txt", "changed a\n");
    write(&g, "b.txt", "changed b\n");
    let mut app = App::new(g.clone());

    // Select the two file lines, then decline the confirmation.
    goto_match(&mut app, |t| t.contains("a.txt"));
    press(&mut app, &["C-SPC", "C-n"]);
    app.script_answers(&["n"]);
    app.press("K").unwrap();
    assert_eq!(app.message(), "Abort");
    assert!(tracked_files(&g).contains(&"a.txt".to_string()));
    assert!(tracked_files(&g).contains(&"b.txt".to_string()));

    // Accepting untracks both files.
    app.refresh();
    goto_match(&mut app, |t| t.contains("a.txt"));
    press(&mut app, &["C-SPC", "C-n"]);
    app.script_answers(&["y"]);
    app.press("K").unwrap();
    assert!(!tracked_files(&g).contains(&"a.txt".to_string()));
    assert!(!tracked_files(&g).contains(&"b.txt".to_string()));
}

#[test]
fn untrack_with_prefix_uses_force() {
    let (_td, g) = repo();
    // A staged change makes a plain `git rm --cached` fail; with C-u the
    // command passes --force, like magit-file-untrack FORCE.
    write(&g, "file.txt", "staged\n");
    g.git_interactive(&["add", "file.txt"]).unwrap();
    let mut app = App::new(g.clone());
    app.script_answers(&["file.txt"]);
    press(&mut app, &["C-u", "K"]);
    assert!(!tracked_files(&g).contains(&"file.txt".to_string()));
    assert_eq!(read(&g, "file.txt"), "staged\n");
}

// ----------------------------------------------------- `A` on a stash = pop

#[test]
fn cherry_pick_on_stash_line_pops_instead() {
    let (_td, g) = repo();
    // A stash containing a real worktree change, on a clean worktree.
    write(&g, "file.txt", "stashed\n");
    g.git_interactive(&["stash", "push", "-q", "-m", "wip"]).unwrap();
    assert_eq!(g.stashes().len(), 1);

    let mut app = App::new(g.clone());
    // The Stashes section is hidden by default; expand it and move onto the
    // stash line, then press `A` (normally the cherry-pick menu).
    goto_match(&mut app, |t| t.trim().starts_with("Stashes"));
    app.press("TAB").unwrap();
    goto_match(&mut app, |t| t.starts_with("stash@{0}"));
    app.script_answers(&[""]); // read_stash: accept the default stash@{0}
    app.press("A").unwrap();

    // `A` popped the stash rather than opening the cherry-pick menu.
    assert!(g.stashes().is_empty(), "A should pop the stash: {:?}\n{}", g.stashes(), app.message());
    assert_eq!(read(&g, "file.txt"), "stashed\n");
}
