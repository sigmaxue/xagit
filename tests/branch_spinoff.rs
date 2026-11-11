//! magit-branch-spinoff (b s) and magit-branch-spinout (b S): the
//! unpushed commits move to a new branch, the current branch is reset to
//! where it meets its upstream (or to FROM^, FROM the oldest commit
//! selected).

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn rev(g: &Git, r: &str) -> String {
    g.git_string(&["rev-parse", r]).unwrap().trim().to_string()
}

fn upstream(g: &Git, b: &str) -> String {
    g.git_string(&["rev-parse", "--abbrev-ref", &format!("{b}@{{upstream}}")]).unwrap_or_default().trim().to_string()
}

fn commit(g: &Git, msg: &str) {
    write(g, "file.txt", &format!("{msg}\n"));
    git(g, &["commit", "-qam", msg]);
}

/// master tracking origin/master, with the unpushed commits c1, c2, c3.
fn ahead() -> (TempDir, TempDir, Git) {
    let (utd, up, _) = repo_with_commit();
    let (td, g, _) = repo_with_commit();
    git(&g, &["remote", "add", "origin", &up.toplevel.to_string_lossy()]);
    git(&g, &["fetch", "-q", "origin"]);
    git(&g, &["reset", "-q", "--hard", "origin/master"]);
    git(&g, &["branch", "-q", "-u", "origin/master"]);
    for c in ["c1", "c2", "c3"] {
        commit(&g, c);
    }
    (utd, td, g)
}

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

#[test]
fn spinoff() {
    let (_u, _td, g) = ahead();
    let tip = rev(&g, "HEAD");
    let mut app = App::new(g.clone());
    app.script_answers(&["feat"]);
    press(&mut app, &["b", "s"]);
    assert_eq!(app.prompts().last().unwrap(), "Spin off branch");
    assert_eq!(g.current_branch().as_deref(), Some("feat"), "{}", app.message());
    assert_eq!(rev(&g, "feat"), tip);
    assert_eq!(rev(&g, "master"), rev(&g, "origin/master"));
    let base = rev(&g, "origin/master");
    let reflog = g.git_string(&["reflog", "-1", "--format=%gs", "master"]).unwrap();
    assert_eq!(reflog.trim(), format!("reset: moving to {base}"));
    // git sets no upstream for a local starting point
    assert_eq!(upstream(&g, "feat"), "");
    // it exists
    git(&g, &["checkout", "-q", "master"]);
    app.script_answers(&["feat"]);
    press(&mut app, &["b", "s"]);
    assert_eq!(app.message(), "Cannot spin off feat.  It already exists");
    // nothing unpushed: nothing reset
    app.script_answers(&["feat2"]);
    press(&mut app, &["b", "s"]);
    assert_eq!(g.current_branch().as_deref(), Some("feat2"));
    assert_eq!(rev(&g, "master"), base);
}

#[test]
fn spinoff_prefer_remote_upstream() {
    let (_u, _td, g) = ahead();
    let mut app = App::new(g.clone());
    let (o, _) = xagit_lib::ui::options::Options::parse("magit-branch-prefer-remote-upstream master");
    app.set_options(o);
    // the indirect upstream: origin/master (fast-forwardable to master)
    app.script_answers(&["feat"]);
    press(&mut app, &["b", "s"]);
    assert_eq!(upstream(&g, "feat"), "origin/master");
}

#[test]
fn spinoff_without_upstream() {
    let (_td, g, _) = repo_with_commit();
    commit(&g, "c1");
    let tip = rev(&g, "HEAD");
    let mut app = App::new(g.clone());
    app.script_answers(&["feat"]);
    press(&mut app, &["b", "s"]);
    assert_eq!(g.current_branch().as_deref(), Some("feat"));
    assert_eq!(rev(&g, "master"), tip);
    // detached HEAD: created at HEAD
    git(&g, &["checkout", "-q", "--detach", "HEAD~"]);
    app.script_answers(&["det"]);
    press(&mut app, &["b", "S"]);
    assert_eq!(rev(&g, "det"), rev(&g, "HEAD"));
    assert_eq!(g.current_branch(), None);
    app.script_answers(&["det2"]);
    press(&mut app, &["b", "s"]);
    assert_eq!(g.current_branch().as_deref(), Some("det2"));
}

