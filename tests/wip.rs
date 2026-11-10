//! magit-wip: with magit-wip-mode the changes are committed to the wip
//! refs before and after staging, unstaging, discarding, reversing,
//! applying and resetting; magit-wip-commit (z r), the wip logs (l i,
//! l w) and safe-with-wip.

mod common;

use common::*;
use xagit_lib::git::wip::Wip;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;
use xagit_lib::ui::options::Options;

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn goto(app: &mut App, pred: impl Fn(&str) -> bool) {
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-p").unwrap();
    }
    for _ in 0..n {
        if pred(app.point_text()) {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no such line: {:?}", app.visible_lines().iter().map(|l| l.text.clone()).collect::<Vec<_>>());
}

fn wip_app(g: &Git, extra: &str) -> App {
    let mut app = App::new(g.clone());
    let (o, e) = Options::parse(&format!("magit-wip-mode t\n{extra}"));
    assert!(e.is_empty(), "{e:?}");
    app.set_options(o);
    app
}

/// The subjects of the commits of REF, newest first.
fn subjects(g: &Git, r: &str) -> Vec<String> {
    g.git_string(&["log", "--format=%s", r]).unwrap_or_default().lines().map(str::to_string).collect()
}

const INDEX: &str = "refs/wip/index/refs/heads/master";
const WTREE: &str = "refs/wip/wtree/refs/heads/master";

#[test]
fn stage_and_unstage() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "changed\n");
    let mut app = wip_app(&g, "");
    goto(&mut app, |t| t.contains("modified") && t.contains("file.txt"));
    app.press("s").unwrap();
    assert_eq!(subjects(&g, INDEX)[..2], ["autosave file.txt after stage", "start autosaving index"]);
    assert_eq!(subjects(&g, WTREE)[..2], ["autosave file.txt before stage", "start autosaving worktree"]);
    assert_eq!(g.git_string(&["show", &format!("{WTREE}:file.txt")]).unwrap(), "changed");
    // nothing new in the worktree: no worktree commit; the index is
    // committed again after unstaging
    goto(&mut app, |t| t.contains("modified") && t.contains("file.txt"));
    app.press("u").unwrap();
    assert_eq!(subjects(&g, INDEX)[0], "autosave file.txt after unstage");
    assert_eq!(subjects(&g, WTREE).len(), 3, "{:?}", subjects(&g, WTREE));
    // U: all tracked files
    git(&g, &["add", "file.txt"]);
    app.refresh();
    app.script_answers(&["y"]);
    app.press("U").unwrap();
    assert_eq!(subjects(&g, INDEX)[..2], ["autosave tracked files after unstage", "autosave tracked files before unstage"]);
}

#[test]
fn without_wip_mode() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g.clone());
    goto(&mut app, |t| t.contains("modified") && t.contains("file.txt"));
    app.press("s").unwrap();
    assert!(subjects(&g, INDEX).is_empty());
    assert!(g.git_string(&["for-each-ref", "refs/wip/"]).unwrap().is_empty());
}

#[test]
fn discard_is_saved_first() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "precious\n");
    // safe-with-wip: discard isn't confirmed
    let mut app = wip_app(&g, "magit-no-confirm safe-with-wip");
    goto(&mut app, |t| t.contains("modified") && t.contains("file.txt"));
    app.press("k").unwrap();
    assert!(app.prompts().is_empty(), "{:?}", app.prompts());
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert_eq!(subjects(&g, WTREE)[..2], ["autosave file.txt after discard", "autosave file.txt before discard"]);
    assert_eq!(g.git_string(&["show", &format!("{WTREE}~:file.txt")]).unwrap(), "precious");
    // a hunk
    write(&g, "file.txt", "again\n");
    app.refresh();
    goto(&mut app, |t| t.contains("modified") && t.contains("file.txt"));
    app.press("tab").unwrap();
    goto(&mut app, |t| t.starts_with("@@"));
    app.press("k").unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert_eq!(g.git_string(&["show", &format!("{WTREE}~:file.txt")]).unwrap(), "again");
    // a declined discard has no after hook
    write(&g, "file.txt", "third\n");
    let (o, _) = Options::parse("magit-wip-mode t");
    app.set_options(o);
    app.refresh();
    goto(&mut app, |t| t.contains("modified") && t.contains("file.txt"));
    app.script_answers(&["n"]);
    app.press("k").unwrap();
    assert_eq!(subjects(&g, WTREE)[0], "autosave file.txt before discard");
    assert_eq!(read(&g, "file.txt"), "third\n");
}

