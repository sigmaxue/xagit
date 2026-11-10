//! The references buffer (magit-refs-mode, `y`): its sections, the focus
//! column and commit counts, the cherry commits shown when a ref section
//! is expanded, and the commands remapped in its section keymaps (RET,
//! k, R).

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

fn texts(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn commit(g: &Git, file: &str, msg: &str) {
    write(g, file, msg);
    git(g, &["add", "."]);
    git(g, &["commit", "-q", "-m", msg]);
}

fn has_ref(g: &Git, r: &str) -> bool {
    g.git_ok(&["show-ref", "--verify", "--quiet", r])
}

/// Move to the line TEXT (ignoring leading and trailing whitespace).
fn goto(app: &mut App, text: &str) {
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-p").unwrap();
    }
    for _ in 0..n {
        if app.point_text().trim() == text {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no line {text:?} in {:#?}", texts(app));
}

fn last_prompt(app: &App) -> String {
    app.prompts().last().cloned().unwrap_or_default()
}

/// A clone of a repository with the branch "feature" one commit ahead
/// of master, with a local "topic" one commit ahead of master and the
/// tag v1.0 at master.
fn repo() -> (TempDir, TempDir, Git) {
    let (origin, o, _) = repo_with_commit();
    git(&o, &["checkout", "-q", "-b", "feature"]);
    commit(&o, "f.txt", "feature work");
    git(&o, &["checkout", "-q", "master"]);
    let td = TempDir::new().unwrap();
    let dir = td.path().join("clone");
    git(&o, &["clone", "-q", &o.toplevel.to_string_lossy(), &dir.to_string_lossy()]);
    let g = Git::discover(&dir).unwrap();
    git(&g, &["checkout", "-q", "-b", "topic"]);
    commit(&g, "t.txt", "topic work");
    git(&g, &["tag", "-a", "v1.0", "-m", "release one", "master"]);
    git(&g, &["checkout", "-q", "master"]);
    (origin, td, g)
}

fn refs(g: &Git) -> App {
    let mut app = App::new(g.clone());
    app.press("y").unwrap();
    assert!(matches!(app.mode(), BufferMode::Refs { upstream } if upstream == "HEAD"), "{:?}", app.mode());
    app
}

#[test]
fn sections() {
    let (origin, _td, g) = repo();
    let app = refs(&g);
    assert_eq!(app.header_line().trim_end(), "HEAD");
    let url = origin.path().to_string_lossy().to_string();
    let t = texts(&app);
    assert_eq!(t[0], "Branches (2)");
    assert_eq!(t[1], "@ master          origin/master initial");
    assert_eq!(t[2], "  topic           topic work");
    assert_eq!(t[3], "");
    assert!(t[4].starts_with("Remote origin (") && t[4].ends_with(") (2)"), "{}", t[4]);
    assert!(t[4].contains(url.trim_start_matches("/private")), "{} {url}", t[4]);
    assert_eq!(t[5..8], ["  feature         feature work", "  master          initial", ""]);
    assert_eq!(t[8..11], ["Tags (1)", "  v1.0            release one", ""]);
    assert_eq!(t.len(), 11);
}

#[test]
fn detached_head_and_tracking() {
    let (_o, _td, g) = repo();
    // master ahead of and behind its upstream
    commit(&g, "m.txt", "local work");
    git(&g, &["update-ref", "refs/remotes/origin/master", "refs/remotes/origin/feature"]);
    let app = refs(&g);
    // the counts around the padding (magit-refs--format-local-branches)
    assert_eq!(texts(&app)[1], "@ master1>      <1origin/master local work");
    git(&g, &["checkout", "-q", "--detach", "topic"]);
    let app = refs(&g);
    let t = texts(&app);
    assert_eq!(t[0], "Branches (3)");
    assert_eq!(t[1], "@ (detached)      topic work");
    assert_eq!(t[2], "  master1>      <1origin/master local work");
}

#[test]
fn focus_and_commit_counts() {
    let (_o, _td, g) = repo();
    let mut app = refs(&g);
    // magit-refs-set-show-commit-count
    app.script_answers(&["a"]);
    app.press("C-y").unwrap();
    assert_eq!(last_prompt(&app), "Show commit counts for [a]ll refs, [b]ranches only, [n]othing");
    let t = texts(&app);
    assert_eq!(t[1], "@     master          origin/master initial");
    assert_eq!(t[2], "   <1 topic           topic work");
    assert_eq!(t[5], "   <1 feature         feature work");
    assert_eq!(t[6], "    = master          initial");
    assert_eq!(t[9], "    = v1.0            release one");
    // branches only
    app.script_answers(&["b"]);
    press(&mut app, &["y", "r"]);
    assert_eq!(texts(&app)[9], "      v1.0            release one");
    // compared with another branch (y o): behind it
    app.script_answers(&["topic"]);
    press(&mut app, &["y", "o"]);
    assert_eq!(last_prompt(&app), "Compare with");
    assert!(matches!(app.mode(), BufferMode::Refs { upstream } if upstream == "topic"));
    assert_eq!(app.header_line().trim_end(), "topic");
    let t = texts(&app);
    assert_eq!(t[1], "   1> master          origin/master initial");
    assert_eq!(t[2], "*     topic           topic work");
    // with the current branch (y c)
    app.script_answers(&["n"]);
    press(&mut app, &["C-y", "y", "c"]);
    assert_eq!(app.header_line().trim_end(), "master");
    assert_eq!(texts(&app)[1], "* master          origin/master initial");
}

#[test]
fn cherry_commits() {
    let (_o, _td, g) = repo();
    let mut app = refs(&g);
    goto(&mut app, "topic           topic work");
    app.press("TAB").unwrap();
    let hash = g.git_string(&["rev-parse", "--short", "topic"]).unwrap();
    let t = texts(&app);
    assert_eq!(t[3], format!("+ {hash} topic work"));
    assert_eq!(app.point_text().trim(), "topic           topic work");
    // the commit at point: RET shows it
    app.press("C-n").unwrap();
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { rev, .. } if hash.starts_with(rev.as_str()) || rev.starts_with(&hash)), "{:?}", app.mode());
    app.press("q").unwrap();
    // hidden again
    goto(&mut app, "topic           topic work");
    app.press("TAB").unwrap();
    assert_eq!(texts(&app).len(), 11);
    // nothing to show
    goto(&mut app, "v1.0            release one");
    app.press("TAB").unwrap();
    assert_eq!(app.message(), "No cherries for v1.0");
    assert_eq!(texts(&app).len(), 11);
}

#[test]
fn visit_ref() {
    let (_o, _td, g) = repo();
    let mut app = refs(&g);
    goto(&mut app, "feature         feature work");
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { rev, .. } if rev == "origin/feature"), "{:?}", app.mode());
    app.press("q").unwrap();
    assert!(matches!(app.mode(), BufferMode::Refs { .. }));
    // with a prefix argument magit-show-commit reads the revision, the
    // ref at point the default
    goto(&mut app, "topic           topic work");
    app.script_keys(&["RET"]);
    press(&mut app, &["C-u", "RET"]);
    assert_eq!(app.prompts().last().map(String::as_str), Some("Show commit"));
    assert!(matches!(app.mode(), BufferMode::Revision { rev, .. } if rev == "topic"), "{:?}", app.mode());
    app.press("q").unwrap();
    goto(&mut app, "topic           topic work");
    app.script_keys(&["C-a", "C-k", "v", "1", "TAB", "RET"]);
    press(&mut app, &["C-u", "RET"]);
    assert!(matches!(app.mode(), BufferMode::Revision { rev, .. } if rev == "v1.0"), "{:?}", app.mode());
    assert_eq!(app.revision_history()[0], "v1.0");
    app.press("q").unwrap();
    // magit-branch-or-commit-at-point is the default of b b
    goto(&mut app, "topic           topic work");
    app.script_answers(&[""]);
    press(&mut app, &["b", "b"]);
    assert_eq!(g.current_branch().as_deref(), Some("topic"), "{:?} {}", app.prompts(), app.message());
}

