//! User options (`~/.config/xagit/options`): magit-no-confirm and the
//! confirmations of magit-confirm, including those of the abort
//! commands and magit-bisect-reset.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;
use xagit_lib::ui::options::Options;

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
    panic!("no such line: {:?}", app.visible_lines().iter().map(|l| l.text.clone()).collect::<Vec<_>>());
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
    assert_eq!(app.prompts().last().unwrap(), "Stage all changes? (y or n) ");
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
    assert_eq!(app.prompts().last().unwrap(), "Abort merge? (y or n) ");
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
    assert_eq!(app.prompts().last().unwrap(), "Really abort cherry-pick? (y or n) ");
    assert!(!g.cherry_in_progress(), "{}", app.message());
    commit(&g, "file.txt", "ours 2\n", "ours 2");
    let _ = g.git_interactive(&["revert", "--no-edit", "HEAD~1"]);
    assert!(g.revert_in_progress());
    app.refresh();
    app.script_answers(&["y"]);
    press(&mut app, &["V", "a"]);
    assert_eq!(app.prompts().last().unwrap(), "Really abort revert? (y or n) ");
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
    assert_eq!(app.prompts(), ["Reset bisect? (y or n) "]);
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
    std::fs::write(&file, "; xagit options\nmagit-no-confirm discard\nmagit-frob t\n").unwrap();
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
    // a region: confirmed (drop-stashes) with "yes or no"
    // (magit-slow-confirm), dropped from the last
    goto(&mut app, |t| t.starts_with("stash@{0}"));
    press(&mut app, &["C-SPC", "C-n"]);
    let n = app.prompts().len();
    app.script_answers(&["y", "yes"]);
    app.press("k").unwrap();
    let p = "stash@{0}\nstash@{1}\n\nDrop 2 stashes? (yes or no) ";
    assert_eq!(app.prompts()[n..], [p, p], "y isn't an answer");
    assert!(stash_list(&g).is_empty());
    assert!(app.message().starts_with("Deleted refs/stash@{0} (was "), "{}", app.message());
    // no stashes
    press(&mut app, &["z", "k"]);
    assert_eq!(app.message(), "No stashes");
}

#[test]
fn y_or_n_p() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g.clone());
    goto(&mut app, |t| t.starts_with("modified"));
    // only y/Y/SPC and n/N/DEL answer: "yes", RET (empty) ask again
    app.script_answers(&["yes", "", "N"]);
    app.press("k").unwrap();
    let p = "Discard unstaged changes in \"file.txt\"? (y or n) ";
    assert_eq!(app.prompts(), [p; 3]);
    assert_eq!(app.message(), "Abort");
    assert_eq!(read(&g, "file.txt"), "changed\n");
    // the key answers at once, the answer echoed
    app.script_keys(&["Y"]);
    app.press("k").unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert_eq!(app.message(), format!("{p}y"));
    write(&g, "file.txt", "changed\n");
    app.refresh();
    goto(&mut app, |t| t.starts_with("modified"));
    app.script_keys(&["x", "backspace"]);
    app.press("k").unwrap();
    assert_eq!(app.message(), "Abort");
    assert_eq!(read(&g, "file.txt"), "changed\n");
}

