//! The sequence sections of the status buffer (magit-insert-rebase-sequence,
//! magit-insert-sequencer-sequence): the type words of the lines and their
//! magit-sequence-* faces, and how the stopped commit is shown.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;
use xagit_lib::ui::theme::Face;

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn texts(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

/// The face given to the first word of the line starting with WORD.
fn word_face(app: &App, word: &str) -> Option<Face> {
    let l = app.visible_lines().iter().find(|l| l.text.starts_with(&format!("{word} ")))?;
    l.faces.iter().find(|(a, b, _)| *a == 0 && *b == word.len()).map(|(_, _, f)| *f)
}

/// topic (t1, t2, t3) rebased onto master; t2 conflicts.
fn rebase_conflict() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    write(&g, "f.txt", "base\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "base"]);
    git(&g, &["checkout", "-q", "-b", "topic"]);
    write(&g, "t1.txt", "1\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "t1"]);
    write(&g, "f.txt", "topic\n");
    git(&g, &["commit", "-q", "-am", "t2"]);
    write(&g, "t3.txt", "3\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "t3"]);
    git(&g, &["checkout", "-q", "master"]);
    write(&g, "f.txt", "master\n");
    git(&g, &["commit", "-q", "-am", "m"]);
    git(&g, &["checkout", "-q", "topic"]);
    let _ = g.git_string(&["rebase", "master"]);
    assert!(g.rebase_in_progress());
    (td, g)
}

#[test]
fn rebase_sequence_types_and_faces() {
    let (_td, g) = rebase_conflict();
    let mut app = App::new(g.clone());
    app.press("M-4").unwrap();
    let t = texts(&app);
    assert!(t.iter().any(|l| l.starts_with("Rebasing topic onto master")), "{t:?}");
    // the remaining step, the conflicted one, the done one and onto
    assert!(t.iter().any(|l| l.starts_with("pick ") && l.ends_with(" t3")), "{t:?}");
    assert!(t.iter().any(|l| l.starts_with("join ") && l.ends_with(" t2")), "{t:?}");
    assert!(t.iter().any(|l| l.starts_with("done ") && l.ends_with(" t1")), "{t:?}");
    assert!(t.iter().any(|l| l.starts_with("onto ") && l.ends_with(" m")), "{t:?}");
    // magit-sequence-pick / -part / -head (t1 is HEAD) / -onto
    assert_eq!(word_face(&app, "pick"), Some(Face::SequencePick));
    assert_eq!(word_face(&app, "join"), Some(Face::SequencePart));
    assert_eq!(word_face(&app, "done"), Some(Face::SequenceHead));
    assert_eq!(word_face(&app, "onto"), Some(Face::SequenceOnto));
}

#[test]
fn stopped_commit_states() {
    let (_td, g) = rebase_conflict();
    // resolved with the commit's own version and staged, not committed:
    // committing would give the same tree as the stopped commit -> goal
    write(&g, "f.txt", "topic\n");
    git(&g, &["add", "f.txt"]);
    let mut app = App::new(g.clone());
    app.press("M-4").unwrap();
    assert!(texts(&app).iter().any(|l| l.starts_with("goal ") && l.ends_with(" t2")), "{:?}", texts(&app));
    assert_eq!(word_face(&app, "goal"), Some(Face::SequencePart));
    // a different resolution -> work
    write(&g, "f.txt", "something else\n");
    git(&g, &["add", "f.txt"]);
    app.refresh();
    assert!(texts(&app).iter().any(|l| l.starts_with("work ") && l.ends_with(" t2")), "{:?}", texts(&app));
    // nothing left of the commit's changes -> gone
    git(&g, &["checkout", "-q", "HEAD", "--", "f.txt"]);
    app.refresh();
    assert!(texts(&app).iter().any(|l| l.starts_with("gone ") && l.ends_with(" t2")), "{:?}", texts(&app));
    assert_eq!(word_face(&app, "gone"), Some(Face::SequenceDrop));
}

#[test]
fn edit_stop_is_stop_with_head_face() {
    let (_td, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    for i in 1..=3 {
        write(&g, &format!("f{i}"), "x\n");
        git(&g, &["add", "."]);
        git(&g, &["commit", "-q", "-m", &format!("c{i}")]);
    }
    let out = std::process::Command::new("git")
        .args(["rebase", "-i", "HEAD~2"])
        .env("GIT_SEQUENCE_EDITOR", "sed -i.bak '1s/^pick/edit/'")
        .current_dir(&g.toplevel)
        .output()
        .unwrap();
    assert!(g.rebase_in_progress(), "{}", String::from_utf8_lossy(&out.stderr));
    let mut app = App::new(g.clone());
    app.press("M-4").unwrap();
    // stopped at c2 for editing, nothing done yet: "stop" in magit-sequence-head
    assert!(texts(&app).iter().any(|l| l.starts_with("stop ") && l.ends_with(" c2")), "{:?}", texts(&app));
    assert_eq!(word_face(&app, "stop"), Some(Face::SequenceHead));
    // a new commit on top of it: there are new commits -> "like", in
    // magit-sequence-stop (the stop is no longer HEAD)
    write(&g, "extra", "x\n");
    git(&g, &["add", "extra"]);
    git(&g, &["commit", "-q", "-m", "extra"]);
    app.refresh();
    assert!(texts(&app).iter().any(|l| l.starts_with("like ") && l.ends_with(" c2")), "{:?}", texts(&app));
    assert_eq!(word_face(&app, "like"), Some(Face::SequenceStop));
    // amending it instead changes its patch: the stop is "gone"
    git(&g, &["reset", "-q", "--hard", "HEAD~"]);
    write(&g, "f2", "amended\n");
    git(&g, &["commit", "-q", "-a", "--amend", "--no-edit"]);
    app.refresh();
    assert!(texts(&app).iter().any(|l| l.starts_with("gone ") && l.ends_with(" c2")), "{:?}", texts(&app));
    git(&g, &["rebase", "--abort"]);
}
