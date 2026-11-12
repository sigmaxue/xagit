//! The suffixes of magit-reset, magit-tag, magit-stash (and
//! magit-stash-push) and magit-gitignore.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};
use xagit_lib::ui::options::Options;

/// Ignore the user's git configuration; the editor writes "edited
/// message" into a message without one.
fn isolate() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("xagit-rtsi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gitconfig");
        std::fs::write(&file, "[user]\n\tname = t\n\temail = t@t\n").unwrap();
        let editor = dir.join("editor.sh");
        std::fs::write(&editor, "#!/bin/sh\ngrep -q '^[^#]' \"$1\" || echo 'edited message' > \"$1\"\n").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &file);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_EDITOR", format!("sh {}", editor.display()));
    });
}

fn repo() -> (TempDir, Git) {
    isolate();
    let (td, g, _) = repo_with_commit();
    (td, g)
}

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn out(g: &Git, args: &[&str]) -> String {
    g.git_string(args).unwrap_or_default().trim_end().to_string()
}

fn commit(g: &Git, file: &str, content: &str, msg: &str) {
    write(g, file, content);
    git(g, &["add", file]);
    git(g, &["commit", "-q", "-m", msg]);
}

fn level(app: &mut App, prefix: &str, level: u8) {
    let (o, _) = Options::parse(&format!("transient-level {prefix} {level}"));
    app.set_options(o);
}

// ------------------------------------------------------------------ reset

#[test]
fn reset_file_checkout_offers_directories() {
    let (_td, g) = repo();
    std::fs::create_dir_all(g.toplevel.join("d")).unwrap();
    commit(&g, "d/a", "a\n", "a");
    commit(&g, "d/b", "b\n", "b");
    write(&g, "d/a", "changed\n");
    write(&g, "d/b", "changed\n");
    git(&g, &["add", "d"]);
    let mut app = App::new(g.clone());
    app.press("X").unwrap();
    assert_eq!(app.menu_keys()[..2], ["b", "f"]);
    app.script_answers(&["HEAD", "d/"]);
    app.press("f").unwrap();
    assert_eq!(read(&g, "d/a"), "a\n");
    assert_eq!(read(&g, "d/b"), "b\n");
    assert_eq!(out(&g, &["status", "--porcelain"]), "");
}

// ------------------------------------------------------------------ tag

#[test]
fn tag_create_annotated_with_prefix() {
    let (_td, g) = repo();
    let mut app = App::new(g.clone());
    app.script_answers(&["light", "HEAD"]);
    press(&mut app, &["t", "t"]);
    let prompts = app.prompts();
    assert_eq!(prompts[prompts.len() - 2], "Create tag");
    assert_eq!(prompts[prompts.len() - 1], "Place tag on");
    assert_eq!(out(&g, &["cat-file", "-t", "light"]), "commit");
    // with a prefix argument (or --annotate) the editor is used
    app.script_answers(&["annotated", "HEAD"]);
    press(&mut app, &["t", "C-u", "t"]);
    assert_eq!(out(&g, &["cat-file", "-t", "annotated"]), "tag");
    assert_eq!(out(&g, &["tag", "-l", "--format=%(contents:subject)", "annotated"]), "edited message");
}

#[test]
fn tag_release() {
    let (_td, g) = repo();
    // the first release: the version of "Release version V" the
    // initial input, the message reviewed (--edit)
    commit(&g, "a", "1", "Release version 0.1");
    let mut app = App::new(g.clone());
    app.script_keys(&["RET"]);
    press(&mut app, &["t", "r"]);
    assert_eq!(app.prompts().last().unwrap(), "Create first release tag: ");
    assert_eq!(out(&g, &["tag", "-l"]), "v0.1");

    // the highest release tag (by version) the initial input, its
    // message with the version replaced
    git(&g, &["tag", "-d", "v0.1"]);
    git(&g, &["tag", "-a", "-m", "Project 1.9", "v1.9", "HEAD"]);
    git(&g, &["tag", "-a", "-m", "Project 1.10", "v1.10", "HEAD"]);
    git(&g, &["tag", "-a", "-m", "Project 1.10-rc1", "v1.10-rc1", "HEAD"]);
    commit(&g, "a", "2", "more");
    app.script_answers(&["v1.11"]);
    press(&mut app, &["t", "-", "a", "r"]);
    assert_eq!(app.prompts().last().unwrap(), "Create release tag (previous was v1.10): ");
    assert_eq!(out(&g, &["tag", "-l", "--format=%(contents:subject)", "v1.11"]), "Project 1.11");
    assert!(matches!(app.mode(), BufferMode::Refs { upstream } if upstream == "HEAD"));

    // the version of the commit "Release version V": not read
    app.press("q").unwrap();
    commit(&g, "a", "3", "Release version 1.12");
    press(&mut app, &["t", "-", "a", "r"]);
    assert_eq!(out(&g, &["tag", "-l", "--format=%(contents:subject)", "v1.12"]), "Project 1.12");
}

