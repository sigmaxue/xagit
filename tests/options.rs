//! User options (`~/.config/ngit/options`): magit-no-confirm and the
//! confirmations of magit-confirm, including those of the abort
//! commands and magit-bisect-reset.

mod common;

use common::*;
use ngit_lib::prelude::*;
use ngit_lib::ui::app::App;
use ngit_lib::ui::options::Options;

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn commit(g: &Git, file: &str, content: &str, msg: &str) {
    write(g, file, content);
    git(g, &["add", "."]);
    git(g, &["commit", "-q", "-m", msg]);
}

fn options(text: &str) -> Options {
    let (o, e) = Options::parse(text);
    assert!(e.is_empty(), "{e:?}");
    o
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
    panic!("no such line");
}

/// A merge of "other" into master stopped by a conflict in file.txt.
fn conflict(g: &Git) {
    git(g, &["checkout", "-q", "-b", "other"]);
    commit(g, "file.txt", "theirs\n", "theirs");
    git(g, &["checkout", "-q", "master"]);
    commit(g, "file.txt", "ours\n", "ours");
    let _ = g.git_interactive(&["merge", "other"]);
    assert!(g.merge_in_progress());
}

#[test]
fn no_confirm() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g.clone());
    // asked by default; declining aborts
    goto(&mut app, |t| t.contains("file.txt"));
    app.script_answers(&["n"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts().len(), 1, "{:?}", app.prompts());
    assert_eq!(app.message(), "Abort");
    assert_eq!(read(&g, "file.txt"), "changed\n");
    // not with discard in magit-no-confirm
    app.set_options(options("magit-no-confirm discard"));
    goto(&mut app, |t| t.contains("file.txt"));
    app.press("k").unwrap();
    assert_eq!(app.prompts().len(), 1, "{:?}", app.prompts());
    assert_ne!(read(&g, "file.txt"), "changed\n");
    // other actions are still confirmed
    write(&g, "new.txt", "new\n");
    git(&g, &["add", "new.txt"]);
    write(&g, "file.txt", "changed\n");
    app.refresh();
    app.script_answers(&["n"]);
    press(&mut app, &["S"]);
    assert_eq!(app.prompts().last().unwrap(), "Stage all changes? (y/n)");
    // t: nothing is confirmed
    app.set_options(options("magit-no-confirm t"));
    let n = app.prompts().len();
    press(&mut app, &["S"]);
    assert_eq!(app.prompts().len(), n);
    assert!(g.git_string(&["diff", "--name-only"]).unwrap().is_empty());
}

#[test]
fn abort_merge() {
    let (_td, g, _) = repo_with_commit();
    conflict(&g);
    let mut app = App::new(g.clone());
    app.script_answers(&["n"]);
    press(&mut app, &["m", "a"]);
    assert_eq!(app.prompts().last().unwrap(), "Abort merge? (y/n)");
    assert_eq!(app.message(), "Abort");
    assert!(g.merge_in_progress());
    app.script_answers(&["y"]);
    press(&mut app, &["m", "a"]);
    assert!(!g.merge_in_progress(), "{}", app.message());
    // magit-no-confirm abort-merge
    conflict2(&g);
    app.set_options(options("magit-no-confirm abort-merge"));
    let n = app.prompts().len();
    press(&mut app, &["m", "a"]);
    assert_eq!(app.prompts().len(), n);
    assert!(!g.merge_in_progress(), "{}", app.message());
}

/// Another conflicting merge after `conflict`.
fn conflict2(g: &Git) {
    git(g, &["checkout", "-q", "other"]);
    commit(g, "file.txt", "theirs 2\n", "theirs 2");
    git(g, &["checkout", "-q", "master"]);
    let _ = g.git_interactive(&["merge", "other"]);
    assert!(g.merge_in_progress());
}

#[test]
fn abort_cherry_pick_and_revert() {
    let (_td, g, _) = repo_with_commit();
    git(&g, &["checkout", "-q", "-b", "other"]);
    commit(&g, "file.txt", "theirs\n", "theirs");
    git(&g, &["checkout", "-q", "master"]);
    commit(&g, "file.txt", "ours\n", "ours");
    let _ = g.git_interactive(&["cherry-pick", "other"]);
    assert!(g.cherry_in_progress());
    let mut app = App::new(g.clone());
    app.script_answers(&["y"]);
    press(&mut app, &["A", "a"]);
    assert_eq!(app.prompts().last().unwrap(), "Really abort cherry-pick? (y/n)");
    assert!(!g.cherry_in_progress(), "{}", app.message());
    commit(&g, "file.txt", "ours 2\n", "ours 2");
    let _ = g.git_interactive(&["revert", "--no-edit", "HEAD~1"]);
    assert!(g.revert_in_progress());
    app.refresh();
    app.script_answers(&["y"]);
    press(&mut app, &["V", "a"]);
    assert_eq!(app.prompts().last().unwrap(), "Really abort revert? (y/n)");
    assert!(!g.revert_in_progress(), "{}", app.message());
}

#[test]
fn bisect_reset() {
    let (_td, g, _) = repo_with_commit();
    commit(&g, "file.txt", "two\n", "two");
    commit(&g, "file.txt", "three\n", "three");
    git(&g, &["bisect", "start", "HEAD", "HEAD~2"]);
    assert!(g.bisect_in_progress());
    let mut app = App::new(g.clone());
    app.script_answers(&["n"]);
    press(&mut app, &["B", "r"]);
    assert_eq!(app.prompts(), ["Reset bisect? (y/n)"]);
    assert!(g.bisect_in_progress());
    app.set_options(options("magit-no-confirm reset-bisect"));
    press(&mut app, &["B", "r"]);
    assert_eq!(app.prompts().len(), 1);
    assert!(!g.bisect_in_progress(), "{}", app.message());
}