#[test]
fn slow_confirm() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g.clone());
    // magit-slow-confirm t: all of magit-confirm asks "yes or no"
    app.set_options(options("magit-slow-confirm t"));
    goto(&mut app, |t| t.starts_with("modified"));
    app.script_answers(&["n", "no"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts(), ["Discard unstaged changes in \"file.txt\"? (yes or no) "; 2]);
    assert_eq!(app.message(), "Abort");
    assert_eq!(read(&g, "file.txt"), "changed\n");
    app.script_answers(&["YES"]);
    app.press("k").unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n", "{}", app.message());
    // by action
    write(&g, "file.txt", "changed\n");
    app.set_options(options("magit-slow-confirm stage-all-changes"));
    goto(&mut app, |t| t.starts_with("modified"));
    app.script_answers(&["n"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "Discard unstaged changes in \"file.txt\"? (y or n) ");
    // (confirmed when something is staged already)
    write(&g, "b.txt", "b\n");
    git(&g, &["add", "b.txt"]);
    app.refresh();
    app.script_answers(&["yes"]);
    app.press("S").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "Stage all changes? (yes or no) ");
    assert_eq!(g.staged_files(), ["b.txt", "file.txt"]);
    git(&g, &["reset", "-q", "--", "b.txt"]);
    std::fs::remove_file(g.toplevel.join("b.txt")).unwrap();
    // nil: drop-stashes asks "y or n"
    app.set_options(options("magit-slow-confirm nil"));
    git(&g, &["stash"]);
    git(&g, &["stash", "apply"]);
    git(&g, &["stash"]);
    app.refresh();
    goto(&mut app, |t| t.trim() == "Stashes (2)");
    app.press("TAB").unwrap();
    goto(&mut app, |t| t.starts_with("stash@{0}"));
    press(&mut app, &["C-SPC", "C-n"]);
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "stash@{0}\nstash@{1}\n\nDrop 2 stashes? (y or n) ");
    assert!(stash_list(&g).is_empty());
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
        "This commit has already been published to origin/master.\nDo you really want to modify it? (y or n) "
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
         Do you really want to modify it? (y or n) "
    );
    // rebasing them
    app.script_answers(&["n"]);
    press(&mut app, &["l", "l"]);
    goto(&mut app, |t| t.ends_with(" two"));
    press(&mut app, &["r", "w"]);
    assert_eq!(
        app.prompts().last().unwrap(),
        "origin/master\norigin/main\n\nSome of these commits have already been published to 2 public branches.\n\
         Do you really want to modify them? (y or n) "
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

#[test]
fn branch_reset() {
    let (_td, g, first) = repo_with_commit();
    commit(&g, "file.txt", "two\n", "two");
    let two = g.rev_parse("HEAD").unwrap();
    git(&g, &["branch", "other"]);
    let mut app = App::new(g.clone());
    // another branch: update-ref with a reflog message
    app.script_answers(&["other", &first]);
    press(&mut app, &["b", "x"]);
    assert_eq!(app.prompts(), ["Reset branch", "Reset other to"]);
    assert_eq!(g.rev_parse("other").unwrap(), first, "{}", app.message());
    let reflog = g.git_string(&["reflog", "-1", "--format=%gs", "other"]).unwrap();
    assert_eq!(reflog, format!("reset: moving to {first}"));
    // the current branch with uncommitted changes: yes-or-no-p
    write(&g, "file.txt", "changed\n");
    app.script_answers(&["master", "other", "n", "no"]);
    press(&mut app, &["b", "x"]);
    assert_eq!(app.prompts()[3..], ["Reset master to", "Uncommitted changes will be lost.  Proceed? (yes or no) ", "Uncommitted changes will be lost.  Proceed? (yes or no) "]);
    assert_eq!(app.message(), "Abort");
    assert_eq!(g.rev_parse("HEAD").unwrap(), two);
    app.script_answers(&["master", "other", "yes"]);
    press(&mut app, &["b", "x"]);
    assert_eq!(g.rev_parse("HEAD").unwrap(), first, "{}", app.message());
    assert_eq!(read(&g, "file.txt"), "hello\n", "a hard reset");
    // untracked files aren't uncommitted changes: not confirmed
    write(&g, "new.txt", "new\n");
    let n = app.prompts().len();
    app.script_answers(&["master", &two]);
    press(&mut app, &["b", "x"]);
    assert_eq!(app.prompts().len(), n + 2);
    assert_eq!(g.rev_parse("HEAD").unwrap(), two);
}

/// The names in DIR, sorted.
fn names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => Vec::new(),
    };
    v.sort();
    v
}

