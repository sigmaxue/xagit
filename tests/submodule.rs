//! magit-submodule (o) and magit-list-submodules.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

/// A superproject and a repository TMP/lib (one commit, file a).
fn setup() -> (TempDir, Git, TempDir, String) {
    let (td, g, _) = repo_with_commit();
    let src = TempDir::new().unwrap();
    let lib = src.path().join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    git_init(&lib);
    std::fs::write(lib.join("a"), "x\n").unwrap();
    let sg = Git::discover(&lib).unwrap();
    sg.git_interactive(&["add", "."]).unwrap();
    sg.commit(Some("initial"), &[]).unwrap();
    (td, g, src, lib.to_string_lossy().into_owned())
}

/// `o a URL RET RET`: added at "lib" with the name "lib", committed.
fn add(app: &mut App, g: &Git, url: &str) {
    app.script_answers(&[url]);
    app.script_keys(&["RET"]);
    app.script_keys(&["RET"]);
    press(app, &["o", "a"]);
    g.commit(Some("add lib"), &[]).unwrap_or_else(|e| panic!("{e:#}\n{}\n{:?}\n{:?}", app.message(), app.prompts(), g.git_string(&["status", "--short"])));
}

#[test]
fn menu() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g);
    app.press("o").unwrap();
    assert_eq!(app.menu_keys(), ["-f", "-r", "-N", "-C", "-R", "-M", "-U", "a", "r", "p", "u", "s", "d", "k", "l", "f"]);
    let p = app.transient_popup().join("\n");
    assert!(p.contains("Add            git submodule add [--force]"), "{p}");
    assert!(p.contains("Populate       git submodule update --init [--recursive]"), "{p}");
    assert!(p.contains("Unpopulate     git submodule deinit [--force]"), "{p}");
}

#[test]
fn add_reads_url_path_and_name() {
    let (_td, g, _src, url) = setup();
    let mut app = App::new(g.clone());
    add(&mut app, &g, &url);
    let prompts = app.prompts();
    assert_eq!(prompts[..3], ["Add submodule (remote url)", "Add submodules at path: ", "Submodule name"]);
    assert!(read(&g, ".gitmodules").contains("[submodule \"lib\"]"), "{}", app.message());
    assert!(exists(&g, "lib/a"));
    // the gitdir is in the superproject's
    assert!(g.gitdir.join("modules/lib").is_dir());
    assert!(std::fs::metadata(g.toplevel.join("lib/.git")).unwrap().is_file());
}

#[test]
fn add_offers_to_reuse_an_existing_gitdir() {
    let (_td, g, _src, url) = setup();
    let mut app = App::new(g.clone());
    add(&mut app, &g, &url);
    // o k keeps the gitdir
    app.script_answers(&["lib"]);
    press(&mut app, &["o", "k"]);
    assert!(!exists(&g, "lib"));
    assert!(g.gitdir.join("modules/lib").is_dir());
    g.commit(Some("remove lib"), &[]).unwrap();
    // [r] renames it and clones again
    app.script_answers(&[&url]);
    app.script_keys(&["RET"]);
    app.script_keys(&["RET"]);
    app.script_keys(&["r"]);
    press(&mut app, &["o", "a"]);
    let last = app.prompts().last().cloned().unwrap();
    assert!(last.contains("already exists.\nType [u] to use the existing gitdir"), "{last}");
    let renamed = std::fs::read_dir(g.gitdir.join("modules"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .any(|n| n.starts_with("lib-"));
    assert!(renamed);
    assert!(exists(&g, "lib/a"), "{}", app.message());
}

#[test]
fn one_module_or_all_with_a_prefix_argument() {
    let (_td, g, _src, url) = setup();
    let mut app = App::new(g.clone());
    add(&mut app, &g, &url);
    // only the arguments deinit understands are used (no --recursive)
    app.script_answers(&["lib"]);
    press(&mut app, &["o", "-", "r", "d"]);
    assert_eq!(app.prompts().last().unwrap(), "Unpopulate module");
    assert!(!exists(&g, "lib/a"), "{}", app.message());
    // populate: only modules without a working tree
    app.script_answers(&["lib"]);
    press(&mut app, &["o", "p"]);
    assert_eq!(app.prompts().last().unwrap(), "Populate module");
    assert!(exists(&g, "lib/a"), "{}", app.message());
    press(&mut app, &["o", "C-u", "p"]);
    assert_eq!(app.message(), "No modules satisfying magit-module-no-worktree-p available");
    // update all (confirmed, also when there is just one)
    app.script_keys(&["y"]);
    press(&mut app, &["o", "C-u", "u"]);
    assert_eq!(app.prompts().last().unwrap(), "T? (y or n) ");
    // a module that isn't one
    app.script_answers(&["nope"]);
    press(&mut app, &["o", "s"]);
    assert_eq!(app.message(), "[No match]");
}

#[test]
fn remove_dirty_modules_and_gitdirs() {
    let (_td, g, _src, url) = setup();
    let mut app = App::new(g.clone());
    add(&mut app, &g, &url);
    std::fs::write(g.toplevel.join("lib/a"), "changed\n").unwrap();
    app.script_answers(&["lib"]);
    press(&mut app, &["o", "k"]);
    assert_eq!(app.message(), "Omitting module lib, it has uncommitted changes");
    assert!(exists(&g, "lib/a"));
    // --force: stashed after confirming; C-u: the gitdir trashed
    app.script_answers(&["lib"]);
    app.script_keys(&["y"]);
    app.script_keys(&["y"]);
    press(&mut app, &["o", "-", "f", "C-u", "k"]);
    let prompts = app.prompts();
    assert_eq!(prompts[prompts.len() - 2..], ["Remove dirty module lib? (y or n) ", "Trash gitdir of module lib? (y or n) "]);
    assert!(!exists(&g, "lib"), "{}", app.message());
    assert!(!g.gitdir.join("modules/lib").exists());
    assert!(!read(&g, ".gitmodules").contains("lib"));
}

#[test]
fn list_modules() {
    let (_td, g, _src, url) = setup();
    let mut app = App::new(g.clone());
    add(&mut app, &g, &url);
    press(&mut app, &["o", "l"]);
    let (title, text) = app.text_view().unwrap();
    assert_eq!(title, "Modules");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("Path") && lines[0].contains("Version") && lines[0].ends_with("S   B"), "{text}");
    assert!(lines[1].starts_with("lib "), "{text}");
    assert!(lines[1].contains("#") && lines[1].ends_with("0   1"), "{text}");
}
