//! transient-common-commands "Value commands": C-x s (set), C-x C-s
//! (save to transient-values-file), C-x C-k (reset), C-x p / C-x n
//! (history), and the "s" / "w" suffixes of magit-diff-refresh and
//! magit-log-refresh.

mod common;

use common::*;
use ngit_lib::ui::app::App;
use ngit_lib::ui::options::Options;

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
    press_all(&mut app, &["C-x", "y"]);
    assert_eq!(app.message(), "Unbound suffix: `C-x y' (Use `C-g' to abort, `?' for help) [nil]");
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

#[test]
fn status_log_defaults_are_per_mode() {
    let (td, g, _) = repo_with_commit();
    let file = td.path().join("transient-values");
    let mut app = App::new(g.clone());
    app.load_values(file.clone());
    press_all(&mut app, &["L", "-", "g", "w"]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "magit-log:magit-status-mode\t--decorate\t--graph\t-n256\n");
    assert_eq!(app.log_args, ["--graph", "-n256", "--decorate"], "magit-log-mode is not changed");
    let mut app = App::new(g.clone());
    app.load_values(file);
    app.press("L").unwrap();
    assert_eq!(app.active_switches(), ["--decorate", "--graph", "-n256"]);
}

const HELP: &str = "(Use `C-g' to abort, `?' for help)";

#[test]
fn unbound_suffix() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    // a key bound in the buffer but not a suffix (transient--do-warn):
    // the menu stays
    app.press("M-1").unwrap();
    assert_eq!(app.message(), format!("Unbound suffix: `M-1' {HELP} [magit-section-show-level-1-all]"));
    assert_eq!(app.current_menu(), Some("commit-menu"));
    let suffixes = app.menu_keys();
    for (k, cmd) in [("p", "magit-section-backward"), ("k", "magit-discard"), ("b", "magit-branch"), ("C-n", "next-line")] {
        assert!(!suffixes.contains(&k.to_string()), "{suffixes:?}");
        app.press(k).unwrap();
        assert_eq!(app.message(), format!("Unbound suffix: `{k}' {HELP} [{cmd}]"));
    }
    assert_eq!(app.current_menu(), Some("commit-menu"));
    // a key bound nowhere
    app.press("C-o").unwrap();
    assert_eq!(app.message(), "C-o is undefined");
    // digit-argument stays transient (M-5 isn't a section command)
    app.press("M-5").unwrap();
    assert_eq!(app.current_menu(), Some("commit-menu"));
    assert_eq!(app.message(), "C-u 5-");
    app.press("C-g").unwrap();
    // C-q: transient-quit-all
    app.press("C-q").unwrap();
    assert_eq!(app.current_menu(), None);
    // in a file buffer only its own bindings count
    write(&g, "file.txt", "changed\n");
    app.refresh();
    for _ in 0..app.visible_lines().len() {
        app.press("C-p").unwrap();
    }
    for _ in 0..app.visible_lines().len() {
        if app.point_text() == "modified   file.txt" {
            break;
        }
        app.press("C-n").unwrap();
    }
    app.press("RET").unwrap();
    assert_eq!(app.header_line(), "file.txt");
    press_all(&mut app, &["C-c", "M-g"]);
    assert!(app.current_menu().is_some());
    assert!(!app.menu_keys().contains(&"k".to_string()));
    app.press("k").unwrap();
    assert_eq!(app.message(), "k is undefined");
    app.press("C-p").unwrap();
    assert_eq!(app.message(), format!("Unbound suffix: `C-p' {HELP} [previous-line]"));
    app.press("M-x").unwrap();
    assert_eq!(app.message(), format!("Unbound suffix: `M-x' {HELP} [execute-extended-command]"));
    // M-x is global: file buffers have it
    app.press("C-g").unwrap();
    app.script_answers(&["magit-commit"]);
    app.press("M-x").unwrap();
    assert_eq!(app.current_menu(), Some("commit-menu"));
}

const HELP_LINES: [&str; 2] =
    ["Type a <KEY> to show help for that suffix command, or ? to show manual.", "Type C-g to exit help."];

fn in_help_mode(app: &App) -> bool {
    let popup = app.transient_popup();
    HELP_LINES.iter().all(|l| popup.iter().any(|r| r == l))
}

/// The line of the text view shown first.
fn shown_line(app: &App) -> String {
    let (_, text) = app.text_view().expect("help shown");
    text.lines().nth(app.text_view_scroll()).unwrap_or("").trim().to_string()
}

