//! magit-notes (T).

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};

/// Ignore the user's global configuration (C and D set it); the editor
/// writes "my note".
fn isolate() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("xagit-notes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gitconfig");
        std::fs::write(&file, "[user]\n\tname = t\n\temail = t@t\n").unwrap();
        let editor = dir.join("editor.sh");
        std::fs::write(&editor, "#!/bin/sh\necho 'my note' > \"$1\"\n").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &file);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_EDITOR", format!("sh {}", editor.display()));
        file
    })
    .clone()
}

fn repo() -> (TempDir, Git) {
    isolate();
    let (td, g, _) = repo_with_commit();
    g.git_interactive(&["config", "--unset", "core.editor"]).unwrap();
    (td, g)
}

fn note(g: &Git, r: &str) -> Option<String> {
    g.git_string(&["notes", &format!("--ref={r}"), "show", "HEAD"]).ok()
}

#[test]
fn menu() {
    let (_td, g) = repo();
    let mut app = App::new(g);
    app.press("T").unwrap();
    assert_eq!(app.menu_keys(), ["c", "d", "C", "D", "-n", "-r", "-s", "T", "r", "m", "p"]);
    let p = app.transient_popup().join("\n");
    assert!(p.contains("c core.notesRef unset") && p.contains("D notes.displayRef unset"), "{p}");
    assert!(p.contains("-r Manipulate ref") && p.contains("-s Merge strategy"), "{p}");
}

/// Type TEXT into the built-in text buffer and save it (C-c C-c).
fn edit(app: &mut App, text: &str) {
    assert_eq!(app.mode(), &BufferMode::TextEdit, "{}", app.message());
    app.text_edit_set(text);
    press(app, &["C-c", "C-c"]);
}

#[test]
fn edit_and_remove() {
    let (_td, g) = repo();
    let mut app = App::new(g.clone());
    app.script_answers(&["HEAD"]);
    press(&mut app, &["T", "T"]);
    assert_eq!(app.prompts(), ["Edit notes"]);
    assert_eq!(app.text_edit_title().as_deref(), Some("NOTES_EDITMSG"));
    edit(&mut app, "my note\n");
    assert_eq!(note(&g, "commits").as_deref(), Some("my note"), "{}", app.message());
    app.script_answers(&["HEAD"]);
    press(&mut app, &["T", "r"]);
    assert_eq!(app.prompts()[1], "Remove notes");
    assert_eq!(note(&g, "commits"), None);
}

#[test]
fn manipulate_another_ref() {
    let (_td, g) = repo();
    g.git_interactive(&["notes", "--ref=review", "add", "-m", "old", "HEAD"]).unwrap();
    let mut app = App::new(g.clone());
    // only existing notes refs, qualified
    app.script_answers(&["nope"]);
    press(&mut app, &["T", "-", "r"]);
    assert_eq!(app.message(), "[No match]");
    app.script_answers(&["review", "HEAD"]);
    press(&mut app, &["-", "r", "T"]);
    assert_eq!(app.prompts()[1], "--ref=");
    // the current note is pre-filled, then written back to that same ref
    assert_eq!(app.text_edit_text().as_deref(), Some("old\n"));
    edit(&mut app, "my note\n");
    assert_eq!(note(&g, "refs/notes/review").as_deref(), Some("my note"), "{}", app.message());
    assert_eq!(note(&g, "commits"), None);
}

#[test]
fn configure_refs() {
    let (_td, g) = repo();
    g.git_interactive(&["notes", "--ref=review", "add", "-m", "r", "HEAD"]).unwrap();
    g.git_interactive(&["notes", "add", "-m", "c", "HEAD"]).unwrap();
    let mut app = App::new(g.clone());
    let local = |k: &str| g.git_lines(&["config", "--local", "--get-all", k]);
    // c: read when unset (the transient stays), unset when set
    app.script_answers(&["review"]);
    press(&mut app, &["T", "c"]);
    assert_eq!(app.prompts(), ["Set local core.notesRef"]);
    assert_eq!(local("core.notesRef"), ["refs/notes/review"]);
    assert_eq!(app.current_menu(), Some("notes-menu"));
    assert!(app.transient_popup().join("\n").contains("c core.notesRef refs/notes/review"));
    app.press("c").unwrap();
    assert!(local("core.notesRef").is_empty());
    // d: several, the current ones the initial input
    app.script_answers(&["commits,review"]);
    app.press("d").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "Set local notes.displayRef: ");
    assert_eq!(local("notes.displayRef"), ["refs/notes/commits", "refs/notes/review"]);
    // C: the global value
    app.script_answers(&["review"]);
    app.press("C").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "Set global core.notesRef");
    let global = std::fs::read_to_string(isolate()).unwrap();
    assert!(global.contains("notesRef = refs/notes/review"), "{global}");
    app.press("C").unwrap();
    assert!(!std::fs::read_to_string(isolate()).unwrap().contains("notesRef"));
}

#[test]
fn prune_dry_run() {
    let (_td, g) = repo();
    // a note about an object that isn't reachable
    write(&g, "x", "x\n");
    let obj = g.git_string(&["hash-object", "-w", "x"]).unwrap();
    g.git_interactive(&["notes", "add", "-m", "dangling", &obj]).unwrap();
    std::fs::remove_file(g.gitdir.join("objects").join(&obj[..2]).join(&obj[2..])).unwrap();
    let mut app = App::new(g.clone());
    press(&mut app, &["T", "-", "n", "p"]);
    let (_, text) = app.text_view().unwrap();
    assert!(text.contains(&obj), "{text}");
    assert!(g.git_string(&["notes", "list"]).unwrap().contains(&obj));
    press(&mut app, &["T", "p"]);
    assert!(!g.git_string(&["notes", "list"]).unwrap().contains(&obj), "{}", app.message());
}

#[test]
fn merging() {
    let (_td, g) = repo();
    g.git_interactive(&["notes", "add", "-m", "a", "HEAD"]).unwrap();
    g.git_interactive(&["notes", "--ref=other", "add", "-m", "b", "HEAD"]).unwrap();
    let mut app = App::new(g.clone());
    app.script_answers(&["refs/notes/other"]);
    press(&mut app, &["T", "m"]);
    assert_eq!(app.prompts(), ["Merge reference"]);
    // a conflict: the merge actions
    press(&mut app, &["T"]);
    assert_eq!(app.current_menu(), Some("notes-menu@progress"), "{}", app.message());
    assert_eq!(app.menu_keys(), ["d", "C", "D", "c", "a"]);
    app.press("a").unwrap();
    press(&mut app, &["T"]);
    assert_eq!(app.current_menu(), Some("notes-menu"));
}

/// An empty buffer removes the note; C-c C-k leaves the repository alone.
#[test]
fn clear_or_cancel_a_note() {
    let (_td, g) = repo();
    g.git_interactive(&["notes", "add", "-m", "keep me", "HEAD"]).unwrap();
    let mut app = App::new(g.clone());
    // cancel: nothing changes
    app.script_answers(&["HEAD"]);
    press(&mut app, &["T", "T"]);
    assert_eq!(app.text_edit_text().as_deref(), Some("keep me\n"));
    app.text_edit_set("changed\n");
    press(&mut app, &["C-c", "C-k"]);
    assert_eq!(note(&g, "commits").as_deref(), Some("keep me"));
    // empty text on save removes the note
    app.script_answers(&["HEAD"]);
    press(&mut app, &["T", "T"]);
    edit(&mut app, "");
    assert_eq!(note(&g, "commits"), None, "{}", app.message());
}
