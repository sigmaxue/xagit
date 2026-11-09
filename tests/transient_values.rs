//! transient-common-commands "Value commands": C-x s (set), C-x C-s
//! (save to transient-values-file), C-x C-k (reset), C-x p / C-x n
//! (history), and the "s" / "w" suffixes of magit-diff-refresh and
//! magit-log-refresh.

mod common;

use common::*;
use ngit_lib::ui::app::App;

fn press_all(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

#[test]
fn set_for_this_session() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    press_all(&mut app, &["c", "-", "e"]);
    assert_eq!(app.active_switches(), ["--allow-empty", "--verbose"]);
    press_all(&mut app, &["C-x", "s"]);
    assert_eq!(app.current_menu(), Some("commit-menu"), "the transient stays");
    press_all(&mut app, &["C-g", "c"]);
    assert_eq!(app.active_switches(), ["--allow-empty", "--verbose"]);
    // without setting, toggling is forgotten
    press_all(&mut app, &["-", "v", "C-g", "c"]);
    assert_eq!(app.active_switches(), ["--allow-empty", "--verbose"]);
    // a new session starts with the default value
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    assert_eq!(app.active_switches(), ["--verbose"]);
}

#[test]
fn save_and_reset() {
    let (td, g, _) = repo_with_commit();
    let file = td.path().join("cfg").join("transient-values");
    let mut app = App::new(g.clone());
    app.load_values(file.clone());
    press_all(&mut app, &["c", "-", "e", "C-x", "C-s"]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "commit-menu\t--allow-empty\t--verbose\n");
    let mut app = App::new(g.clone());
    app.load_values(file.clone());
    app.press("c").unwrap();
    assert_eq!(app.active_switches(), ["--allow-empty", "--verbose"]);
    press_all(&mut app, &["C-x", "C-k"]);
    assert_eq!(app.active_switches(), ["--verbose"]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    press_all(&mut app, &["C-g", "c"]);
    assert_eq!(app.active_switches(), ["--verbose"]);
}

#[test]
fn history() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    press_all(&mut app, &["c", "-", "e", "C-x", "s", "C-x", "C-k", "C-g", "c"]);
    assert_eq!(app.active_switches(), ["--verbose"]);
    press_all(&mut app, &["C-x", "p"]);
    assert_eq!(app.active_switches(), ["--allow-empty", "--verbose"]);
    press_all(&mut app, &["C-x", "p"]);
    assert_eq!(app.message(), "End of history");
    press_all(&mut app, &["C-x", "n"]);
    assert_eq!(app.active_switches(), ["--verbose"]);
    press_all(&mut app, &["C-x", "n"]);
    assert_eq!(app.message(), "End of history");
    // C-g after C-x only cancels the key sequence
    press_all(&mut app, &["C-x", "C-g"]);
    assert_eq!(app.current_menu(), Some("commit-menu"));
    press_all(&mut app, &["C-x", "z"]);
    assert_eq!(app.message(), "C-x z is undefined");
}

#[test]
fn diff_refresh_set_and_save_defaults() {
    let (td, g, _) = repo_with_commit();
    let file = td.path().join("transient-values");
    let mut app = App::new(g.clone());
    app.load_values(file.clone());
    press_all(&mut app, &["D", "-", "w", "s"]);
    assert_eq!(app.current_menu(), None, "s refreshes the buffer and exits");
    app.press("D").unwrap();
    assert_eq!(app.active_switches(), ["--ignore-all-space", "--ignore-submodules=none", "--no-ext-diff"]);
    assert!(!file.exists(), "s does not save");
    press_all(&mut app, &["-", "b", "w"]);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "magit-diff:magit-status-mode\t--ignore-all-space\t--ignore-space-change\t--ignore-submodules=none\t--no-ext-diff\n"
    );
    let mut app = App::new(g.clone());
    app.load_values(file);
    app.press("D").unwrap();
    assert_eq!(
        app.active_switches(),
        ["--ignore-all-space", "--ignore-space-change", "--ignore-submodules=none", "--no-ext-diff"]
    );
}

#[test]
fn log_set_defaults() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    press_all(&mut app, &["l", "-", "r", "C-x", "s", "C-g"]);
    assert_eq!(app.log_args, ["--decorate", "--graph", "--reverse", "-n256"]);
    // the log buffer's L s: the buffer's arguments and the default
    press_all(&mut app, &["l", "l", "L", "-", "g", "s"]);
    assert_eq!(app.log_args, ["--decorate", "--reverse", "-n256"]);
    press_all(&mut app, &["L"]);
    assert_eq!(app.active_switches(), ["--decorate", "--reverse", "-n256"]);
}
