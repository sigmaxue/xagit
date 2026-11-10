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
    assert_eq!(app.menu_keys(), ["b", "c", "m", "k", "g", "l"]);
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

/// TEXT of the visible lines.
fn texts(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

/// Point on the first visible line containing TEXT.
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
    panic!("no line {text:?}: {:?}", texts(app));
}

/// repo plus the worktrees TMP/wt-topic (topic) and TMP/wt-detached.
fn three_worktrees() -> (TempDir, Git, std::path::PathBuf) {
    let (td, g, base) = repo();
    git(&g, &["worktree", "add", "-q", &base.join("wt-topic").to_string_lossy(), "topic"]);
    git(&g, &["worktree", "add", "-q", "--detach", &base.join("wt-detached").to_string_lossy(), "HEAD"]);
    (td, g, base)
}

#[test]
fn list_buffer() {
    let (_td, g, base) = three_worktrees();
    let mut app = App::new(g.clone());
    press(&mut app, &["Z", "l"]);
    assert_eq!(app.mode(), &BufferMode::Worktrees);
    let t = texts(&app);
    assert_eq!(t[0], "Worktrees (3)", "{t:?}");
    // the branch (else the abbreviated commit), aligned, then the
    // directory relative to this worktree
    let short = g.rev_short("HEAD");
    let width = short.len().max("master".len()).max("topic".len()) + 1;
    // the current worktree by its absolute name (its relative name is "./")
    assert_eq!(t[1], format!("{:<width$}{}/", "master", base.join("repo").display()), "{t:?}");
    // the others in git's order
    let mut rest = t[2..4].to_vec();
    rest.sort();
    let mut want = vec![format!("{:<width$}../wt-topic/", "topic"), format!("{:<width$}../wt-detached/", short)];
    want.sort();
    assert_eq!(rest, want, "{t:?}");
    // a single worktree is listed too
    let (_td2, g2, _) = repo();
    let mut app2 = App::new(g2);
    press(&mut app2, &["Z", "l"]);
    assert_eq!(texts(&app2)[..2].len(), 2);
    assert_eq!(texts(&app2)[0], "Worktrees (1)");
}

#[test]
fn list_buffer_locked_and_keys() {
    let (_td, g, base) = three_worktrees();
    git(&g, &["worktree", "lock", "--reason", "usb", &base.join("wt-topic").to_string_lossy()]);
    let mut app = App::new(g.clone());
    press(&mut app, &["Z", "l"]);
    assert!(texts(&app).iter().any(|l| l.ends_with("../wt-topic/ (locked: usb)")), "{:?}", texts(&app));
    // k on a locked worktree refuses
    goto(&mut app, "wt-topic");
    app.script_answers(&[""]);
    app.press("k").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "Delete worktree");
    assert!(app.message().starts_with("Cannot delete locked"), "{}", app.message());
    // k on another: the one at point is the default; confirmed
    goto(&mut app, "wt-detached");
    app.script_answers(&["", "y"]);
    app.press("k").unwrap();
    assert!(!base.join("wt-detached").exists(), "{}", app.message());
    assert_eq!(worktrees(&g).len(), 2);
    // the list was refreshed
    assert!(!texts(&app).iter().any(|l| l.contains("wt-detached")), "{:?}", texts(&app));
    // RET: the status of the worktree at point, without asking
    goto(&mut app, "wt-topic");
    let n = app.prompts().len();
    app.press("RET").unwrap();
    assert_eq!(app.prompts().len(), n);
    assert_eq!(app.mode(), &BufferMode::Status);
    assert_eq!(canon(app.toplevel()), base.join("wt-topic"));
}

#[test]
fn status_section_when_enabled() {
    let (_td, g, base) = three_worktrees();
    // not inserted by default (magit-status-sections-hook)
    let mut app = App::new(g.clone());
    assert!(!texts(&app).iter().any(|l| l.starts_with("Worktrees")));
    let (o, e) = Options::parse("magit-insert-worktrees t");
    assert!(e.is_empty(), "{e:?}");
    app.set_options(o.clone());
    app.refresh();
    assert!(texts(&app).iter().any(|l| l == "Worktrees (3)"), "{:?}", texts(&app));
    // only one worktree: nothing, like magit-insert-worktrees
    let (_td2, g2, _) = repo();
    let mut app2 = App::new(g2);
    app2.set_options(o);
    app2.refresh();
    assert!(!texts(&app2).iter().any(|l| l.starts_with("Worktrees")));
    // RET on the heading opens the list buffer, g on a worktree there
    goto(&mut app, "Worktrees (3)");
    app.press("RET").unwrap();
    assert_eq!(app.mode(), &BufferMode::Worktrees);
    // Z g / m default to the worktree at point
    goto(&mut app, "wt-topic");
    press(&mut app, &["Z", "g"]);
    assert_eq!(canon(app.toplevel()), base.join("wt-topic"));
}
