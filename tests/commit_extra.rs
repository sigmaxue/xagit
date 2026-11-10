//! The suffixes of magit-commit beyond the default level:
//! magit-commit-reshelve (d, level 0), magit-rebase-reword-commit
//! (R, level 0), magit-commit-autofixup (x, level 6) and
//! magit-commit-absorb-modules (X, level 6).

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};
use xagit_lib::ui::options::Options;

/// The user's git configuration ignored (user t <t@t>); a fake
/// git-autofixup on PATH writes its arguments to "autofixup.args".
fn isolate() -> std::path::PathBuf {
    static ONCE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("xagit-commit-extra-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gitconfig");
        std::fs::write(&file, "[user]\n\tname = t\n\temail = t@t\n[protocol \"file\"]\n\tallow = always\n").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &file);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        dir
    })
    .clone()
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

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

fn options(app: &mut App, text: &str) {
    let (o, errors) = Options::parse(text);
    assert!(errors.is_empty(), "{errors:?}");
    app.set_options(o);
}

fn goto(app: &mut App, suffix: &str) {
    for _ in 0..app.visible_lines().len() {
        if app.point_text().ends_with(suffix) {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no line ending with {suffix:?}");
}

#[test]
fn reshelve_head() {
    let (_td, g) = repo();
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    assert!(!app.menu_keys().contains(&"d".to_string()));
    app.press("C-g").unwrap();
    options(&mut app, "transient-level magit-commit d 1");
    let old = out(&g, &["log", "-1", "--format=%ad", "--date=format:%F %T %z"]);
    app.script_answers(&["2020-01-01 10:00:00 +0000"]);
    press(&mut app, &["c", "d"]);
    assert_eq!(app.prompts().last().unwrap(), "Change author and committer dates to: ");
    assert_eq!(out(&g, &["log", "-1", "--format=%at %ct %s"]), "1577872800 1577872800 initial");
    // the old date the previous history element
    assert_eq!(app.infix_history("magit--reshelve-history"), ["2020-01-01 10:00:00 +0000", old.as_str()]);

    // with a prefix argument the committer date only
    app.script_answers(&["2021-01-01 00:00:00 +0000"]);
    press(&mut app, &["c", "C-u", "d"]);
    assert_eq!(app.prompts().last().unwrap(), "Change committer date to: ");
    assert_eq!(out(&g, &["log", "-1", "--format=%at %ct"]), "1577872800 1609459200");

    // not the author: the committer date only
    git(&g, &["commit", "-q", "--amend", "--no-edit", "--author=x <x@x>"]);
    app.script_answers(&["2022-01-01 00:00:00 +0000"]);
    press(&mut app, &["c", "d"]);
    assert_eq!(app.prompts().last().unwrap(), "Change committer date to: ");
    assert_eq!(out(&g, &["log", "-1", "--format=%ct"]), "1640995200");
}

#[test]
fn autofixup() {
    let (_td, g) = repo();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g.clone());
    app.press("c").unwrap();
    assert!(!app.menu_keys().contains(&"x".to_string()));
    app.press("C-g").unwrap();
    options(&mut app, "transient-level magit-commit 6");
    let installed = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .any(|d| d.join("git-autofixup").is_file());
    if !installed {
        press(&mut app, &["c", "x"]);
        assert_eq!(
            app.message(),
            "This command requires the git-autofixup script, which is available from https://github.com/torbiak/git-autofixup"
        );
    }
    // a fake git-autofixup
    let bin = isolate().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("git-autofixup"), "#!/bin/sh\necho \"$@\" > autofixup.args\n").unwrap();
    assert!(std::process::Command::new("chmod").arg("+x").arg(bin.join("git-autofixup")).status().unwrap().success());
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<_> = std::env::split_paths(&path).collect();
    paths.insert(0, bin);
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());

    press(&mut app, &["c", "x"]);
    assert!(matches!(app.mode(), BufferMode::LogSelect { .. }));
    assert_eq!(app.header_line(), "Type C-c C-c to select commit at point, or C-c C-k to abort");
    goto(&mut app, " initial");
    press(&mut app, &["C-c", "C-c"]);
    let head = out(&g, &["rev-parse", "--short", "HEAD"]);
    assert_eq!(read(&g, "autofixup.args"), format!("-vv {head}\n"));

    // with a prefix argument the arguments are set in a transient
    press(&mut app, &["c", "C-u", "x"]);
    assert_eq!(app.current_menu(), Some("commit-autofixup-menu"));
    app.script_answers(&["2"]);
    press(&mut app, &["-", "c", "x"]);
    goto(&mut app, " initial");
    press(&mut app, &["C-c", "C-c"]);
    assert_eq!(read(&g, "autofixup.args"), format!("--context=2 -vv {head}\n"));

    // nothing to absorb
    git(&g, &["checkout", "--", "file.txt"]);
    std::fs::remove_file(g.toplevel.join("autofixup.args")).unwrap();
    git(&g, &["status", "--porcelain"]);
    press(&mut app, &["c", "x"]);
    assert_eq!(app.message(), "There are no changes that could be absorbed");
}

#[test]
fn absorb_modules() {
    let (_td, g) = repo();
    let std = TempDir::new().unwrap();
    let sub = std.path().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    git_init(&sub);
    let s = Git::discover(&sub).unwrap();
    write(&s, "a", "1");
    git(&s, &["add", "a"]);
    git(&s, &["commit", "-qm", "one"]);
    git(&g, &["submodule", "add", "-q", &sub.to_string_lossy(), "lib"]);
    git(&g, &["commit", "-qm", "add lib"]);
    let mut app = App::new(g.clone());
    options(&mut app, "transient-level magit-commit 6");
    press(&mut app, &["c", "X"]);
    assert_eq!(app.message(), "There are no modified modules that could be absorbed");

    let lib = Git::discover(&g.toplevel.join("lib")).unwrap();
    write(&lib, "b", "2");
    git(&lib, &["add", "b"]);
    git(&lib, &["commit", "-qm", "two"]);
    press(&mut app, &["c", "X"]);
    assert!(matches!(app.mode(), BufferMode::LogSelect { .. }));
    goto(&mut app, " initial");
    press(&mut app, &["C-c", "C-c"]);
    assert_eq!(out(&g, &["log", "-1", "--format=%s"]), "fixup! add lib");
    assert_eq!(out(&g, &["submodule", "status"]).chars().next(), Some(' '));
}
