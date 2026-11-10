//! History: the minibuffer (M-p / M-n through a history list, the
//! editing keys), the history of each infix (transient-infix-read), the
//! history of each prefix with C-M-p / C-M-n, and saving both to
//! transient-history-file when quitting (transient-save-history).

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;
use xagit_lib::ui::keys::key_from_label;
use xagit_lib::ui::minibuf::{add_to_history, LineEditor, Step};
use xagit_lib::ui::transient::{read_history, write_history, History, HISTORY_LIMIT};

fn press_all(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

fn type_keys(ed: &mut LineEditor, keys: &[&str]) -> Step {
    let mut last = Step::Continue;
    for k in keys {
        last = ed.key(&key_from_label(k));
    }
    last
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn minibuffer_editing() {
    let mut ed = LineEditor::new(Vec::new(), None);
    type_keys(&mut ed, &["h", "e", "l", "o", "C-b", "l"]);
    assert_eq!(ed.text(), "hello");
    assert_eq!(ed.point(), 4);
    type_keys(&mut ed, &["C-a", "C-d", "H", "C-e", "SPC", "w", "o", "r", "l", "d"]);
    assert_eq!(ed.text(), "Hello world");
    type_keys(&mut ed, &["M-b", "C-k"]);
    assert_eq!(ed.text(), "Hello ");
    type_keys(&mut ed, &["DEL", "C-w"]);
    assert_eq!(ed.text(), "");
    type_keys(&mut ed, &["a", "-", "b", "C-a", "M-d"]);
    assert_eq!(ed.text(), "-b");
    assert_eq!(type_keys(&mut ed, &["RET"]), Step::Done("-b".into()));
    assert_eq!(type_keys(&mut ed, &["C-g"]), Step::Abort);
    assert_eq!(type_keys(&mut ed, &["C-t"]), Step::Message("C-t is undefined".into()));
}

#[test]
fn minibuffer_history() {
    let mut ed = LineEditor::new(strs(&["new", "old"]), Some("dflt".into()));
    type_keys(&mut ed, &["x"]);
    type_keys(&mut ed, &["M-p"]);
    assert_eq!(ed.text(), "new");
    assert_eq!(ed.point(), 3);
    type_keys(&mut ed, &["<up>"]);
    assert_eq!(ed.text(), "old");
    assert_eq!(type_keys(&mut ed, &["M-p"]), Step::Message("Beginning of history; no preceding item".into()));
    assert_eq!(ed.text(), "old");
    // back to the input typed
    type_keys(&mut ed, &["M-n", "<down>"]);
    assert_eq!(ed.text(), "x");
    // the future history: the default
    type_keys(&mut ed, &["M-n"]);
    assert_eq!(ed.text(), "dflt");
    assert_eq!(type_keys(&mut ed, &["M-n"]), Step::Message("End of defaults".into()));
    let mut ed = LineEditor::new(Vec::new(), None);
    assert_eq!(type_keys(&mut ed, &["M-n"]), Step::Message("End of history; no default available".into()));
    // add-to-history
    let mut h = strs(&["a", "b"]);
    add_to_history(&mut h, "b", false);
    assert_eq!(h, ["b", "a", "b"]);
    add_to_history(&mut h, "b", false);
    add_to_history(&mut h, "", false);
    assert_eq!(h, ["b", "a", "b"]);
    add_to_history(&mut h, "a", true);
    assert_eq!(h, ["a", "b"]);
}

#[test]
fn prompts_share_minibuffer_history() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    app.script_keys(&["f", "o", "o", "RET"]);
    app.script_keys(&["RET"]);
    press_all(&mut app, &["b", "c"]);
    assert_eq!(app.prompts(), ["New branch name", "Start from (empty = HEAD)"]);
    assert!(g.branches().contains(&"foo".to_string()), "{}", app.message());
    // empty inputs aren't added
    assert_eq!(app.minibuffer_history(), ["foo"]);
    // M-p gives the inputs of earlier prompts
    app.script_keys(&["M-p", "-", "2", "RET"]);
    app.script_keys(&["M-p", "M-p", "C-a", "C-k", "RET"]);
    press_all(&mut app, &["b", "c"]);
    assert!(g.branches().contains(&"foo-2".to_string()), "{}", app.message());
    assert_eq!(app.minibuffer_history(), ["foo-2", "foo"]);
}

#[test]
fn infix_history() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    // reading a value adds it to the infix's history
    app.script_answers(&["fix"]);
    press_all(&mut app, &["l", "-", "F"]);
    assert!(app.active_switches().contains(&"--grep=fix".to_string()), "{:?}", app.active_switches());
    press_all(&mut app, &["-", "F"]);
    assert!(!app.active_switches().iter().any(|a| a.starts_with("--grep=")));
    app.script_answers(&["feat"]);
    press_all(&mut app, &["-", "F"]);
    assert_eq!(app.infix_history("--grep="), ["feat", "fix"]);
    // M-p in the minibuffer: the values read before
    press_all(&mut app, &["-", "F"]);
    app.script_keys(&["M-p", "M-p", "RET"]);
    press_all(&mut app, &["-", "F"]);
    assert!(app.active_switches().contains(&"--grep=fix".to_string()), "{:?}", app.active_switches());
    assert_eq!(app.infix_history("--grep="), ["fix", "feat"]);
    // C-g reads nothing
    press_all(&mut app, &["-", "F"]);
    app.script_keys(&["x", "C-g"]);
    press_all(&mut app, &["-", "F"]);
    assert!(!app.active_switches().iter().any(|a| a.starts_with("--grep=")));
    assert_eq!(app.infix_history("--grep="), ["fix", "feat"]);
    // the infix of the same argument in another prefix shares the
    // history (magit-log:--author in l and in L of log buffers)
    app.script_answers(&["Jo"]);
    press_all(&mut app, &["-", "A", "l"]);
    assert_eq!(app.current_menu(), None);
    app.press("L").unwrap();
    assert_eq!(app.current_menu(), Some("log-refresh-menu"));
    press_all(&mut app, &["-", "A"]);
    app.script_keys(&["M-p", "RET"]);
    press_all(&mut app, &["-", "A"]);
    assert!(app.active_switches().contains(&"--author=Jo".to_string()), "{:?}", app.active_switches());
}

