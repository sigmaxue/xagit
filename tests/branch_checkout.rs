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

fn with_options(g: &Git, text: &str) -> App {
    let mut app = App::new(g.clone());
    let (o, e) = xagit_lib::ui::options::Options::parse(text);
    assert!(e.is_empty(), "{e:?}");
    app.set_options(o);
    app
}

fn popup(app: &mut App) -> String {
    app.press("b").unwrap();
    let p = app.transient_popup().join("\n");
    app.press("C-g").unwrap();
    p
}

#[test]
fn levels() {
    let (_u, _td, g) = with_remote();
    // level 4: neither -m (6), -r (7) nor w/W (5)
    let mut app = App::new(g.clone());
    let p = popup(&mut app);
    assert!(!p.contains("--merge") && !p.contains("Recurse submodules") && !p.contains("new worktree"), "{p}");
    assert!(p.contains("b branch/revision"));
    // a hidden suffix isn't bound
    press(&mut app, &["b", "-", "m"]);
    assert!(!app.menu_keys().contains(&"-m".to_string()));
    app.press("C-g").unwrap();
    // transient-default-level 5: the worktree commands
    let p = popup(&mut with_options(&g, "transient-default-level 5"));
    assert!(p.contains("w new worktree") && p.contains("W new worktree") && !p.contains("-m Automatically"), "{p}");
    // the prefix at level 6: -m too; -r at level 6 too
    let p = popup(&mut with_options(&g, "transient-level magit-branch 6"));
    assert!(p.contains("-m Automatically merge conflicting local modifications") && !p.contains("-r Recurse"), "{p}");
    let p = popup(&mut with_options(&g, "transient-level magit-branch 6\ntransient-level magit-branch -r 6"));
    assert!(p.contains("-r Recurse submodules when checking out an existing branch"), "{p}");
    // level 0: never shown
    let p = popup(&mut with_options(&g, "transient-level magit-branch b 0"));
    assert!(!p.contains("branch/revision"), "{p}");
    // other prefixes keep the default level
    let (o, e) = xagit_lib::ui::options::Options::parse("transient-level magit-branch 8\ntransient-default-level 0\ntransient-level x");
    assert_eq!(e.len(), 3, "{e:?}");
    assert_eq!(o.prefix_level("magit-branch"), 4);
}

#[test]
fn merge_argument() {
    let (_u, _td, g) = with_remote();
    write(&g, "file.txt", "dirty\n");
    git(&g, &["update-ref", "refs/remotes/origin/other", "origin/feature"]);
    // -m: checked out with --merge despite the uncommitted changes
    let level6 = "transient-level magit-branch 6";
    let mut app = with_options(&g, level6);
    app.script_answers(&["origin/feature"]);
    press(&mut app, &["b", "-", "m", "l"]);
    assert_eq!(head(&g), "feature", "{}", app.message());
    assert_eq!(read(&g, "file.txt"), "dirty\n");
    // the value set (C-x s) isn't used while -m is hidden
    press(&mut app, &["b", "-", "m", "C-x", "s", "C-g"]);
    assert_eq!(app.current_menu(), None);
    app.set_options(Default::default());
    app.script_answers(&["origin/other"]);
    press(&mut app, &["b", "l"]);
    assert_eq!(app.message(), "Cannot checkout when there are uncommitted changes", "{:?}", app.prompts());
    // at level 6 again it is
    let (o, _) = xagit_lib::ui::options::Options::parse(level6);
    app.set_options(o);
    app.script_answers(&["origin/other"]);
    press(&mut app, &["b", "l"]);
    assert_eq!(head(&g), "other", "{}", app.message());
}

fn upstream(g: &Git, b: &str) -> String {
    g.git_string(&["rev-parse", "--abbrev-ref", &format!("{b}@{{upstream}}")]).unwrap_or_default().trim().to_string()
}

