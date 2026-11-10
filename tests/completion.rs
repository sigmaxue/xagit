//! Completion in the minibuffer (completing-read): TAB completes the input
//! (the basic, then the partial-completion style), `?` and a TAB without
//! progress list the possible completions, RET with require-match only
//! exits with a candidate; branches and commits are read with
//! magit-revision-history.

mod common;

use common::*;
use xagit_lib::ui::app::App;
use xagit_lib::ui::keys::key_from_label;
use xagit_lib::ui::minibuf::{completion_rows, Completion, LineEditor, Step};

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

fn editor(cands: &[&str], require_match: bool) -> LineEditor {
    LineEditor::new(Vec::new(), None).with_completion(Completion {
        candidates: strs(cands),
        require_match,
        multiple: false,
    })
}

const BRANCHES: &[&str] = &["master", "main", "maint", "feature/a-b", "origin/master"];

#[test]
fn tab_completes() {
    let mut ed = editor(BRANCHES, false);
    // the common prefix
    assert_eq!(type_keys(&mut ed, &["m", "TAB"]), Step::Continue);
    assert_eq!(ed.text(), "ma");
    assert_eq!(ed.completions(), None);
    // no progress: the possible completions are shown
    type_keys(&mut ed, &["TAB"]);
    assert_eq!(ed.text(), "ma");
    assert_eq!(ed.completions().unwrap(), ["master", "main", "maint"]);
    // completed to a candidate that isn't the only one
    assert_eq!(type_keys(&mut ed, &["i", "TAB"]), Step::Message("[Complete, but not unique]".into()));
    assert_eq!(ed.text(), "main");
    assert_eq!(ed.completions().unwrap(), ["main", "maint"]);
    // the only completion
    assert_eq!(type_keys(&mut ed, &["t", "TAB"]), Step::Message("[Sole completion]".into()));
    assert_eq!(ed.completions(), None);
    assert_eq!(type_keys(&mut ed, &["x", "TAB"]), Step::Message("[No match]".into()));
    assert_eq!(ed.text(), "maintx");
    // without require-match RET exits with anything
    assert_eq!(type_keys(&mut ed, &["RET"]), Step::Done("maintx".into()));

    let mut ed = editor(BRANCHES, false);
    assert_eq!(type_keys(&mut ed, &["f", "TAB"]), Step::Continue);
    assert_eq!(ed.text(), "feature/a-b");
    // TAB again on an exact, non-unique input shows the completions
    let mut ed = editor(BRANCHES, false);
    type_keys(&mut ed, &["m", "a", "i", "n", "TAB"]);
    assert_eq!(ed.completions(), None);
    type_keys(&mut ed, &["TAB"]);
    assert_eq!(ed.completions().unwrap(), ["main", "maint"]);
}

#[test]
fn partial_completion() {
    let mut ed = editor(BRANCHES, false);
    type_keys(&mut ed, &["o", "/", "m", "TAB"]);
    assert_eq!(ed.text(), "origin/master");
    let mut ed = editor(BRANCHES, false);
    type_keys(&mut ed, &["f", "-", "b", "TAB"]);
    assert_eq!(ed.text(), "feature/a-b");
    // `*` matches anything
    let mut ed = editor(BRANCHES, false);
    type_keys(&mut ed, &["*", "a", "s", "t", "TAB"]);
    assert_eq!(ed.text(), "*ast");
    assert_eq!(ed.completions().unwrap(), ["master", "origin/master"]);
}

#[test]
fn completion_help() {
    let mut ed = editor(BRANCHES, false);
    assert_eq!(type_keys(&mut ed, &["?"]), Step::Continue);
    assert_eq!(ed.completions().unwrap().len(), BRANCHES.len());
    type_keys(&mut ed, &["m", "a", "s", "?"]);
    assert_eq!(ed.text(), "mas");
    assert_eq!(ed.completions().unwrap(), ["master"]);
    assert_eq!(type_keys(&mut ed, &["z", "?"]), Step::Message("[No match]".into()));
    assert_eq!(ed.completions(), None);
    // without a completion table TAB and ? are not completion commands
    let mut ed = LineEditor::new(Vec::new(), None);
    assert_eq!(type_keys(&mut ed, &["?"]), Step::Continue);
    assert_eq!(ed.text(), "?");
    assert_eq!(type_keys(&mut ed, &["TAB"]), Step::Message("TAB is undefined".into()));
    // the *Completions* buffer
    let rows = completion_rows(&strs(&["bb", "a", "ccc"]), 12);
    assert_eq!(rows, ["3 possible completions:", "a    bb", "ccc"]);
    assert_eq!(completion_rows(&strs(&["x"]), 0), ["1 possible completions:", "x"]);
}