#[test]
fn prefix_history_keys() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    // the value of a prefix is added when one of its suffixes is invoked
    press_all(&mut app, &["c", "-", "e", "C-x", "s", "C-x", "C-k", "C-g", "c"]);
    assert_eq!(app.active_switches(), ["--verbose"]);
    // C-M-p / C-M-n are transient-history-prev / -next
    app.press("C-M-p").unwrap();
    assert_eq!(app.active_switches(), ["--allow-empty", "--verbose"]);
    app.press("C-M-n").unwrap();
    assert_eq!(app.active_switches(), ["--verbose"]);
    app.press("C-M-n").unwrap();
    assert_eq!(app.message(), "End of history");
    assert_eq!(app.prefix_history("commit-menu")[..2], [strs(&["--verbose"]), strs(&["--allow-empty", "--verbose"])]);
}

#[test]
fn history_file() {
    let mut h = History::default();
    h.prefixes.insert("commit-menu".into(), vec![strs(&["--verbose"]), strs(&[]), strs(&["--verbose"])]);
    let many: Vec<String> = (0..15).map(|i| format!("v{i}")).collect();
    h.infixes.insert("--grep=".into(), many.clone());
    h.infixes.insert("--author=".into(), strs(&["a\tb", "Jo <j@x>"]));
    let td = TempDir::new().unwrap();
    let f = td.path().join("cfg").join("transient-history");
    write_history(&f, &h).unwrap();
    let text = std::fs::read_to_string(&f).unwrap();
    assert!(text.starts_with("P\tcommit-menu\t--verbose\nP\tcommit-menu\nI\t--author=\tJo <j@x>\nI\t--grep=\tv0\n"), "{text}");
    let back = read_history(&f);
    assert_eq!(back.prefixes["commit-menu"], [strs(&["--verbose"]), strs(&[])]);
    assert_eq!(back.infixes["--author="], ["Jo <j@x>"]);
    assert_eq!(back.infixes["--grep="], many[..HISTORY_LIMIT]);
    // nothing written without history and file
    let none = td.path().join("none");
    write_history(&none, &History::default()).unwrap();
    assert!(!none.exists());
    assert_eq!(read_history(&none), History::default());
}

#[test]
fn saved_between_sessions() {
    let (td, g, _) = repo_with_commit();
    let file = td.path().join("transient-history");
    let mut app = App::new(g.clone());
    app.load_history(file.clone());
    app.script_answers(&["fix"]);
    press_all(&mut app, &["l", "-", "F", "C-g"]);
    press_all(&mut app, &["c", "-", "e", "C-x", "s", "C-g"]);
    app.save_history();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("I\t--grep=\tfix\n"), "{text}");
    assert!(text.contains("P\tcommit-menu\t--allow-empty\t--verbose\n"), "{text}");
    // the next session
    let mut app = App::new(g.clone());
    app.load_history(file.clone());
    assert_eq!(app.infix_history("--grep="), ["fix"]);
    app.script_keys(&["M-p", "RET"]);
    press_all(&mut app, &["l", "-", "F"]);
    assert!(app.active_switches().contains(&"--grep=fix".to_string()), "{:?}", app.active_switches());
    press_all(&mut app, &["C-g", "c", "C-x", "p"]);
    assert_eq!(app.active_switches(), ["--allow-empty", "--verbose"]);
    // values of this session come first
    press_all(&mut app, &["-", "a", "C-x", "s", "C-g"]);
    app.save_history();
    let text = std::fs::read_to_string(&file).unwrap();
    let commit: Vec<&str> = text.lines().filter(|l| l.starts_with("P\tcommit-menu")).collect();
    assert_eq!(commit[0], "P\tcommit-menu\t--all\t--allow-empty\t--verbose", "{text}");
}