#[test]
fn spinout() {
    let (_u, _td, g) = ahead();
    let tip = rev(&g, "HEAD");
    let mut app = App::new(g.clone());
    app.script_answers(&["out"]);
    press(&mut app, &["b", "S"]);
    assert_eq!(app.prompts().last().unwrap(), "Spin out branch");
    // stays, hard reset
    assert_eq!(g.current_branch().as_deref(), Some("master"));
    assert_eq!(rev(&g, "out"), tip);
    assert_eq!(rev(&g, "HEAD"), rev(&g, "origin/master"));
    assert_eq!(read(&g, "file.txt"), "hello\n");
    // uncommitted changes: like spinoff
    git(&g, &["reset", "-q", "--hard", &tip]);
    write(&g, "file.txt", "dirty\n");
    app.script_answers(&["out2"]);
    press(&mut app, &["b", "S"]);
    assert_eq!(app.message(), "Staying on HEAD due to uncommitted changes");
    assert_eq!(g.current_branch().as_deref(), Some("out2"));
    assert_eq!(rev(&g, "master"), rev(&g, "origin/master"));
    assert_eq!(read(&g, "file.txt"), "dirty\n");
}

#[test]
fn spinoff_from_selection() {
    let (_u, _td, g) = ahead();
    let c1 = rev(&g, "HEAD~2");
    let tip = rev(&g, "HEAD");
    let mut app = App::new(g.clone());
    press(&mut app, &["l", "l"]);
    // c3 and c2 selected: master is reset to c2^ (c1)
    goto(&mut app, "c3");
    press(&mut app, &["C-SPC", "C-n"]);
    assert!(app.point_text().contains("c2"), "{}", app.point_text());
    app.script_answers(&["two"]);
    press(&mut app, &["b", "s"]);
    assert_eq!(g.current_branch().as_deref(), Some("two"), "{}", app.message());
    assert_eq!(rev(&g, "two"), tip);
    assert_eq!(rev(&g, "master"), c1);
    // a commit of the upstream
    git(&g, &["checkout", "-q", "-f", "master"]);
    app.refresh();
    goto(&mut app, "c1");
    press(&mut app, &["C-SPC", "C-n"]);
    app.script_answers(&["up"]);
    press(&mut app, &["b", "s"]);
    let first = g.git_string(&["rev-parse", "--short", "origin/master"]).unwrap();
    assert_eq!(app.message(), format!("Cannot spin off up.  {} is ancestor of upstream origin/master", first.trim()));
    assert!(!g.branches().contains(&"up".to_string()));
    // not reachable from the current branch
    git(&g, &["checkout", "-q", "two"]);
    app.refresh();
    press(&mut app, &["q"]);
    git(&g, &["checkout", "-q", "master"]);
    git(&g, &["checkout", "-q", "-b", "other", "origin/master"]);
    commit(&g, "o1");
    commit(&g, "o2");
    git(&g, &["checkout", "-q", "master"]);
    app.script_answers(&["other"]);
    press(&mut app, &["l", "o"]);
    goto(&mut app, "o2");
    press(&mut app, &["C-SPC", "C-n"]);
    app.script_answers(&["far"]);
    press(&mut app, &["b", "s"]);
    let o1 = g.git_string(&["rev-parse", "--short", "other~"]).unwrap();
    assert_eq!(app.message(), format!("Cannot spin off far.  {} is not reachable from master", o1.trim()));
}

#[test]
fn read_string_ns() {
    let (_u, _td, g) = ahead();
    let mut app = App::new(g.clone());
    app.script_answers(&["a b"]);
    press(&mut app, &["b", "s"]);
    assert_eq!(app.message(), "Input contains whitespace");
    app.script_answers(&["  "]);
    press(&mut app, &["b", "s"]);
    assert_eq!(app.message(), "Need non-empty input");
    // trimmed
    app.script_answers(&[" ok "]);
    press(&mut app, &["b", "s"]);
    assert_eq!(g.current_branch().as_deref(), Some("ok"));
}