#[test]
fn discard_untracked_to_trash() {
    let (_td, g, _) = repo_with_commit();
    let ttd = tempfile::tempdir().unwrap();
    let trash = ttd.path().join("trash");
    let mut app = App::new(g.clone());
    app.set_options(options(&format!("trash-directory {}", trash.display())));
    write(&g, "junk.txt", "junk\n");
    app.refresh();
    goto(&mut app, |t| t.starts_with("Untracked files"));
    app.press("TAB").unwrap();
    goto(&mut app, |t| t.trim() == "junk.txt");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts(), ["Trash \"junk.txt\"? (y or n) "]);
    assert!(!exists(&g, "junk.txt"));
    assert_eq!(names(&trash), ["junk.txt"]);
    // magit-no-confirm discard doesn't cover trash
    app.set_options(options(&format!("trash-directory {}\nmagit-no-confirm discard", trash.display())));
    write(&g, "junk.txt", "again\n");
    app.refresh();
    goto(&mut app, |t| t.trim() == "junk.txt");
    app.script_answers(&["n"]);
    app.press("k").unwrap();
    assert_eq!(app.message(), "Abort");
    assert!(exists(&g, "junk.txt"));
    app.set_options(options(&format!("trash-directory {}\nmagit-no-confirm trash", trash.display())));
    goto(&mut app, |t| t.trim() == "junk.txt");
    let n = app.prompts().len();
    app.press("k").unwrap();
    assert_eq!(app.prompts().len(), n);
    assert_eq!(names(&trash), ["junk.txt", "junk.txt.~1~"]);
    // a directory: recursively
    std::fs::create_dir(g.toplevel.join("dir")).unwrap();
    write(&g, "dir/a.txt", "a\n");
    app.refresh();
    goto(&mut app, |t| t.trim() == "dir/");
    app.script_answers(&["what", "no"]);
    app.press("k").unwrap();
    let q = "Recursively trash dir/? (yes, no, all, quit, help)";
    assert_eq!(app.prompts()[n..], [q, q]);
    assert_eq!(app.message(), "Directory is not empty, not moving to trash");
    assert!(exists(&g, "dir/a.txt"));
    app.script_answers(&["yes"]);
    app.press("k").unwrap();
    assert!(!exists(&g, "dir"));
    assert_eq!(names(&trash.join("dir")), ["a.txt"]);
}

#[test]
fn discard_untracked_delete() {
    let (_td, g, _) = repo_with_commit();
    let ttd = tempfile::tempdir().unwrap();
    let trash = ttd.path().join("trash");
    let mut app = App::new(g.clone());
    app.set_options(options(&format!("magit-delete-by-moving-to-trash nil\ntrash-directory {}", trash.display())));
    write(&g, "a.txt", "a\n");
    write(&g, "b.txt", "b\n");
    app.refresh();
    goto(&mut app, |t| t.starts_with("Untracked files"));
    app.press("TAB").unwrap();
    goto(&mut app, |t| t.trim() == "a.txt");
    press(&mut app, &["C-SPC", "C-n"]);
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts(), ["a.txt\nb.txt\n\nDelete 2 files? (y or n) "]);
    assert!(!exists(&g, "a.txt") && !exists(&g, "b.txt"));
    assert!(names(&trash).is_empty(), "deleted, not trashed");
    // the untracked section: magit-list-untracked-files, directories
    // deleted recursively if confirmed
    std::fs::create_dir(g.toplevel.join("dir")).unwrap();
    write(&g, "dir/c.txt", "c\n");
    write(&g, "d.txt", "d\n");
    app.refresh();
    goto(&mut app, |t| t.starts_with("Untracked files"));
    app.script_answers(&["y", "quit"]);
    app.press("k").unwrap();
    let n = app.prompts().len();
    assert_eq!(
        app.prompts()[n - 2..],
        ["d.txt\ndir/\n\nDelete 2 files? (y or n) ", "Recursively delete dir/? (yes, no, all, quit, help)"]
    );
    assert_eq!(app.message(), "Quit");
    assert!(!exists(&g, "d.txt") && exists(&g, "dir/c.txt"));
    app.script_answers(&["y", "all"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "Recursively delete dir/? (yes, no, all, quit, help)");
    assert!(!exists(&g, "dir"), "{}", app.message());
}

