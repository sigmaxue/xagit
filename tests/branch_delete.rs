//! magit-branch-delete (b k): unmerged and current branches, and
//! remote-tracking branches (also deleted on the remote if confirmed).

mod common;

use common::*;
use ngit_lib::prelude::*;
use ngit_lib::ui::app::App;

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

fn delete(app: &mut App, answers: &[&str]) {
    app.script_answers(answers);
    app.press("b").unwrap();
    app.press("k").unwrap();
}

fn prompts_since(app: &App, n: usize) -> Vec<String> {
    app.prompts()[n..].to_vec()
}

#[test]
fn current_branch() {
    let (_td, g, _) = repo_with_commit();
    git(&g, &["checkout", "-q", "-b", "feature"]);
    commit(&g, "f.txt", "feature");
    let mut app = App::new(g.clone());
    // abort
    delete(&mut app, &["feature", "a"]);
    assert_eq!(
        app.prompts(),
        ["Delete branch", "Branch feature is checked out.  [d]etach HEAD & delete, [c]heckout master & delete, [a]bort"]
    );
    assert_eq!(app.message(), "Abort");
    assert!(has_ref(&g, "refs/heads/feature"));
    // check out the main branch first: feature is not merged into it
    let n = app.prompts().len();
    delete(&mut app, &["feature", "c", "y"]);
    assert_eq!(prompts_since(&app, n)[2], "Delete unmerged branch feature? (y/n)");
    assert_eq!(g.current_branch().as_deref(), Some("master"));
    assert!(!has_ref(&g, "refs/heads/feature"));
}

#[test]
fn detach_and_delete() {
    let (_td, g, _) = repo_with_commit();
    git(&g, &["checkout", "-q", "-b", "topic"]);
    let mut app = App::new(g.clone());
    // merged into another branch (master): no confirmation
    delete(&mut app, &["topic", "d"]);
    assert_eq!(app.prompts().len(), 2, "{:?}", app.prompts());
    assert_eq!(g.current_branch(), None);
    assert!(!has_ref(&g, "refs/heads/topic"));
    // unmerged, but with one prefix argument not confirmed either
    git(&g, &["checkout", "-q", "-b", "wip"]);
    commit(&g, "w.txt", "wip");
    let n = app.prompts().len();
    app.script_answers(&["wip", "d"]);
    for k in ["b", "C-u", "k"] {
        app.press(k).unwrap();
    }
    assert_eq!(prompts_since(&app, n)[0], "Force delete branch");
    assert_eq!(prompts_since(&app, n).len(), 2);
    assert!(!has_ref(&g, "refs/heads/wip"));
    // the main branch itself: only detaching is offered
    git(&g, &["checkout", "-q", "master"]);
    let n = app.prompts().len();
    delete(&mut app, &["master", "a"]);
    assert_eq!(prompts_since(&app, n)[1], "Branch master is checked out.  [d]etach HEAD & delete, [a]bort");
}

/// A clone of a repository with the branches x and y (at master).
fn cloned() -> (TempDir, TempDir, Git) {
    let (origin, o, _) = repo_with_commit();
    git(&o, &["branch", "x"]);
    git(&o, &["branch", "y"]);
    let td = TempDir::new().unwrap();
    let dir = td.path().join("clone");
    git(&o, &["clone", "-q", &o.toplevel.to_string_lossy(), &dir.to_string_lossy()]);
    let g = Git::discover(&dir).unwrap();
    (origin, td, g)
}

#[test]
fn remote_tracking_branches() {
    let (origin, _td, g) = cloned();
    let on_origin = |b: &str| {
        std::process::Command::new("git")
            .args(["show-ref", "--verify", "--quiet", &format!("refs/heads/{b}")])
            .current_dir(origin.path())
            .status()
            .unwrap()
            .success()
    };
    let mut app = App::new(g.clone());
    // just the local ref
    delete(&mut app, &["origin/x", "n"]);
    assert_eq!(app.prompts(), ["Delete branch", "Deleting local refs/remotes/origin/x.  Also delete on origin? (y/n)"]);
    assert!(!has_ref(&g, "refs/remotes/origin/x"));
    assert!(on_origin("x"));
    assert!(app.message().starts_with("Delete refs/remotes/origin/x (was "), "{}", app.message());
    // also on the remote
    delete(&mut app, &["origin/y", "y"]);
    assert!(!has_ref(&g, "refs/remotes/origin/y"));
    assert!(!on_origin("y"));
    // a branch gone on the remote: the local ref is deleted all the same
    // (a local remote only warns, others fail and the local ref alone is
    // deleted)
    git(&g, &["fetch", "-q"]);
    assert!(has_ref(&g, "refs/remotes/origin/x"));
    std::process::Command::new("git").args(["branch", "-q", "-D", "x"]).current_dir(origin.path()).status().unwrap();
    delete(&mut app, &["origin/x", "y"]);
    assert!(!has_ref(&g, "refs/remotes/origin/x"), "{}", app.message());
}

#[test]
fn default_prefers_other_branches() {
    let (_td, g, _) = repo_with_commit();
    git(&g, &["branch", "other"]);
    commit(&g, "a.txt", "a");
    git(&g, &["branch", "at-head"]);
    let mut app = App::new(g.clone());
    for k in ["l", "l"] {
        app.press(k).unwrap();
    }
    // HEAD has master and at-head: the one that isn't current
    delete(&mut app, &[""]);
    assert!(!has_ref(&g, "refs/heads/at-head"), "{}", app.message());
    // elsewhere the previous branch
    git(&g, &["checkout", "-q", "other"]);
    git(&g, &["checkout", "-q", "master"]);
    let mut app = App::new(g.clone());
    delete(&mut app, &[""]);
    assert!(!has_ref(&g, "refs/heads/other"), "{}", app.message());
}