#[test]
fn wip_commit_and_logs() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "wip\n");
    let mut app = wip_app(&g, "");
    press(&mut app, &["z", "r"]);
    assert_eq!(subjects(&g, WTREE)[..2], ["wip-save tracked files", "start autosaving worktree"]);
    // with a prefix argument the message is read
    write(&g, "file.txt", "wip 2\n");
    app.script_answers(&["my message"]);
    press(&mut app, &["z", "C-u", "r"]);
    assert_eq!(app.prompts().last().unwrap(), "Wip commit message");
    assert_eq!(subjects(&g, WTREE)[0], "my message");
    // l w: the worktree wip log, l i: the index one
    press(&mut app, &["l", "w"]);
    assert!(app.visible_lines().iter().any(|l| l.text.contains("my message")), "{:?}", app.visible_lines().iter().map(|l| l.text.clone()).collect::<Vec<_>>());
    app.press("q").unwrap();
    press(&mut app, &["l", "i"]);
    assert!(app.visible_lines().iter().any(|l| l.text.contains("start autosaving index")));
    app.press("q").unwrap();
    // only with magit-wip-mode
    app.set_options(Options::default());
    press(&mut app, &["l", "w"]);
    assert!(!app.visible_lines().iter().any(|l| l.text.contains("my message")));
}

#[test]
fn reset() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "lost\n");
    let mut app = wip_app(&g, "");
    app.script_answers(&["HEAD"]);
    press(&mut app, &["X", "w"]);
    assert_eq!(app.prompts().last().unwrap(), "Reset worktree to");
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert_eq!(subjects(&g, WTREE)[..2], ["autosave tracked files after reset", "autosave tracked files before reset"]);
    // the index reset to HEAD is unstaging
    write(&g, "file.txt", "staged\n");
    git(&g, &["add", "file.txt"]);
    app.script_answers(&["HEAD"]);
    press(&mut app, &["X", "i"]);
    assert_eq!(subjects(&g, INDEX)[..2], ["autosave tracked files after unstage", "autosave tracked files before unstage"]);
    assert_eq!(g.git_string(&["status", "--porcelain"]).unwrap(), " M file.txt");
    // a hard reset: the worktree saved before
    write(&g, "file.txt", "hard\n");
    app.script_answers(&["HEAD"]);
    press(&mut app, &["X", "h"]);
    assert_eq!(app.prompts().last().unwrap(), "Hard reset master to");
    assert_eq!(subjects(&g, WTREE)[0], "autosave tracked files before reset");
    assert_eq!(g.git_string(&["show", &format!("{WTREE}:file.txt")]).unwrap(), "hard");
    assert_eq!(read(&g, "file.txt"), "hello\n");
}

#[test]
fn new_commits() {
    let (_td, g, _) = repo_with_commit();
    let wip = Wip::default();
    write(&g, "file.txt", "one\n");
    wip.commit(&g, &[], None).unwrap();
    assert_eq!(subjects(&g, WTREE)[..2], ["autosave tracked files", "start autosaving worktree"]);
    // a new commit: the wip ref starts again from it
    git(&g, &["commit", "-qam", "c2"]);
    write(&g, "file.txt", "two\n");
    wip.commit(&g, &[], Some(" after save")).unwrap();
    assert_eq!(subjects(&g, WTREE)[..3], ["autosave tracked files after save", "start autosaving worktree", "c2"]);
    let tips = Wip::log_tips(&g, WTREE, 2);
    assert_eq!(tips.len(), 2, "{tips:?}");
    assert_eq!(tips[0], WTREE);
    assert_eq!(g.git_string(&["log", "-1", "--format=%s", &tips[1]]).unwrap(), "autosave tracked files");
    // magit-wip-merge-branch: the branch is merged into the wip ref
    let merge = Wip { merge_branch: true, ..Wip::default() };
    git(&g, &["commit", "-qam", "c3"]);
    write(&g, "file.txt", "three\n");
    merge.commit(&g, &[], None).unwrap();
    assert_eq!(subjects(&g, WTREE)[..2], ["autosave tracked files", "merge refs/heads/master into worktree"]);
    // a detached HEAD has its own wip refs
    git(&g, &["checkout", "-q", "--detach"]);
    assert_eq!(wip.wip_ref(&g, "wtree/", None), "refs/wip/wtree/HEAD");
}