#[test]
fn worktree_delete() {
    let (_td, g, _) = repo_with_commit();
    let ttd = tempfile::tempdir().unwrap();
    let trash = ttd.path().join("trash");
    let wt = ttd.path().join("wt");
    let wt2 = ttd.path().join("wt2");
    let wt3 = ttd.path().join("wt3");
    for (w, b) in [(&wt, "a"), (&wt2, "b"), (&wt3, "c")] {
        git(&g, &["worktree", "add", "-q", "-b", b, &w.to_string_lossy()]);
    }
    let mut app = App::new(g.clone());
    app.set_options(options(&format!("trash-directory {}", trash.display())));
    let path = |p: &std::path::Path| p.canonicalize().unwrap().to_string_lossy().into_owned();
    // the primary worktree isn't a choice
    app.script_answers(&[&path(g.toplevel.as_path())]);
    press(&mut app, &["%", "k"]);
    assert_eq!(app.message(), "[No match]");
    // trashed, then pruned
    let wt_path = path(&wt);
    app.script_answers(&[&wt_path, "y"]);
    press(&mut app, &["%", "k"]);
    assert_eq!(app.prompts().last().unwrap(), "Trash worktree \"wt\"? (y or n) ");
    assert!(!wt.exists() && trash.join("wt").is_dir(), "{}", app.message());
    let list = g.git_string(&["worktree", "list", "--porcelain"]).unwrap();
    assert!(!list.contains(&format!("worktree {wt_path}\n")), "pruned: {list}");
    // uncommitted changes: danger (not covered by magit-no-confirm)
    std::fs::write(wt2.join("new.txt"), "x").unwrap();
    app.set_options(options(&format!("trash-directory {}\nmagit-no-confirm trash", trash.display())));
    app.script_answers(&[&path(&wt2), "n"]);
    press(&mut app, &["%", "k"]);
    assert_eq!(app.prompts().last().unwrap(), "Trash worktree \"wt2\" despite uncommitted changes? (y or n) ");
    assert_eq!(app.message(), "Abort");
    assert!(wt2.exists());
    // locked
    git(&g, &["worktree", "lock", "--reason", "busy", &wt3.to_string_lossy()]);
    app.script_answers(&[&path(&wt3)]);
    press(&mut app, &["%", "k"]);
    assert_eq!(app.message(), format!("Cannot delete locked {} [reason: busy]", path(&wt3)));
    // without the trash: git worktree remove --force
    app.set_options(options(&format!("trash-directory {}\nmagit-delete-by-moving-to-trash nil", trash.display())));
    app.script_answers(&[&path(&wt2), "y"]);
    press(&mut app, &["%", "k"]);
    assert_eq!(app.prompts().last().unwrap(), "Delete worktree \"wt2\" despite uncommitted changes? (y or n) ");
    assert!(!wt2.exists(), "{}", app.message());
    assert!(!trash.join("wt2").exists());
    assert!(!g.git_string(&["worktree", "list"]).unwrap().contains("wt2"));
}

#[test]
fn worktree_delete_current() {
    let (_td, g, _) = repo_with_commit();
    let ttd = tempfile::tempdir().unwrap();
    let trash = ttd.path().join("trash");
    let wt = ttd.path().join("wt");
    git(&g, &["worktree", "add", "-q", "-b", "a", &wt.to_string_lossy()]);
    let wg = Git::discover(&wt).unwrap();
    let mut app = App::new(wg.clone());
    app.set_options(options(&format!("trash-directory {}", trash.display())));
    // the status of the worktree, a diff buffer buried below it
    press(&mut app, &["d", "d"]);
    app.press("q").unwrap();
    let wt_path = wg.toplevel.to_string_lossy().into_owned();
    app.script_answers(&[&wt_path, "y"]);
    press(&mut app, &["%", "k"]);
    assert!(!wt.exists(), "{}", app.message());
    // pruned from the primary worktree, whose status is shown
    assert!(!g.git_string(&["worktree", "list"]).unwrap().contains("/wt"));
    assert_eq!(app.git.toplevel, g.toplevel);
    assert_eq!(app.cwd, g.toplevel);
    assert!(app.visible_lines()[0].text.starts_with("Head:"), "{:?}", app.visible_lines()[0].text);
    assert!(app.visible_lines().iter().any(|l| l.text.contains("master")));
}