#[test]
fn require_match() {
    let mut ed = editor(BRANCHES, true);
    // completed to the only completion: exits
    assert_eq!(type_keys(&mut ed, &["m", "a", "s", "RET"]), Step::Done("master".into()));
    // completed to a candidate that isn't the only one: confirm
    let mut ed = editor(BRANCHES, true);
    assert_eq!(type_keys(&mut ed, &["m", "a", "i", "RET"]), Step::Message("[Confirm]".into()));
    assert_eq!(ed.text(), "main");
    assert_eq!(type_keys(&mut ed, &["RET"]), Step::Done("main".into()));
    // not a candidate: keeps reading
    let mut ed = editor(BRANCHES, true);
    assert_eq!(type_keys(&mut ed, &["z", "RET"]), Step::Message("[No match]".into()));
    assert_eq!(type_keys(&mut ed, &["DEL", "RET"]), Step::Done(String::new()));
    let mut ed = editor(BRANCHES, true);
    assert_eq!(type_keys(&mut ed, &["m", "RET"]), Step::Continue);
    assert_eq!(ed.text(), "ma");
    assert_eq!(ed.completions(), None);
    assert_eq!(type_keys(&mut ed, &["RET"]), Step::Continue);
    assert_eq!(ed.completions().unwrap(), ["master", "main", "maint"]);
}

#[test]
fn completing_multiple() {
    let mut ed = LineEditor::new(Vec::new(), None).with_completion(Completion {
        candidates: strs(&["a.txt", "b.txt", "dir/c.txt"]),
        require_match: true,
        multiple: true,
    });
    type_keys(&mut ed, &["a", "TAB", ",", "SPC", "d", "TAB"]);
    assert_eq!(ed.text(), "a.txt, dir/c.txt");
    // RET exits (each file is checked by the command)
    assert_eq!(type_keys(&mut ed, &[",", "x", "RET"]), Step::Done("a.txt, dir/c.txt,x".into()));
}

#[test]
fn revision_history() {
    let (_td, g, _) = repo_with_commit();
    let main = g.current_branch().unwrap();
    g.branch_create("feature", None).unwrap();
    let mut app = App::new(g.clone());
    // completing the branch to check out
    app.script_keys(&["f", "e", "TAB", "RET"]);
    for k in ["b", "b"] {
        app.press(k).unwrap();
    }
    assert_eq!(app.prompts(), ["Checkout branch/revision"]);
    assert_eq!(g.current_branch().as_deref(), Some("feature"), "{}", app.message());
    assert_eq!(app.revision_history(), ["feature"]);
    assert!(app.minibuffer_history().is_empty());
    // a name read with minibuffer-history, the start with
    // magit-revision-history
    app.script_keys(&["n", "e", "w", "RET"]);
    app.script_keys(&["M-p", "RET"]);
    for k in ["b", "c"] {
        app.press(k).unwrap();
    }
    assert_eq!(app.minibuffer_history(), ["new"]);
    assert_eq!(app.revision_history(), ["feature"]);
    assert_eq!(g.current_branch().as_deref(), Some("new"), "{}", app.message());
    // back to the main branch, typed
    let keys: Vec<String> = main.chars().map(|c| c.to_string()).collect();
    let mut keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    keys.push("RET");
    app.script_keys(&keys);
    for k in ["b", "b"] {
        app.press(k).unwrap();
    }
    assert_eq!(g.current_branch().as_deref(), Some(main.as_str()), "{}", app.message());
    assert_eq!(app.revision_history(), [main.clone(), "feature".into()]);
    // an empty input adds the default (read-from-minibuffer): the
    // previous branch
    app.script_keys(&["RET"]);
    for k in ["l", "o"] {
        app.press(k).unwrap();
    }
    assert_eq!(app.prompts().last().unwrap(), "Log rev,s");
    assert_eq!(app.revision_history(), ["new".to_string(), main, "feature".into()]);
}