#[test]
fn transient_help() {
    let (td, g, head) = repo_with_commit();
    let dir = td.path().join("lisp");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("magit.info"),
        "This is magit.info.\n\u{1f}\nFile: magit.info,  Node: Top,  Next: Introduction,  Up: (dir)\n\nMagit\n*****\n\
         \u{1f}\nFile: magit.info,  Node: Initiating a Commit,  Next: Editing Commit Messages,  Up: Committing\n\n\
         6.1.1 Initiating a Commit\n-------------------------\n\u{1f}\nTag Table:\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("magit-commit.el"),
        ";;; magit-commit.el\n\n(defun magit-commit-create (&optional args)\n  \"Create a new commit on `HEAD'.\n\
         With a prefix argument, amend to the commit at `HEAD' instead.\n\\n\\n(fn &optional ARGS)\"\n  (interactive))\n",
    )
    .unwrap();
    let mut app = App::new(g.clone());
    app.set_help_dirs(vec![dir.clone()]);
    app.press("c").unwrap();
    assert!(!in_help_mode(&app));
    // ?: help-mode, the transient stays
    app.press("?").unwrap();
    assert_eq!(app.current_menu(), Some("commit-menu"));
    assert!(in_help_mode(&app), "{:?}", app.transient_popup());
    // ? again: the info-manual of the prefix; the transient is suspended
    app.press("?").unwrap();
    assert_eq!(app.current_menu(), None);
    assert!(app.transient_popup().is_empty());
    assert_eq!(app.text_view().map(|v| v.0), Some("*info* (magit) Initiating a Commit"));
    assert_eq!(shown_line(&app), "6.1.1 Initiating a Commit");
    assert_eq!(app.message(), "Type `q' to resume transient command.");
    // q: transient-resume
    app.press("q").unwrap();
    assert_eq!(app.current_menu(), Some("commit-menu"));
    assert!(app.text_view().is_none());
    assert!(!in_help_mode(&app), "help-mode ended");
    // an infix: the manpage of the prefix at its argument
    press_all(&mut app, &["C-h", "-", "v"]);
    assert_eq!(app.text_view().map(|v| v.0), Some("*Man git-commit*"));
    assert!(shown_line(&app).starts_with("-v, --verbose"), "{}", shown_line(&app));
    // the help scrolls
    let at = app.text_view_scroll();
    app.press("C-n").unwrap();
    assert_eq!(app.text_view_scroll(), at + 1);
    press_all(&mut app, &["q", "?", "-", "A"]);
    assert!(shown_line(&app).starts_with("--author=<author>"), "{}", shown_line(&app));
    assert_eq!(app.active_switches(), ["--verbose"], "the infix isn't invoked");
    // a suffix: its documentation
    press_all(&mut app, &["q", "?", "c"]);
    assert_eq!(app.text_view().map(|v| v.0), Some("*Help*"));
    assert_eq!(
        app.text_view().unwrap().1,
        "magit-commit-create is an interactive Lisp function in `magit-commit.el'.\n\n\
         (magit-commit-create &optional ARGS)\n\nCreate a new commit on `HEAD'.\n\
         With a prefix argument, amend to the commit at `HEAD' instead.\n"
    );
    assert_eq!(g.rev_parse("HEAD").unwrap(), head, "no commit was made");
    // one not found in the sources
    press_all(&mut app, &["q", "?", "e"]);
    assert_eq!(app.text_view().unwrap().1, "magit-commit-extend is an interactive command.\n\nExtend.\n");
    // C-g leaves help-mode; unbound keys are transient-undefined
    press_all(&mut app, &["q", "?", "p"]);
    assert_eq!(app.message(), format!("Unbound suffix: `p' {HELP} [magit-section-backward]"));
    assert!(in_help_mode(&app));
    app.press("C-g").unwrap();
    assert!(!in_help_mode(&app));
    assert_eq!(app.current_menu(), Some("commit-menu"));
    // the value commands
    press_all(&mut app, &["?", "C-x", "s"]);
    assert!(app.text_view().unwrap().1.starts_with("transient-set is an interactive command.\n\nSet."));
    press_all(&mut app, &["q", "C-g"]);
    assert_eq!(app.current_menu(), None);
    // a sub-prefix: its help; a prefix with a man-page and no info-manual
    press_all(&mut app, &["h", "?", "c"]);
    assert_eq!(app.text_view().map(|v| v.0), Some("*info* (magit) Initiating a Commit"));
    press_all(&mut app, &["q", "?", "b"]);
    assert_eq!(app.text_view().map(|v| v.0), Some("*Man git-branch*"));
    assert_eq!(app.text_view_scroll(), 0);
    app.press("q").unwrap();
    assert_eq!(app.current_menu(), Some("dispatch"));
    // the manual not found: a message, the transient stays
    app.set_help_dirs(Vec::new());
    press_all(&mut app, &["?", "?"]);
    assert_eq!(app.message(), "transient-help: Info file magit does not exist");
    assert_eq!(app.current_menu(), Some("dispatch"));
    assert!(app.text_view().is_none());
}