#[test]
fn tag_prune() {
    let (_td, g) = repo();
    let btd = TempDir::new().unwrap();
    let dir = btd.path().join("o.git");
    assert!(std::process::Command::new("git").args(["init", "-q", "--bare"]).arg(&dir).status().unwrap().success());
    git(&g, &["remote", "add", "origin", &dir.to_string_lossy()]);
    git(&g, &["tag", "both"]);
    git(&g, &["tag", "remote-only"]);
    git(&g, &["push", "-q", "origin", "both", "remote-only"]);
    git(&g, &["tag", "-d", "remote-only"]);
    git(&g, &["tag", "local-1"]);
    git(&g, &["tag", "local-2"]);
    let mut app = App::new(g.clone());
    app.script_answers(&["origin", "y", "y"]);
    press(&mut app, &["t", "p"]);
    let prompts = app.prompts();
    assert_eq!(prompts[prompts.len() - 3], "Prune tags using remote");
    assert!(prompts[prompts.len() - 2].contains("Delete 2 tags locally?"), "{prompts:?}");
    assert!(prompts[prompts.len() - 1].starts_with("Delete remote-only from remote?"), "{prompts:?}");
    assert_eq!(out(&g, &["tag", "-l"]), "both");
    assert_eq!(out(&g, &["ls-remote", "--tags", "origin"]).lines().count(), 1);
    app.script_answers(&["origin"]);
    press(&mut app, &["t", "p"]);
    assert_eq!(app.message(), "Same tags exist locally and remotely");
}

// ------------------------------------------------------------------ stash

fn stash_list(g: &Git) -> Vec<String> {
    out(g, &["stash", "list", "--format=%gs"]).lines().map(str::to_string).collect()
}

/// file.txt with a staged change, b with an unstaged one, u untracked.
fn changes(g: &Git) {
    commit(g, "b", "b\n", "b");
    write(g, "file.txt", "staged\n");
    git(g, &["add", "file.txt"]);
    write(g, "b", "unstaged\n");
    write(g, "u", "untracked\n");
}

#[test]
fn stash_variants() {
    let (_td, g) = repo();
    changes(&g);
    let mut app = App::new(g.clone());
    let head = out(&g, &["log", "-1", "--format=%h %s"]);
    // the default message
    app.script_answers(&[""]);
    press(&mut app, &["z", "z"]);
    assert_eq!(app.prompts().last().unwrap(), "Stash message (default: On…:…): ");
    assert_eq!(stash_list(&g), [format!("On master: {head}")]);
    assert_eq!(out(&g, &["status", "--porcelain"]), "?? u");
    // the index and the worktree restored
    app.script_answers(&["stash@{0}"]);
    press(&mut app, &["z", "p"]);
    assert_eq!(out(&g, &["status", "--porcelain"]), " M b\nM  file.txt\n?? u");
    assert!(stash_list(&g).is_empty());

    // index: the unstaged changes kept
    app.script_answers(&["idx"]);
    press(&mut app, &["z", "i"]);
    assert_eq!(out(&g, &["status", "--porcelain"]), " M b\n?? u");
    assert_eq!(out(&g, &["show", "stash@{0}:file.txt"]), "staged");
    assert_eq!(out(&g, &["show", "stash@{0}:b"]), "b");
    git(&g, &["checkout", "--", "b"]);
    app.script_answers(&["stash@{0}"]);
    press(&mut app, &["z", "p"]);
    write(&g, "b", "unstaged\n");

    // worktree: the index kept, the stash's index the pre-stash one
    app.script_answers(&["wt"]);
    press(&mut app, &["z", "w"]);
    assert_eq!(out(&g, &["status", "--porcelain"]), "M  file.txt\n?? u");
    assert_eq!(out(&g, &["show", "stash@{0}:b"]), "unstaged");
    assert_eq!(out(&g, &["log", "-1", "--format=%s", "stash@{0}^1"]), "pre-stash index");

    // with a prefix argument the untracked files too
    app.script_answers(&["all"]);
    press(&mut app, &["z", "C-u", "z"]);
    assert_eq!(out(&g, &["status", "--porcelain"]), "");
    assert_eq!(out(&g, &["show", "stash@{0}^3:u"]), "untracked");
    assert_eq!(stash_list(&g), ["all", "wt"]);

    // nothing to stash
    app.script_answers(&["x"]);
    press(&mut app, &["z", "i"]);
    assert_eq!(app.message(), "No staged changes to save");
}

