//! magit-discard on file sections (magit-discard-files): each file is
//! handled by its status, each kind confirmed on its own: conflicts are
//! resolved, deletions resurrected, new files deleted (to the trash),
//! renames undone, other changes discarded.

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

/// The file section TEXT of the section starting with HEADING.
fn goto_file(app: &mut App, heading: &str, text: &str) {
    app.refresh();
    goto(app, |t| t.starts_with(heading));
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-n").unwrap();
        if app.point_text().trim() == text {
            return;
        }
    }
    panic!("no {text} after {heading}: {:?}", app.visible_lines().iter().map(|l| l.text.clone()).collect::<Vec<_>>());
}

/// An app whose trash is a directory of TD.
fn app_with_trash(g: &Git, trash: &std::path::Path) -> App {
    let mut app = App::new(g.clone());
    let (o, e) = Options::parse(&format!("trash-directory {}", trash.display()));
    assert!(e.is_empty());
    app.set_options(o);
    app
}

fn status(g: &Git) -> String {
    g.git_string(&["status", "--porcelain"]).unwrap()
}

#[test]
fn staged_new_files() {
    let (_td, g, _) = repo_with_commit();
    let ttd = tempfile::tempdir().unwrap();
    let mut app = app_with_trash(&g, &ttd.path().join("trash"));
    // added: trashed and removed from the index
    write(&g, "new.txt", "new\n");
    git(&g, &["add", "new.txt"]);
    goto_file(&mut app, "Staged changes", "new file   new.txt");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts(), ["Trash \"new.txt\"? (y or n) "]);
    assert!(!exists(&g, "new.txt"));
    assert!(ttd.path().join("trash/new.txt").is_file());
    assert_eq!(status(&g), "");
    // added and modified: discard-new, the file becomes untracked
    write(&g, "new.txt", "new\n");
    git(&g, &["add", "new.txt"]);
    write(&g, "new.txt", "changed\n");
    goto_file(&mut app, "Staged changes", "new file   new.txt");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "Discard staged changes in \"new.txt\"? (y or n) ");
    assert_eq!(status(&g), "?? new.txt");
    assert_eq!(read(&g, "new.txt"), "changed\n");
}

#[test]
fn staged_changes_keep_unstaged_ones() {
    let (_td, g, _) = repo_with_commit();
    commit(&g, "f.txt", "a\nb\nc\nd\ne\nf\ng\n", "f");
    write(&g, "f.txt", "A\nb\nc\nd\ne\nf\ng\n");
    git(&g, &["add", "f.txt"]);
    write(&g, "f.txt", "A\nb\nc\nd\ne\nf\nG\n");
    let mut app = App::new(g.clone());
    goto_file(&mut app, "Staged changes", "modified   f.txt");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts(), ["Discard staged changes in \"f.txt\"? (y or n) "]);
    assert_eq!(read(&g, "f.txt"), "a\nb\nc\nd\ne\nf\nG\n", "the unstaged change stays");
    assert_eq!(status(&g), " M f.txt");
    // declined: nothing changes
    goto_file(&mut app, "Unstaged changes", "modified   f.txt");
    app.script_answers(&["n"]);
    app.press("k").unwrap();
    assert_eq!(app.message(), "Abort");
    assert_eq!(status(&g), " M f.txt");
}

#[test]
fn resurrect_deleted_files() {
    let (_td, g, _) = repo_with_commit();
    git(&g, &["rm", "-q", "file.txt"]);
    let mut app = App::new(g.clone());
    // staged: unstaged only (git reset)
    goto_file(&mut app, "Staged changes", "deleted    file.txt");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts(), ["Resurrect \"file.txt\"? (y or n) "]);
    assert_eq!(status(&g), " D file.txt");
    // unstaged: checked out
    goto_file(&mut app, "Unstaged changes", "deleted    file.txt");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts().last().unwrap(), "Resurrect \"file.txt\"? (y or n) ");
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert_eq!(status(&g), "");
}