#[test]
fn adjust_remote_upstream() {
    let (_u, _td, g) = with_remote();
    // the branches but master and next: master
    let mut app = with_options(&g, "magit-branch-adjust-remote-upstream-alist master (master next)");
    app.script_answers(&["origin/feature"]);
    press(&mut app, &["b", "l"]);
    assert_eq!(upstream(&g, "feature"), "master");
    // the first rule whose upstream exists and that matches
    let text = "magit-branch-adjust-remote-upstream-alist nope ^f\n\
                magit-branch-adjust-remote-upstream-alist master \\`hotfix/\n\
                magit-branch-adjust-remote-upstream-alist topic ^fea";
    let mut app = with_options(&g, text);
    app.script_answers(&["origin/feature", "f2"]);
    press(&mut app, &["b", "c"]);
    assert_eq!(upstream(&g, "f2"), "topic", "{}", app.message());
    // not for local starting points; b n too
    app.script_answers(&["master", "f3"]);
    press(&mut app, &["b", "n"]);
    assert_eq!(upstream(&g, "f3"), "");
    git(&g, &["update-ref", "refs/remotes/origin/hotfix/a", "origin/feature"]);
    let mut app = with_options(&g, "magit-branch-adjust-remote-upstream-alist master \\`hotfix/");
    app.script_answers(&["origin/hotfix/a", "h"]);
    press(&mut app, &["b", "n"]);
    assert_eq!(upstream(&g, "h"), "master");
    // a rule not matching
    app.script_answers(&["origin/feature", "f4"]);
    press(&mut app, &["b", "n"]);
    assert_eq!(upstream(&g, "f4"), "origin/feature");
}

#[test]
fn prefer_remote_upstream() {
    let (_u, _td, g) = with_remote();
    // master ahead of origin/master (the repositories' root commits only
    // agree when made within the same second)
    git(&g, &["reset", "-q", "--hard", "origin/master"]);
    write(&g, "file.txt", "ahead\n");
    git(&g, &["commit", "-qam", "ahead"]);
    git(&g, &["branch", "-q", "-u", "origin/master", "master"]);
    git(&g, &["config", "branch.autoSetupMerge", "always"]);
    // not opted in: the local branch is the upstream
    let mut app = App::new(g.clone());
    app.script_answers(&["master", "x"]);
    press(&mut app, &["b", "n"]);
    assert_eq!(upstream(&g, "x"), "master");
    // opted in (by name, by regexp): its upstream
    let mut app = with_options(&g, "magit-branch-prefer-remote-upstream main master");
    app.script_answers(&["master", "y"]);
    press(&mut app, &["b", "c"]);
    assert_eq!(upstream(&g, "y"), "origin/master", "{}", app.message());
    let mut app = with_options(&g, "magit-branch-prefer-remote-upstream ^ma*");
    app.script_answers(&["master", "z"]);
    press(&mut app, &["b", "n"]);
    assert_eq!(upstream(&g, "z"), "origin/master");
    // not when the remote branch can't be fast-forwarded to it
    git(&g, &["checkout", "-q", "master"]);
    git(&g, &["reset", "-q", "--hard", "HEAD"]);
    git(&g, &["update-ref", "refs/remotes/origin/master", "origin/topic"]);
    app.script_answers(&["master", "w"]);
    press(&mut app, &["b", "n"]);
    assert_eq!(upstream(&g, "w"), "master");
}

#[test]
fn name_suggestions() {
    let (_u, _td, g) = with_remote();
    let mut app = with_options(&g, "magit-branch-name-suggestions feature/ fix/");
    app.script_answers(&["master"]);
    app.script_keys(&["f", "i", "TAB", "o", "n", "e", "RET"]);
    press(&mut app, &["b", "n"]);
    assert!(g.branches().contains(&"fix/one".to_string()), "{}", app.message());
    // the branch name of a remote starting point first
    app.script_answers(&["origin/feature"]);
    app.script_keys(&["f", "e", "a", "t", "u", "r", "e", "TAB", "RET"]);
    press(&mut app, &["b", "n"]);
    assert!(g.branches().contains(&"feature".to_string()), "{:?}", g.branches());
}
