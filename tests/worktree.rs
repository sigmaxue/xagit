//! magit-worktree (Z, %) and the worktree suffixes of magit-branch.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};
use xagit_lib::ui::options::Options;

/// A repository at TMP/repo (canonical) with the branch topic.
fn repo() -> (TempDir, Git, std::path::PathBuf) {
    let td = TempDir::new().unwrap();
    let base = td.path().canonicalize().unwrap();
    let dir = base.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    git_init(&dir);
    let g = Git::discover(&dir).unwrap();
    write(&g, "f", "1\n");
    git(&g, &["add", "f"]);
    git(&g, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "initial"]);
    git(&g, &["branch", "topic"]);
    (td, g, base)
}

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

fn worktrees(g: &Git) -> Vec<String> {
    g.git_string(&["worktree", "list", "--porcelain"])
        .unwrap()
        .lines()
        .filter_map(|l| l.strip_prefix("worktree ").map(str::to_string))
        .collect()
}

fn canon(p: &std::path::Path) -> std::path::PathBuf {
    p.canonicalize().unwrap()
}

#[test]
fn layout() {
    let (_td, g, _) = repo();
    let mut app = App::new(g.clone());
    app.press("Z").unwrap();
    assert_eq!(app.menu_keys(), ["b", "c", "m", "k", "g"]);
    let p = app.transient_popup().join("\n");
    assert!(p.contains("b worktree") && p.contains("c branch and worktree") && p.contains("g Visit worktree"), "{p}");
}

#[test]
fn checkout_in_a_sibling_directory() {
    let (_td, g, base) = repo();
    let mut app = App::new(g.clone());
    app.script_answers(&["topic"]);
    app.script_keys(&["RET"]);
    press(&mut app, &["Z", "b"]);
    let prompts = app.prompts();
    assert_eq!(prompts[prompts.len() - 2], "In new worktree; checkout");
    assert_eq!(prompts[prompts.len() - 1], "Checkout topic in new worktree: ");
    let wt = base.join("repo_topic");
    assert!(wt.join("f").exists(), "{}", app.message());
    // the status of the new worktree
    assert_eq!(canon(app.toplevel()), wt);
    assert_eq!(app.mode(), &BufferMode::Status);
    assert_eq!(g.git_string(&["-C", &wt.to_string_lossy(), "branch", "--show-current"]).unwrap().trim(), "topic");

    // a new branch; its slashes replaced
    app.script_answers(&["master", "feat/x"]);
    app.script_keys(&["RET"]);
    press(&mut app, &["Z", "c"]);
    assert_eq!(app.prompts().last().unwrap(), "Checkout feat/x in new worktree: ");
    let wt2 = base.join("repo_feat-x");
    assert_eq!(canon(app.toplevel()), wt2);
    assert_eq!(worktrees(&g).len(), 3);
}

#[test]
fn nested_directory_and_visiting() {
    let (_td, g, base) = repo();
    let mut app = App::new(g.clone());
    let (o, e) = Options::parse("magit-read-worktree-directory-function magit-read-worktree-directory-nested");
    assert!(e.is_empty(), "{e:?}");
    app.set_options(o);
    app.script_answers(&["topic"]);
    app.script_keys(&["RET"]);
    press(&mut app, &["Z", "b"]);
    let wt = base.join("repo").join("topic");
    assert_eq!(canon(app.toplevel()), wt);

    // g: the status of another worktree
    app.script_answers(&[&base.join("repo").to_string_lossy()]);
    press(&mut app, &["Z", "g"]);
    assert_eq!(app.prompts().last().unwrap(), "Show status for worktree");
    assert_eq!(canon(app.toplevel()), base.join("repo"));
}

#[test]
fn move_worktree() {
    let (_td, g, base) = repo();
    let old = base.join("old");
    git(&g, &["worktree", "add", "-q", &old.to_string_lossy(), "topic"]);
    let mut app = App::new(g.clone());
    let new = base.join("new");
    app.script_answers(&[&old.to_string_lossy(), &new.to_string_lossy()]);
    press(&mut app, &["Z", "m"]);
    let prompts = app.prompts();
    assert_eq!(prompts[prompts.len() - 2], "Move worktree");
    assert_eq!(prompts[prompts.len() - 1], "Move worktree to: ");
    assert!(new.join("f").exists() && !old.exists(), "{}", app.message());

    // moving the worktree shown: its status follows it into an existing
    // directory
    let mut app = App::new(Git::discover(&new).unwrap());
    let into = base.join("into");
    std::fs::create_dir_all(&into).unwrap();
    app.script_answers(&[&new.to_string_lossy(), &into.to_string_lossy()]);
    press(&mut app, &["Z", "m"]);
    assert_eq!(canon(app.toplevel()), into.join("new"));
}