#[test]
fn options_file() {
    let (_td, g, _) = repo_with_commit();
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("options");
    let mut app = App::new(g.clone());
    // a missing file: the defaults
    app.load_options(&file);
    assert_eq!(app.message(), "");
    std::fs::write(&file, "; ngit options\nmagit-no-confirm discard\nmagit-frob t\n").unwrap();
    app.load_options(&file);
    assert!(app.message().ends_with("options:3: unknown option magit-frob"), "{}", app.message());
    write(&g, "file.txt", "changed\n");
    app.refresh();
    goto(&mut app, |t| t.contains("file.txt"));
    app.press("k").unwrap();
    assert!(app.prompts().is_empty(), "{:?}", app.prompts());
    assert_ne!(read(&g, "file.txt"), "changed\n");
}

/// Two stashes: stash@{0} "second", stash@{1} "first".
fn stashes(g: &Git) {
    for m in ["first", "second"] {
        write(g, "file.txt", &format!("{m}\n"));
        git(g, &["stash", "push", "-q", "-m", m]);
    }
}

fn stash_list(g: &Git) -> Vec<String> {
    g.stashes().into_iter().map(|(_, m)| m).collect()
}

#[test]
fn read_and_drop_stashes() {
    let (_td, g, _) = repo_with_commit();
    stashes(&g);
    let mut app = App::new(g.clone());
    // magit-read-stash: the first stash the default, completing
    // "stash@{N} subject"
    app.script_keys(&["C-g"]);
    press(&mut app, &["z", "a"]);
    assert_eq!(app.prompts(), ["Apply stash"]);
    app.script_keys(&["s", "t", "a", "s", "h", "@", "{", "1", "TAB", "RET"]);
    press(&mut app, &["z", "p"]);
    assert_eq!(app.prompts()[1], "Pop stash");
    assert_eq!(read(&g, "file.txt"), "first\n", "{}", app.message());
    assert_eq!(stash_list(&g), ["On master: second"]);
    git(&g, &["checkout", "--", "file.txt"]);
    stashes(&g);
    app.refresh();
    // k on a stash: read, the one at point the default; not confirmed
    goto(&mut app, |t| t.trim() == "Stashes (3)");
    app.press("TAB").unwrap();
    goto(&mut app, |t| t.starts_with("stash@{1}"));
    let n = app.prompts().len();
    app.script_answers(&[""]);
    app.press("k").unwrap();
    assert_eq!(app.prompts()[n..], ["Drop stash"]);
    assert!(app.message().starts_with("Deleted refs/stash@{1} (was "), "{}", app.message());
    assert_eq!(stash_list(&g), ["On master: second", "On master: second"]);
    // a region: confirmed (drop-stashes), dropped from the last
    goto(&mut app, |t| t.starts_with("stash@{0}"));
    press(&mut app, &["C-SPC", "C-n"]);
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    let p = app.prompts().last().unwrap().clone();
    assert_eq!(p, "stash@{0}\nstash@{1}\n\nDrop 2 stashes? (y/n)");
    assert!(stash_list(&g).is_empty());
    assert!(app.message().starts_with("Deleted refs/stash@{0} (was "), "{}", app.message());
    // no stashes
    press(&mut app, &["z", "k"]);
    assert_eq!(app.message(), "No stashes");
}

#[test]
fn published_branches() {
    let (_td, g, _) = repo_with_commit();
    commit(&g, "file.txt", "two\n", "two");
    git(&g, &["update-ref", "refs/remotes/origin/master", "HEAD"]);
    git(&g, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    write(&g, "file.txt", "three\n");
    git(&g, &["add", "file.txt"]);
    let head = g.rev_parse("HEAD").unwrap();
    let mut app = App::new(g.clone());
    // magit-published-branches: origin/master
    app.script_answers(&["n"]);
    press(&mut app, &["c", "e"]);
    assert_eq!(
        app.prompts().last().unwrap(),
        "This commit has already been published to origin/master.\nDo you really want to modify it? (y/n)"
    );
    assert_eq!(app.message(), "Abort");
    assert_eq!(g.rev_parse("HEAD").unwrap(), head);
    // several: listed, then counted
    app.set_options(options("magit-published-branches origin/master origin/main"));
    app.script_answers(&["n"]);
    press(&mut app, &["c", "e"]);
    assert_eq!(
        app.prompts().last().unwrap(),
        "origin/master\norigin/main\n\nThis commit has already been published to 2 public branches.\n\
         Do you really want to modify it? (y/n)"
    );
    // rebasing them
    app.script_answers(&["n"]);
    press(&mut app, &["l", "l"]);
    goto(&mut app, |t| t.ends_with(" two"));
    press(&mut app, &["r", "w"]);
    assert_eq!(
        app.prompts().last().unwrap(),
        "origin/master\norigin/main\n\nSome of these commits have already been published to 2 public branches.\n\
         Do you really want to modify them? (y/n)"
    );
    assert_eq!(g.rev_parse("HEAD").unwrap(), head);
    // none: not asked
    app.set_options(options("magit-published-branches nil"));
    app.press("q").unwrap();
    let n = app.prompts().len();
    press(&mut app, &["c", "e"]);
    assert_eq!(app.prompts().len(), n);
    assert_ne!(g.rev_parse("HEAD").unwrap(), head, "{}", app.message());
}