fn has_row(app: &App, row: &str) -> bool {
    app.transient_popup().iter().any(|r| r == row)
}

#[test]
fn suspend_and_resume() {
    let (td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    press_all(&mut app, &["c", "-", "e"]);
    // C-z: transient-suspend, the stack and its value are saved
    app.press("C-z").unwrap();
    assert_eq!(app.current_menu(), None);
    assert!(app.transient_popup().is_empty());
    // M-x transient-resume
    app.script_answers(&["transient-resume"]);
    app.press("M-x").unwrap();
    assert_eq!(app.prompts().last().map(String::as_str), Some("M-x "));
    assert_eq!(app.current_menu(), Some("commit-menu"));
    assert_eq!(app.active_switches(), ["--allow-empty", "--verbose"]);
    // in a transient M-x isn't a suffix
    app.press("M-x").unwrap();
    assert_eq!(app.message(), format!("Unbound suffix: `M-x' {HELP} [execute-extended-command]"));
    // transient-show: the menu stays (it is always shown)
    app.press("C-t").unwrap();
    assert_eq!(app.current_menu(), Some("commit-menu"));
    app.press("C-g").unwrap();
    app.script_answers(&["transient-resume"]);
    app.press("M-x").unwrap();
    assert_eq!(app.message(), "No suspended transient command");
    // C-z in an incomplete sequence (transient-sticky-map)
    press_all(&mut app, &["c", "C-x", "C-z"]);
    assert_eq!(app.current_menu(), None);
    // a new transient replaces the suspended one
    press_all(&mut app, &["b", "C-g"]);
    app.script_answers(&["transient-resume"]);
    app.press("M-x").unwrap();
    assert_eq!(app.message(), "No suspended transient command");
    assert_eq!(app.current_menu(), None);
    // the prefixes are commands
    app.script_answers(&["magit-branch"]);
    app.press("M-x").unwrap();
    assert_eq!(app.current_menu(), Some("branch-menu"));
    app.press("C-g").unwrap();
    app.script_answers(&["frobnicate"]);
    app.press("M-x").unwrap();
    assert_eq!(app.message(), "[No match]");
    // help for transient-suspend
    let dir = td.path().join("lisp");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("transient.el"),
        "(defun transient-suspend ()\n  \"Suspend the current transient.\nIt can later be resumed using `transient-resume'.\"\n  (interactive))\n",
    )
    .unwrap();
    app.set_help_dirs(vec![dir]);
    press_all(&mut app, &["c", "?", "C-z"]);
    assert_eq!(
        app.text_view().unwrap().1,
        "transient-suspend is an interactive Lisp function in `transient.el'.\n\n(transient-suspend)\n\n\
         Suspend the current transient.\nIt can later be resumed using `transient-resume'.\n"
    );
    app.press("q").unwrap();
    assert_eq!(app.current_menu(), Some("commit-menu"));
}

#[test]
fn common_commands() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    assert!(!has_row(&app, "  Sticky commands"));
    // shown after C-x
    app.press("C-x").unwrap();
    for row in [
        "  Value commands",
        "  C-x s    Set",
        "  Sticky commands",
        "  C-g      Quit prefix or transient",
        "  C-q      Quit transient stack",
        "  C-z      Suspend transient stack",
        "  Customize",
        "  C-x t    Show common permanently",
    ] {
        assert!(has_row(&app, row), "{row}: {:?}", app.transient_popup());
    }
    // C-x t: transient-toggle-common, the transient stays
    app.press("t").unwrap();
    assert_eq!(app.current_menu(), Some("commit-menu"));
    assert!(has_row(&app, "  Sticky commands"));
    assert!(has_row(&app, "  C-x t    Hide common commands"));
    press_all(&mut app, &["C-g", "b"]);
    assert!(has_row(&app, "  Value commands"), "for all transients");
    press_all(&mut app, &["C-x", "t"]);
    assert!(!has_row(&app, "  Value commands"));
    app.press("C-g").unwrap();
    // transient-show-common-commands
    let (o, e) = Options::parse("transient-show-common-commands t");
    assert!(e.is_empty());
    app.set_options(o);
    app.press("c").unwrap();
    assert!(has_row(&app, "  C-x t    Hide common commands"));
}