#[test]
fn undo_renames() {
    let (_td, g, _) = repo_with_commit();
    std::fs::create_dir(g.toplevel.join("sub")).unwrap();
    git(&g, &["mv", "file.txt", "sub/moved.txt"]);
    let mut app = App::new(g.clone());
    goto_file(&mut app, "Staged changes", "renamed    file.txt -> sub/moved.txt");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts(), ["Undo rename file.txt -> sub/moved.txt? (y or n) "]);
    assert_eq!(status(&g), "");
    assert_eq!(read(&g, "file.txt"), "hello\n");
    // the renamed file is gone: the source is unstaged again
    git(&g, &["mv", "file.txt", "moved.txt"]);
    std::fs::remove_file(g.toplevel.join("moved.txt")).unwrap();
    goto_file(&mut app, "Staged changes", "renamed    file.txt -> moved.txt");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(status(&g), " D file.txt");
}

#[test]
fn several_kinds() {
    let (_td, g, _) = repo_with_commit();
    let ttd = tempfile::tempdir().unwrap();
    commit(&g, "a.txt", "a\n", "a");
    write(&g, "a.txt", "A\n");
    git(&g, &["rm", "-q", "file.txt"]);
    write(&g, "n.txt", "n\n");
    git(&g, &["add", "."]);
    let mut app = app_with_trash(&g, &ttd.path().join("trash"));
    goto(&mut app, |t| t.starts_with("Staged changes"));
    // all files of the section: resurrect, delete, discard, in that order
    app.script_answers(&["y", "y", "y"]);
    app.press("k").unwrap();
    assert_eq!(
        app.prompts(),
        ["Resurrect \"file.txt\"? (y or n) ", "Trash \"n.txt\"? (y or n) ", "Discard staged changes in \"a.txt\"? (y or n) "]
    );
    assert_eq!(status(&g), " D file.txt");
    assert_eq!(read(&g, "a.txt"), "a\n");
    // declining one kind stops
    write(&g, "a.txt", "A\n");
    write(&g, "n.txt", "n\n");
    git(&g, &["add", "."]);
    app.refresh();
    goto(&mut app, |t| t.starts_with("Staged changes"));
    let k = app.prompts().len();
    app.script_answers(&["y", "n"]);
    app.press("k").unwrap();
    assert_eq!(app.prompts().len(), k + 2);
    assert_eq!(app.message(), "Abort");
    assert!(status(&g).contains("M  a.txt"), "{}", status(&g));
}

/// A merge of "other" into master stopped by conflicts in FILES.
fn conflicts(g: &Git, files: &[&str]) {
    git(g, &["checkout", "-q", "-b", "other"]);
    for f in files {
        commit(g, f, "theirs\n", "theirs");
    }
    git(g, &["checkout", "-q", "master"]);
    for f in files {
        commit(g, f, "ours\n", "ours");
    }
    assert!(g.git_interactive(&["merge", "-q", "other"]).is_err());
}

#[test]
fn resolve_conflicts() {
    let (_td, g, _) = repo_with_commit();
    conflicts(&g, &["c1.txt", "c2.txt", "c3.txt"]);
    let mut app = App::new(g.clone());
    goto_file(&mut app, "Unstaged changes", "unmerged   c1.txt (both added)");
    app.script_answers(&["x", "t"]);
    app.press("k").unwrap();
    let p = "For c1.txt checkout: [o]ur stage, [t]heir stage, [c]onflict, or [C-g] to abort ";
    assert_eq!(app.prompts(), [p, p]);
    assert_eq!(read(&g, "c1.txt"), "theirs\n");
    assert!(status(&g).contains("M  c1.txt"), "staged: {}", status(&g));
    // several: one stage for all, or decided individually
    goto_file(&mut app, "Unstaged changes", "unmerged   c2.txt (both added)");
    press(&mut app, &["C-SPC", "C-n"]);
    app.script_answers(&["i", "o", "c"]);
    app.press("k").unwrap();
    let n = app.prompts().len();
    assert_eq!(
        app.prompts()[n - 3..],
        [
            "For these 2 files\n  c2.txt\n  c3.txt\ncheckout:\n[o]ur stage, [t]heir stage, [c]onflict, decide [i]ndividually, or [C-g] to abort ",
            "For c2.txt checkout: [o]ur stage, [t]heir stage, [c]onflict, or [C-g] to abort ",
            "For c3.txt checkout: [o]ur stage, [t]heir stage, [c]onflict, or [C-g] to abort ",
        ]
    );
    assert_eq!(read(&g, "c2.txt"), "ours\n");
    assert!(read(&g, "c3.txt").contains("<<<<<<<"), "the conflict restored");
    assert!(status(&g).contains("AA c3.txt"), "{}", status(&g));
}

