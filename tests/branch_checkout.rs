//! Checking out branches like Magit: magit-checkout (b b) detaches HEAD
//! on a remote branch, magit-branch-checkout (b l) turns a remote branch
//! into a local one, magit-branch-and-checkout (b c) and
//! magit-branch-create (b n) read the starting point, then the name.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

fn config(g: &Git, key: &str) -> String {
    g.git_string(&["config", key]).unwrap_or_default().trim().to_string()
}

/// A repository whose remote origin has the branches master, feature and
/// topic (all fetched), topic also local.
fn with_remote() -> (TempDir, TempDir, Git) {
    let (utd, up, _) = repo_with_commit();
    git(&up, &["branch", "feature"]);
    git(&up, &["checkout", "-q", "-b", "topic"]);
    write(&up, "t.txt", "t\n");
    git(&up, &["add", "t.txt"]);
    git(&up, &["commit", "-qm", "topic"]);
    git(&up, &["checkout", "-q", "master"]);
    let (td, g, _) = repo_with_commit();
    git(&g, &["remote", "add", "origin", &up.toplevel.to_string_lossy()]);
    git(&g, &["fetch", "-q", "origin"]);
    git(&g, &["branch", "topic", "origin/topic"]);
    (utd, td, g)
}

/// Move point to the line containing TEXT.
fn goto(app: &mut App, text: &str) {
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-p").unwrap();
    }
    for _ in 0..n {
        if app.point_text().contains(text) {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no line {text:?}");
}

fn head(g: &Git) -> String {
    g.current_branch().unwrap_or_else(|| "HEAD".into())
}

#[test]
fn branch_checkout_remote() {
    let (_u, _td, g) = with_remote();
    let mut app = App::new(g.clone());
    // the remote branch selected in the refs buffer is the default
    press(&mut app, &["y"]);
    goto(&mut app, "feature");
    app.script_answers(&[""]);
    press(&mut app, &["b", "l"]);
    assert_eq!(app.prompts().last().unwrap(), "Checkout branch");
    assert_eq!(head(&g), "feature", "{}", app.message());
    // a local branch of the same name tracking it
    assert_eq!(config(&g, "branch.feature.remote"), "origin");
    assert_eq!(config(&g, "branch.feature.merge"), "refs/heads/feature");
    // the remote is its push-remote (not remote.pushDefault)
    assert_eq!(config(&g, "branch.feature.pushRemote"), "origin");
    // typed: a remote branch with a local branch of that name isn't a
    // choice; its local branch is checked out
    app.script_answers(&["topic"]);
    press(&mut app, &["b", "l"]);
    assert_eq!(head(&g), "topic");
    // on remote.pushDefault no push-remote is set
    git(&g, &["config", "remote.pushDefault", "origin"]);
    git(&g, &["update-ref", "refs/remotes/origin/other", "HEAD"]);
    app.script_answers(&["origin/other"]);
    press(&mut app, &["b", "l"]);
    assert_eq!(head(&g), "other");
    assert_eq!(config(&g, "branch.other.pushRemote"), "");
}

#[test]
fn branch_checkout_default_and_new() {
    let (_u, _td, g) = with_remote();
    let mut app = App::new(g.clone());
    // on the remote branch of a local branch: that branch is the default
    press(&mut app, &["y"]);
    goto(&mut app, "topic");
    // the first topic line is the local branch; the remote one follows
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-n").unwrap();
        if app.point_text().contains("topic") {
            break;
        }
    }
    app.script_answers(&[""]);
    press(&mut app, &["b", "l"]);
    assert_eq!(head(&g), "topic", "{}", app.message());
    // a new name: the starting point is read
    app.script_answers(&["fresh", "origin/feature"]);
    press(&mut app, &["b", "l"]);
    assert_eq!(app.prompts().last().unwrap(), "Create fresh starting at");
    assert_eq!(head(&g), "fresh");
    assert_eq!(config(&g, "branch.fresh.merge"), "refs/heads/feature");
    assert_eq!(config(&g, "branch.fresh.pushRemote"), "");
    // not with uncommitted changes
    write(&g, "file.txt", "dirty\n");
    app.script_answers(&["origin/feature"]);
    press(&mut app, &["b", "l"]);
    assert_eq!(app.message(), "Cannot checkout when there are uncommitted changes");
    assert_eq!(head(&g), "fresh");
}

#[test]
fn checkout_remote_branch_detaches() {
    let (_u, _td, g) = with_remote();
    let mut app = App::new(g.clone());
    press(&mut app, &["y"]);
    goto(&mut app, "feature");
    app.script_answers(&[""]);
    press(&mut app, &["b", "b"]);
    assert_eq!(app.prompts().last().unwrap(), "Checkout");
    assert_eq!(head(&g), "HEAD", "{}", app.message());
    assert_eq!(g.git_string(&["rev-parse", "HEAD"]).unwrap(), g.git_string(&["rev-parse", "origin/feature"]).unwrap());
    // detached at the remote branch's commit: the previous branch is
    // the default, not HEAD's commit
    app.script_answers(&[""]);
    press(&mut app, &["b", "b"]);
    assert_eq!(head(&g), "master");
    // a local branch ambiguous with a tag is qualified, then checked
    // out without heads/
    git(&g, &["branch", "dup"]);
    git(&g, &["tag", "dup"]);
    app.refresh();
    goto(&mut app, "dup");
    app.script_answers(&[""]);
    press(&mut app, &["b", "b"]);
    assert_eq!(g.git_string(&["symbolic-ref", "HEAD"]).unwrap(), "refs/heads/dup", "{}", app.message());
}

#[test]
fn branch_and_checkout() {
    let (_u, _td, g) = with_remote();
    let mut app = App::new(g.clone());
    // the remote branch at point is the starting point, its name
    // suggested
    press(&mut app, &["y"]);
    goto(&mut app, "feature");
    app.script_keys(&["RET"]);
    app.script_keys(&["TAB", "RET"]);
    press(&mut app, &["b", "c"]);
    let p = app.prompts();
    assert_eq!(p[p.len() - 2..], ["Create and checkout branch starting at", "Name for new branch"]);
    assert_eq!(head(&g), "feature", "{}", app.message());
    assert_eq!(config(&g, "branch.feature.merge"), "refs/heads/feature");
    // an existing name is refused, then another is read
    app.script_answers(&["master", "topic", "mine"]);
    press(&mut app, &["b", "c"]);
    assert_eq!(head(&g), "mine", "{}", app.message());
    // not a revision
    app.script_answers(&["nope"]);
    press(&mut app, &["b", "c"]);
    assert_eq!(app.message(), "Not a valid starting-point: nope");
    // b n: created, not checked out
    app.script_answers(&["origin/topic", "made"]);
    press(&mut app, &["b", "n"]);
    assert_eq!(app.prompts().last().unwrap(), "Name for new branch");
    assert_eq!(head(&g), "mine");
    assert!(g.branches().contains(&"made".to_string()));
    // an invalid name
    app.script_answers(&["master", "a..b", "ok"]);
    press(&mut app, &["b", "n"]);
    assert!(g.branches().contains(&"ok".to_string()));
    // a stash: git stash branch
    write(&g, "file.txt", "stashed\n");
    git(&g, &["stash", "-q"]);
    app.script_answers(&["stash@{0}", "from-stash"]);
    press(&mut app, &["b", "c"]);
    assert_eq!(head(&g), "from-stash", "{}", app.message());
    assert_eq!(read(&g, "file.txt"), "stashed\n");
}
