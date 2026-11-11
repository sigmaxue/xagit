//! magit-branch-orphan (b o, level 6), magit-branch-shelve (b h) and
//! magit-branch-unshelve (b H, level 7), magit-delete-shelved-branch.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;
use xagit_lib::ui::options::Options;

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn level7(g: &Git) -> App {
    let mut app = App::new(g.clone());
    let (o, _) = Options::parse("transient-level magit-branch 7");
    app.set_options(o);
    app
}

fn refs(g: &Git, ns: &str) -> Vec<String> {
    g.git_lines(&["for-each-ref", "--format=%(refname)", ns])
}

#[test]
fn orphan() {
    let (_td, g, _) = repo_with_commit();
    // hidden at the default level
    let mut app = App::new(g.clone());
    app.press("b").unwrap();
    assert!(!app.menu_keys().contains(&"o".to_string()));
    app.press("C-g").unwrap();
    let mut app = level7(&g);
    app.press("b").unwrap();
    assert!(app.transient_popup().join("\n").contains("o new orphan"));
    app.script_answers(&["master", "lonely"]);
    app.press("o").unwrap();
    assert_eq!(app.prompts()[..2], ["Create and checkout orphan branch starting at", "Name for new branch"]);
    let head = g.git_string(&["symbolic-ref", "HEAD"]).unwrap();
    assert_eq!(head.trim(), "refs/heads/lonely");
    // no commit yet, the contents of master staged
    assert!(!g.git_ok(&["rev-parse", "--verify", "-q", "HEAD"]));
    assert_eq!(g.staged_files().len(), 1, "{:?}", g.staged_files());
}

#[test]
fn shelve_and_unshelve() {
    let (_td, g, _) = repo_with_commit();
    git(&g, &["branch", "topic"]);
    git(&g, &["config", "branch.topic.pushRemote", "origin"]);
    let date = g.git_string(&["log", "-1", "--format=%cs", "topic"]).unwrap().trim().to_string();
    let mut app = level7(&g);
    // the current branch isn't offered, the previous branch the default
    git(&g, &["checkout", "-q", "topic"]);
    git(&g, &["checkout", "-q", "master"]);
    app.script_answers(&[""]);
    press(&mut app, &["b", "h"]);
    assert_eq!(app.prompts().last().unwrap(), "Shelve branch");
    let shelved = format!("refs/shelved/{date}-topic");
    assert_eq!(refs(&g, "refs/shelved"), std::slice::from_ref(&shelved), "{}", app.message());
    assert!(!g.branches().contains(&"topic".to_string()));
    assert_eq!(g.git_string(&["config", "branch.topic.pushRemote"]).unwrap_or_default().trim(), "");
    // the reflog moved along
    assert!(g.gitdir.join("logs").join(&shelved).exists());
    assert!(!g.gitdir.join("logs/refs/heads/topic").exists());
    // the current branch can't be shelved
    app.script_keys(&["m", "a", "s", "t", "e", "r", "RET", "C-g"]);
    press(&mut app, &["b", "h"]);
    assert!(g.branches().contains(&"master".to_string()));
    assert_eq!(refs(&g, "refs/shelved"), std::slice::from_ref(&shelved));
    // unshelve: the date dropped
    app.script_answers(&[&format!("{date}-topic")]);
    press(&mut app, &["b", "H"]);
    assert_eq!(app.prompts().last().unwrap(), "Unshelve branch");
    assert!(g.branches().contains(&"topic".to_string()), "{}", app.message());
    assert!(refs(&g, "refs/shelved").is_empty(), "{} {:?}", app.message(), refs(&g, "refs/shelved"));
    assert!(g.gitdir.join("logs/refs/heads/topic").exists());
    // a shelved branch without a date keeps its name
    git(&g, &["update-ref", "refs/shelved/plain", "HEAD"]);
    app.script_answers(&["plain"]);
    press(&mut app, &["b", "H"]);
    assert!(g.branches().contains(&"plain".to_string()));
    // an existing branch isn't overwritten
    git(&g, &["update-ref", "refs/shelved/2020-01-01-plain", "HEAD"]);
    app.script_answers(&["2020-01-01-plain"]);
    press(&mut app, &["b", "H"]);
    assert_eq!(refs(&g, "refs/shelved"), ["refs/shelved/2020-01-01-plain"]);
}

#[test]
fn delete_shelved_branch() {
    let (_td, g, _) = repo_with_commit();
    git(&g, &["update-ref", "refs/shelved/2020-01-01-a", "HEAD"]);
    git(&g, &["update-ref", "refs/shelved/2021-01-01-b", "HEAD"]);
    let mut app = App::new(g.clone());
    app.script_answers(&["magit-delete-shelved-branch", "2020-01-01-a"]);
    app.press("M-x").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "Delete shelved branch");
    assert_eq!(refs(&g, "refs/shelved"), ["refs/shelved/2021-01-01-b"]);
    // M-x magit-branch-shelve at any level
    git(&g, &["branch", "x"]);
    app.script_answers(&["magit-branch-shelve", "x"]);
    app.press("M-x").unwrap();
    assert_eq!(refs(&g, "refs/shelved").len(), 2);
}