#[test]
fn resolve_deleted_side() {
    let (_td, g, _) = repo_with_commit();
    git(&g, &["checkout", "-q", "-b", "other"]);
    git(&g, &["rm", "-q", "file.txt"]);
    git(&g, &["commit", "-q", "-m", "rm"]);
    git(&g, &["checkout", "-q", "master"]);
    commit(&g, "file.txt", "changed\n", "change");
    assert!(g.git_interactive(&["merge", "-q", "other"]).is_err());
    let mut app = App::new(g.clone());
    goto_file(&mut app, "Unstaged changes", "unmerged   file.txt (deleted by them)");
    app.script_answers(&["t"]);
    app.press("k").unwrap();
    assert!(!exists(&g, "file.txt"));
    assert_eq!(status(&g), "D  file.txt");
}

#[test]
fn staged_binary_files() {
    let (_td, g, _) = repo_with_commit();
    commit(&g, "bin.dat", "a\0b", "bin");
    write(&g, "bin.dat", "c\0d");
    git(&g, &["add", "bin.dat"]);
    let mut app = App::new(g.clone());
    // without unstaged changes: unstaged only
    goto_file(&mut app, "Staged changes", "modified   bin.dat (binary)");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(status(&g), " M bin.dat");
    assert_eq!(read(&g, "bin.dat"), "c\0d");
    // with unstaged changes: refused
    git(&g, &["add", "bin.dat"]);
    write(&g, "bin.dat", "e\0f");
    goto_file(&mut app, "Staged changes", "modified   bin.dat (binary)");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(
        app.message(),
        "Cannot discard staged changes to binary files, which also have unstaged changes.  Unstage instead."
    );
    assert_eq!(status(&g), "MM bin.dat");
}

/// magit-confirm-files with several files: only a single, counted
/// question is shown in the minibuffer; the file list is not displayed.
#[test]
fn multi_file_confirm_question_only() {
    let (_td, g, _) = repo_with_commit();
    for f in ["a.txt", "b.txt", "c.txt"] {
        write(&g, f, "changed\n");
        git(&g, &["add", f]);
    }
    // make them unstaged modifications (tracked, committed, then changed)
    git(&g, &["commit", "-q", "-m", "add three"]);
    for f in ["a.txt", "b.txt", "c.txt"] {
        write(&g, f, "modified\n");
    }
    let mut app = App::new(g.clone());
    // select the three file lines of the Unstaged changes section
    goto_file(&mut app, "Unstaged changes", "modified   a.txt");
    press(&mut app, &["C-SPC", "C-n", "C-n"]);
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    let p = app.prompts().last().cloned().unwrap_or_default();
    assert!(!p.contains('\n'), "multi-line prompt leaked into the minibuffer: {p:?}");
    assert!(!p.contains("a.txt") && !p.contains("b.txt") && !p.contains("c.txt"),
        "file list leaked into the prompt: {p:?}");
    assert!(p.contains("3 files") && p.contains("(y or n)"), "wrong question: {p:?}");
    // all three files were discarded
    assert_eq!(read(&g, "a.txt"), "changed\n");
    assert_eq!(read(&g, "b.txt"), "changed\n");
    assert_eq!(read(&g, "c.txt"), "changed\n");
}