#[test]
fn delete_things() {
    let (_o, _td, g) = repo();
    git(&g, &["branch", "a"]);
    git(&g, &["branch", "b"]);
    let mut app = refs(&g);
    // k on a branch: magit-branch-delete, the branch at point the default
    goto(&mut app, "topic           topic work");
    app.script_answers(&["", "y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts(), ["Delete branch", "Delete unmerged branch topic? (y or n) "]);
    assert!(!has_ref(&g, "refs/heads/topic"));
    // the selected branches
    goto(&mut app, "a               initial");
    press(&mut app, &["C-SPC", "C-n"]);
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(last_prompt(&app), "Delete 2 branches? (y or n) ");
    assert!(!has_ref(&g, "refs/heads/a") && !has_ref(&g, "refs/heads/b"), "{}", app.message());
    // a tag: magit-tag-delete
    goto(&mut app, "v1.0            release one");
    app.script_answers(&[""]);
    app.press("k").unwrap();
    assert_eq!(last_prompt(&app), "Delete tag");
    assert!(!has_ref(&g, "refs/tags/v1.0"));
    assert_eq!(texts(&app).iter().filter(|t| t.starts_with("Tags")).count(), 0);
    // a remote: magit-remote-remove, cleaning up the push variables
    git(&g, &["config", "remote.pushDefault", "origin"]);
    git(&g, &["config", "branch.master.pushRemote", "origin"]);
    let heading = texts(&app).into_iter().find(|t| t.starts_with("Remote origin")).unwrap();
    goto(&mut app, &heading);
    app.script_answers(&[""]);
    app.press("k").unwrap();
    assert_eq!(last_prompt(&app), "Delete remote");
    assert!(g.remotes().is_empty());
    assert!(g.git_string(&["config", "remote.pushDefault"]).unwrap_or_default().is_empty());
    assert!(g.git_string(&["config", "branch.master.pushRemote"]).unwrap_or_default().is_empty());
}

#[test]
fn rename_things() {
    let (_o, _td, g) = repo();
    git(&g, &["config", "branch.master.pushRemote", "origin"]);
    let mut app = refs(&g);
    // R on a remote: magit-remote-rename
    let heading = texts(&app).into_iter().find(|t| t.starts_with("Remote origin")).unwrap();
    goto(&mut app, &heading);
    app.script_answers(&["", "upstream"]);
    app.press("R").unwrap();
    assert_eq!(app.prompts(), ["Rename remote", "Rename origin to"]);
    assert_eq!(g.remotes(), ["upstream"]);
    assert_eq!(g.git_string(&["config", "branch.master.pushRemote"]).unwrap(), "upstream");
    assert!(texts(&app).iter().any(|t| t.starts_with("Remote upstream (")), "{:#?}", texts(&app));
    // R on a branch: magit-branch-rename, the local branch at point
    // (else the current one) the default
    goto(&mut app, "topic           topic work");
    app.script_answers(&["", "renamed"]);
    app.press("R").unwrap();
    assert_eq!(app.prompts()[2..], ["Rename branch", "Rename branch 'topic' to"]);
    assert!(has_ref(&g, "refs/heads/renamed") && !has_ref(&g, "refs/heads/topic"), "{}", app.message());
    goto(&mut app, "feature         feature work");
    app.script_answers(&["", "main", "n"]);
    app.press("R").unwrap();
    assert_eq!(app.prompts()[5], "Rename branch 'master' to");
    assert_eq!(g.current_branch().as_deref(), Some("main"));
    // the push-remote setup moves along; the push-target is renamed on
    // the remote if confirmed
    assert_eq!(g.git_string(&["config", "branch.main.pushRemote"]).unwrap(), "upstream");
    assert_eq!(app.prompts()[6], "Also rename \"master\" to \"main\" on \"upstream\"? (y or n) ");
}

#[test]
fn arguments_menu() {
    let (_o, _td, g) = repo();
    let mut app = App::new(g.clone());
    // outside the refs buffer y shows the buffer, C-u y the menu
    press(&mut app, &["C-u", "y"]);
    assert_eq!(app.current_menu(), Some("show-refs-menu"));
    assert!(!app.menu_keys().contains(&"r".to_string()), "{:?}", app.menu_keys());
    // --merged (to HEAD)
    press(&mut app, &["-", "m", "y"]);
    assert!(matches!(app.mode(), BufferMode::Refs { .. }));
    assert_eq!(app.header_line(), "HEAD --merged");
    let t = texts(&app);
    assert_eq!(t[0], "Branches (1)");
    assert!(!t.iter().any(|l| l.contains("topic")));
    assert!(!t.iter().any(|l| l.contains("feature")));
    // in the refs buffer y is the menu, with the buffer's arguments
    app.press("y").unwrap();
    assert_eq!(app.current_menu(), Some("show-refs-menu"));
    assert!(app.menu_keys().contains(&"r".to_string()));
    press(&mut app, &["-", "m", "y"]);
    assert_eq!(app.header_line().trim_end(), "HEAD");
    assert_eq!(texts(&app)[0], "Branches (2)");
    // L: magit-margin-settings
    app.press("L").unwrap();
    assert_eq!(app.current_menu(), Some("margin-settings-menu"));
    app.press("C-g").unwrap();
}

#[test]
fn buffer_name() {
    let (_o, _td, g) = repo();
    let mut app = refs(&g);
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    app.script_answers(&["magit-refs: clone"]);
    app.press("J").unwrap();
    assert!(matches!(app.mode(), BufferMode::Refs { .. }), "{}", app.message());
}

fn options(text: &str) -> xagit_lib::ui::options::Options {
    let (o, e) = xagit_lib::ui::options::Options::parse(text);
    assert!(e.is_empty(), "{e:?}");
    o
}

#[test]
fn filter_alist() {
    let (_o, _td, g) = repo();
    let mut app = refs(&g);
    // the first matching entry decides; remote branches by their full
    // name; the tags heading counts all tags
    app.set_options(options(
        "magit-refs-filter-alist ^origin/master$ t\nmagit-refs-filter-alist master nil\n\
         magit-refs-filter-alist ^v[0-9] nil",
    ));
    let t = texts(&app);
    assert_eq!(t[0], "Branches (1)");
    assert_eq!(t[1], "  topic           topic work");
    assert!(t[3].ends_with(") (2)"), "{}", t[3]);
    assert_eq!(t[4..7], ["  feature         feature work", "  master          initial", ""]);
    assert_eq!(t[7..], ["Tags (1)", ""]);
    // the detached HEAD is always shown
    git(&g, &["checkout", "-q", "--detach", "master"]);
    app.set_options(options("magit-refs-filter-alist . nil"));
    let t = texts(&app);
    assert_eq!(t[..3], ["Branches (1)", "@ (detached)      initial", ""]);
    assert!(t[3].ends_with("):"), "{}", t[3]);
}

#[test]
fn visit_ref_behavior() {
    let (_o, _td, g) = repo();
    let mut app = refs(&g);
    // checkout-branch: local branches are checked out and focused
    app.set_options(options("magit-visit-ref-behavior checkout-branch"));
    goto(&mut app, "topic           topic work");
    app.press("RET").unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("topic"), "{}", app.message());
    assert!(matches!(app.mode(), BufferMode::Refs { upstream } if upstream == "topic"), "{:?}", app.mode());
    // but not tags (they are shown)
    goto(&mut app, "v1.0            release one");
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { rev, .. } if rev == "v1.0"), "{:?}", app.mode());
    app.press("q").unwrap();
    // with a prefix argument: shown without reading the revision
    goto(&mut app, "master          origin/master initial");
    let n = app.prompts().len();
    press(&mut app, &["C-u", "RET"]);
    assert_eq!(app.prompts().len(), n);
    assert!(matches!(app.mode(), BufferMode::Revision { rev, .. } if rev == "master"), "{:?}", app.mode());
    app.press("q").unwrap();
    // focus-on-ref: with a prefix argument the refs buffer is focused
    app.set_options(options("magit-visit-ref-behavior focus-on-ref checkout-any"));
    goto(&mut app, "master          origin/master initial");
    press(&mut app, &["C-u", "RET"]);
    assert!(matches!(app.mode(), BufferMode::Refs { upstream } if upstream == "master"), "{:?}", app.mode());
    assert_eq!(g.current_branch().as_deref(), Some("topic"));
    // checkout-any: tags too
    goto(&mut app, "v1.0            release one");
    app.press("RET").unwrap();
    assert_eq!(g.current_branch(), None);
    assert!(matches!(app.mode(), BufferMode::Refs { upstream } if upstream == "v1.0"), "{:?}", app.mode());
    // create-branch: a local branch for a remote one
    git(&g, &["checkout", "-q", "master"]);
    app.set_options(options("magit-visit-ref-behavior create-branch"));
    goto(&mut app, "feature         feature work");
    app.press("RET").unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("feature"), "{}", app.message());
    assert_eq!(g.git_string(&["rev-parse", "--abbrev-ref", "feature@{upstream}"]).unwrap(), "origin/feature");
    assert!(matches!(app.mode(), BufferMode::Refs { upstream } if upstream == "feature"), "{:?}", app.mode());
    // already exists at the same commit: checked out
    git(&g, &["checkout", "-q", "master"]);
    goto(&mut app, "feature         feature work");
    let n = app.prompts().len();
    app.press("RET").unwrap();
    assert_eq!(app.prompts().len(), n);
    assert_eq!(g.current_branch().as_deref(), Some("feature"));
    // elsewhere: checkout as-is, reset or abort
    git(&g, &["checkout", "-q", "master"]);
    git(&g, &["branch", "-f", "feature", "topic"]);
    let topic = g.rev_parse("topic").unwrap();
    goto(&mut app, "feature         feature work");
    app.script_answers(&["a"]);
    app.press("RET").unwrap();
    assert!(last_prompt(&app).starts_with("Branch feature already exists.\n  [c]heckout feature as-is\n"), "{}", last_prompt(&app));
    assert_eq!(app.message(), "Abort");
    assert_eq!(g.current_branch().as_deref(), Some("master"));
    goto(&mut app, "feature         feature work");
    app.script_answers(&["c"]);
    app.press("RET").unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("feature"));
    assert_eq!(g.rev_parse("HEAD").unwrap(), topic);
    git(&g, &["checkout", "-q", "master"]);
    goto(&mut app, "feature         feature work");
    app.script_answers(&["x", "r"]);
    app.press("RET").unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("feature"));
    assert_eq!(g.rev_parse("HEAD").unwrap(), g.rev_parse("origin/feature").unwrap());
    // local branches and tags are shown
    goto(&mut app, "v1.0            release one");
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { rev, .. } if rev == "v1.0"), "{:?}", app.mode());
}