#[test]
fn snapshots_and_transforming() {
    let (_td, g) = repo();
    changes(&g);
    let mut app = App::new(g.clone());
    let head = out(&g, &["log", "-1", "--format=%h %s"]);
    let status = out(&g, &["status", "--porcelain"]);
    press(&mut app, &["z", "Z"]);
    assert_eq!(stash_list(&g), [format!("WIP on master: {head}")]);
    assert_eq!(out(&g, &["status", "--porcelain"]), status);

    // the stash applied to a new branch at the current one
    app.script_answers(&["stash@{0}", "here"]);
    git(&g, &["reset", "-q", "--hard"]);
    press(&mut app, &["z", "B"]);
    assert_eq!(out(&g, &["branch", "--show-current"]), "here");
    assert_eq!(read(&g, "file.txt"), "staged\n");
    assert_eq!(read(&g, "b"), "unstaged\n");

    // a patch named after the subject
    app.script_answers(&["stash@{0}"]);
    press(&mut app, &["z", "f"]);
    assert!(exists(&g, "0001-WIP-on-master.patch") || std::fs::read_dir(&g.toplevel).unwrap().any(|e| {
        let n = e.unwrap().file_name().to_string_lossy().into_owned();
        n.starts_with("0001-WIP-on-master") && n.ends_with(".patch")
    }));
}

#[test]
fn stash_apply_falls_back_to_3way() {
    let (_td, g) = repo();
    commit(&g, "f", "1\n2\n3\n4\n5\n6\n7\n8\n", "f");
    write(&g, "f", "one\n2\n3\n4\n5\n6\n7\n8\n");
    let mut app = App::new(g.clone());
    app.script_answers(&["s1"]);
    press(&mut app, &["z", "z"]);
    write(&g, "f", "1\n2\n3\n4\n5\n6\n7\neight\n");
    app.script_answers(&["stash@{0}", "s"]);
    press(&mut app, &["z", "a"]);
    assert!(app.prompts().last().unwrap().starts_with(
        "Could not apply stash because of unstaged changes.\n\nTo do a three-way merge, these files have to be staged\n  f\n"
    ));
    assert_eq!(read(&g, "f"), "one\n2\n3\n4\n5\n6\n7\neight\n", "{}", app.message());
    assert_eq!(stash_list(&g), ["s1"]);
}

#[test]
fn stash_push_files() {
    let (_td, g) = repo();
    changes(&g);
    let mut app = App::new(g.clone());
    press(&mut app, &["z"]);
    assert!(!app.menu_keys().contains(&"P".to_string()));
    app.press("C-g").unwrap();
    level(&mut app, "magit-stash", 5);
    press(&mut app, &["z", "P"]);
    assert_eq!(app.current_menu(), Some("stash-push-menu"));
    app.script_answers(&["b"]);
    press(&mut app, &["-", "-"]);
    assert_eq!(app.prompts().last().unwrap(), "Limit to file,s: ");
    app.press("P").unwrap();
    assert_eq!(out(&g, &["status", "--porcelain"]), "M  file.txt\n?? u");
    assert_eq!(out(&g, &["show", "stash@{0}:b"]), "unstaged");
}

// ------------------------------------------------------------------ gitignore

#[test]
fn gitignore_suffixes() {
    let (_td, g) = repo();
    std::fs::create_dir_all(g.toplevel.join("sub")).unwrap();
    write(&g, "x.log", "");
    write(&g, "sub/y.tmp", "");
    let mut app = App::new(g.clone());
    app.press("i").unwrap();
    let p = app.transient_popup().join("\n");
    assert!(p.contains("t shared at toplevel (.gitignore)"), "{p}");
    assert!(p.contains("g privately for all repositories (core.excludesfile is not set)"), "{p}");
    assert!(!app.menu_keys().contains(&"w".to_string()));
    app.script_answers(&["*.log"]);
    app.press("t").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "File or pattern to ignore");
    assert_eq!(read(&g, ".gitignore"), "*.log\n");
    assert_eq!(out(&g, &["diff", "--cached", "--name-only"]), ".gitignore");

    // in a subdirectory, the patterns relative to it
    app.script_answers(&["sub", "/y.tmp"]);
    press(&mut app, &["i", "s"]);
    assert_eq!(app.prompts()[app.prompts().len() - 2], "Limit rule to files in: ");
    assert_eq!(read(&g, "sub/.gitignore"), "/y.tmp\n");

    // privately
    write(&g, "z.priv", "");
    app.script_answers(&["z.priv"]);
    press(&mut app, &["i", "p"]);
    assert!(read(&g, ".git/info/exclude").ends_with("z.priv\n"));

    let global = g.toplevel.join("excludes");
    git(&g, &["config", "core.excludesFile", &global.to_string_lossy()]);
    app.press("i").unwrap();
    assert!(app.transient_popup().join("\n").contains(&format!("g privately for all repositories ({})", global.display())));
    app.script_answers(&["*.bak"]);
    app.press("g").unwrap();
    assert_eq!(std::fs::read_to_string(&global).unwrap(), "*.bak\n");

    // skip-worktree at level 7: the only candidate isn't read
    level(&mut app, "magit-gitignore", 7);
    git(&g, &["commit", "-q", "-m", "ignores"]);
    git(&g, &["update-index", "--skip-worktree", "file.txt"]);
    press(&mut app, &["i", "W"]);
    assert_eq!(out(&g, &["ls-files", "-t", "file.txt"]), "H file.txt");
    app.script_answers(&["file.txt"]);
    press(&mut app, &["i", "u"]);
    assert_eq!(out(&g, &["ls-files", "-v", "file.txt"]), "h file.txt");
}
