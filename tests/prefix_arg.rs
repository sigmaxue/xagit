//! The prefix argument (C-u, C-u C-u, C-u N, M--, M-N, C-N) and the
//! commands whose behavior it changes (current-prefix-arg in Magit).

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode, PrefixArg};

fn texts(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

/// Move to the first visible line for which F holds.
fn goto_by(app: &mut App, what: &str, f: impl Fn(&str) -> bool) {
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-p").unwrap();
    }
    for _ in 0..n {
        if f(app.point_text()) {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no line {what:?} in {:#?}", texts(app));
}

fn goto(app: &mut App, text: &str) {
    goto_by(app, text, |t| t == text);
}

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn commit_file(g: &Git, file: &str, content: &str, msg: &str) {
    write(g, file, content);
    git(g, &["add", "."]);
    git(g, &["commit", "-q", "-m", msg]);
}

fn staged(g: &Git) -> Vec<String> {
    g.git_lines(&["diff", "--cached", "--name-only", "-z"])
}

fn tracked(g: &Git) -> Vec<String> {
    g.git_lines(&["ls-files", "-z"])
}

fn last_prompt(app: &App) -> String {
    app.prompts().last().cloned().unwrap_or_default()
}

#[test]
fn universal_argument() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g);
    app.press("C-u").unwrap();
    assert_eq!(app.message(), "C-u-");
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Raw(4)));
    app.press("C-u").unwrap();
    assert_eq!(app.message(), "C-u C-u-");
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Raw(16)));
    app.press("C-g").unwrap();
    assert_eq!(app.message(), "Quit");
    assert_eq!(app.current_prefix_arg(), None);
    // digits after C-u
    press(&mut app, &["C-u", "1", "2"]);
    assert_eq!(app.message(), "C-u 12-");
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Num(12)));
    app.press("C-g").unwrap();
    // - right after C-u, and digits after it
    press(&mut app, &["C-u", "-"]);
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Minus));
    assert_eq!(app.current_prefix_arg().unwrap().numeric(), -1);
    app.press("3").unwrap();
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Num(-3)));
    app.press("C-g").unwrap();
    app.press("M--").unwrap();
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Minus));
    app.press("C-g").unwrap();
    // the argument is for the next command only
    press(&mut app, &["C-u", "g"]);
    assert_eq!(app.current_prefix_arg(), None);
    // ... which may be a key sequence
    press(&mut app, &["C-u", "C-c"]);
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Raw(4)));
    app.press("M-g").unwrap();
    assert_eq!(app.current_menu(), Some("file-dispatch"));
    assert_eq!(app.current_prefix_arg(), None, "consumed by the prefix command");
    // in a transient C-u is the argument of a suffix
    app.press("C-u").unwrap();
    assert_eq!(app.current_menu(), Some("file-dispatch"));
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Raw(4)));
    app.press("C-g").unwrap();
    assert_eq!(app.current_menu(), Some("file-dispatch"), "C-g only cancels the argument");
    app.press("C-g").unwrap();
    assert_eq!(app.current_menu(), None);
}

#[test]
fn digit_and_negative_argument() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g);
    // digit-argument: M-N and C-N, then digits continue
    app.press("M-5").unwrap();
    assert_eq!(app.message(), "C-u 5-");
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Num(5)));
    app.press("2").unwrap();
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Num(52)));
    app.press("C-g").unwrap();
    assert_eq!(app.current_prefix_arg(), None);
    press(&mut app, &["C-3", "C-M-7"]);
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Num(37)));
    app.press("M--").unwrap();
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Num(-37)));
    app.press("C-g").unwrap();
    // negative-argument: M--, C--, C-M--; again cancels it
    press(&mut app, &["C--", "M-0"]);
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Minus), "-0 is just -");
    app.press("M-6").unwrap();
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Num(-6)));
    app.press("C-g").unwrap();
    press(&mut app, &["M--", "C-M--"]);
    assert_eq!(app.current_prefix_arg(), None);
    app.press("C-g").unwrap();
    // - in universal-argument-map is negative-argument before digits
    press(&mut app, &["C-u", "-", "-"]);
    assert_eq!(app.current_prefix_arg(), None);
    app.press("3").unwrap();
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Num(3)));
    app.press("C-g").unwrap();
    // C-u after - and a digit after that
    press(&mut app, &["C-u", "-", "C-u"]);
    assert_eq!(app.message(), "C-u (-4)-");
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Raw(-4)));
    app.press("2").unwrap();
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Num(2)));
    app.press("C-g").unwrap();
    // M-1..M-4 are magit-section-show-level-N-all in Magit buffers
    app.press("M-1").unwrap();
    assert_eq!(app.current_prefix_arg(), None);
    // the argument of the next command
    let context = |app: &App| app.buffer_args().iter().find(|a| a.starts_with("-U")).cloned();
    press(&mut app, &["C-2", "+"]);
    assert_eq!(context(&app).as_deref(), Some("-U5"));
    assert_eq!(app.current_prefix_arg(), None);
}

#[test]
fn digit_argument_in_file_buffer() {
    let (_td, g) = file_repo();
    let mut app = App::new(g);
    visit_line(&mut app, 1);
    app.press("M-1").unwrap();
    assert_eq!(app.current_prefix_arg(), Some(PrefixArg::Num(1)));
    app.press("C-g").unwrap();
    assert_eq!(app.current_prefix_arg(), None);
}

#[test]
fn stage_intent_to_add_and_all() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "new.txt", "new\n");
    let mut app = App::new(g.clone());
    // magit-stage on untracked files: their content is not staged
    goto(&mut app, "Untracked files (1)");
    press(&mut app, &["C-u", "s"]);
    assert!(tracked(&g).contains(&"new.txt".to_string()));
    let unstaged = g.git_lines(&["diff", "--name-only", "-z"]);
    assert!(unstaged.contains(&"new.txt".to_string()), "{unstaged:?}");
    // magit-stage-modified: with --all also new files
    let (_td2, g2, _) = repo_with_commit();
    write(&g2, "file.txt", "changed\n");
    write(&g2, "other.txt", "other\n");
    let mut app = App::new(g2.clone());
    press(&mut app, &["C-u", "S"]);
    assert_eq!(staged(&g2), ["file.txt", "other.txt"]);
}

#[test]
fn stage_ignored_files() {
    let (_td, g, _) = repo_with_commit();
    write(&g, ".gitignore", "ign.txt\n");
    write(&g, "ign.txt", "ignored\n");
    let mut app = App::new(g.clone());
    app.script_answers(&["ign.txt"]);
    press(&mut app, &["C-c", "M-g", "C-u", "s"]);
    assert_eq!(last_prompt(&app), "Stage ignored file,s");
    assert_eq!(staged(&g), ["ign.txt"]);
    // without the argument ignored files are not offered
    app.script_answers(&["ign.txt"]);
    press(&mut app, &["C-c", "M-g", "s"]);
    assert_eq!(last_prompt(&app), "Stage file,s");
    assert_eq!(app.message(), "no match");
}

/// file.txt: "hello" in the initial commit, "world" added by "two".
fn two_commits() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    commit_file(&g, "file.txt", "hello\nworld\n", "two");
    (td, g)
}

fn show_commit(app: &mut App, rev: &str) {
    app.script_answers(&[rev]);
    press(app, &["d", "c"]);
    assert!(matches!(app.mode(), BufferMode::Revision { .. }), "{:?}", app.mode());
    goto(app, "modified   file.txt");
}

#[test]
fn reverse_and_apply_3way() {
    let (_td, g) = two_commits();
    let mut app = App::new(g.clone());
    // magit-reverse: only in the working tree ...
    show_commit(&mut app, "HEAD");
    app.script_answers(&["y"]);
    app.press("v").unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert!(staged(&g).is_empty());
    git(&g, &["checkout", "--", "file.txt"]);
    // ... with a prefix argument by a 3-way merge, also in the index
    app.script_answers(&["y"]);
    press(&mut app, &["C-u", "v"]);
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert_eq!(staged(&g), ["file.txt"]);
    // magit-apply likewise
    git(&g, &["commit", "-q", "-m", "three"]);
    app.press("q").unwrap();
    show_commit(&mut app, "HEAD~1");
    app.press("a").unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\nworld\n");
    assert!(staged(&g).is_empty());
    git(&g, &["checkout", "--", "file.txt"]);
    press(&mut app, &["C-u", "a"]);
    assert_eq!(read(&g, "file.txt"), "hello\nworld\n");
    assert_eq!(staged(&g), ["file.txt"]);
}

#[test]
fn diff_context_by_count() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g);
    let context = |app: &App| app.buffer_args().iter().find(|a| a.starts_with("-U")).cloned();
    assert_eq!(context(&app), None);
    press(&mut app, &["C-u", "+"]);
    assert_eq!(context(&app).as_deref(), Some("-U7"));
    press(&mut app, &["C-u", "2", "-"]);
    assert_eq!(context(&app).as_deref(), Some("-U5"));
    app.press("-").unwrap();
    assert_eq!(context(&app).as_deref(), Some("-U4"));
    press(&mut app, &["C-u", "9", "-"]);
    assert_eq!(context(&app).as_deref(), Some("-U0"));
    app.press("0").unwrap();
    assert_eq!(context(&app), None);
}

#[test]
fn log_move_to_nth_parent() {
    let (_td, g, _) = repo_with_commit();
    git(&g, &["checkout", "-q", "-b", "side"]);
    commit_file(&g, "s.txt", "s\n", "side");
    git(&g, &["checkout", "-q", "master"]);
    commit_file(&g, "m.txt", "m\n", "main");
    git(&g, &["merge", "-q", "--no-ff", "--no-edit", "side"]);
    let mut app = App::new(g.clone());
    press(&mut app, &["l", "l"]);
    let head = g.git_string(&["rev-parse", "--short", "HEAD"]).unwrap();
    goto_by(&mut app, &head, |t| t.contains(&head));
    press(&mut app, &["C-u", "2", "C-c", "C-n"]);
    assert!(app.point_text().ends_with("side"), "{}", app.point_text());
    goto_by(&mut app, &head, |t| t.contains(&head));
    press(&mut app, &["C-c", "C-n"]);
    assert!(app.point_text().ends_with("main"), "{}", app.point_text());
}

#[test]
fn bury_and_kill_buffers() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g);
    press(&mut app, &["d", "u"]);
    assert!(matches!(app.mode(), BufferMode::Diff(_)));
    // one prefix argument kills the buffer shown
    press(&mut app, &["C-u", "q"]);
    assert_eq!(app.mode(), &BufferMode::Status);
    assert!(!app.quitting());
    // two kill all Magit buffers but the status buffer
    press(&mut app, &["d", "u", "l", "l", "C-u", "C-u", "q"]);
    assert!(!app.quitting());
    assert_eq!(app.mode(), &BufferMode::Status);
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status, "nothing was left below");
    assert!(!app.quitting());
}

#[test]
fn branch_delete_rename_and_reset() {
    let (_td, g, _) = repo_with_commit();
    git(&g, &["checkout", "-q", "-b", "side"]);
    commit_file(&g, "s.txt", "s\n", "side");
    git(&g, &["checkout", "-q", "master"]);
    let branches = |g: &Git| g.branches();
    let mut app = App::new(g.clone());
    // an unmerged branch is only deleted when confirmed
    app.script_answers(&["side", "n"]);
    press(&mut app, &["b", "k"]);
    assert_eq!(app.prompts(), ["Delete branch", "Delete unmerged branch side? (y or n) "]);
    assert!(branches(&g).contains(&"side".to_string()));
    // or forced
    app.script_answers(&["side"]);
    press(&mut app, &["b", "C-u", "k"]);
    assert_eq!(last_prompt(&app), "Force delete branch");
    assert!(!branches(&g).contains(&"side".to_string()));
    // a merged one without confirmation
    git(&g, &["branch", "merged"]);
    app.script_answers(&["merged"]);
    press(&mut app, &["b", "k"]);
    assert_eq!(last_prompt(&app), "Delete branch");
    assert!(!branches(&g).contains(&"merged".to_string()));
    // magit-branch-rename: with a prefix argument even over a branch
    git(&g, &["branch", "a"]);
    git(&g, &["branch", "b", "HEAD"]);
    app.script_answers(&["a", "b"]);
    app.press("b").unwrap();
    assert!(app.press("m").is_err(), "b exists");
    assert!(branches(&g).contains(&"a".to_string()));
    app.script_answers(&["a", "b"]);
    press(&mut app, &["b", "C-u", "m"]);
    assert!(!branches(&g).contains(&"a".to_string()));
    // magit-branch-reset: with a prefix argument set the upstream
    git(&g, &["branch", "c"]);
    app.script_answers(&["c", "b"]);
    press(&mut app, &["b", "C-u", "x"]);
    assert_eq!(g.git_string(&["rev-parse", "--abbrev-ref", "c@{upstream}"]).unwrap(), "b");
}

#[test]
fn untrack_and_delete_force() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "staged\n");
    git(&g, &["add", "file.txt"]);
    write(&g, "file.txt", "unstaged\n");
    let mut app = App::new(g.clone());
    // staged and unstaged changes: only forced
    app.script_answers(&["file.txt"]);
    press(&mut app, &["C-c", "M-g", "x"]);
    assert!(tracked(&g).contains(&"file.txt".to_string()), "{}", app.message());
    app.script_answers(&["file.txt"]);
    press(&mut app, &["C-c", "M-g", "C-u", "x"]);
    assert!(!tracked(&g).contains(&"file.txt".to_string()), "{}", app.message());
    // a file with uncommitted changes is only deleted when forced
    commit_file(&g, "d.txt", "d\n", "d");
    write(&g, "d.txt", "changed\n");
    app.script_answers(&["d.txt"]);
    press(&mut app, &["C-c", "M-g", "k"]);
    assert!(exists(&g, "d.txt"), "{}", app.message());
    app.script_answers(&["d.txt"]);
    press(&mut app, &["C-c", "M-g", "C-u", "k"]);
    assert!(!exists(&g, "d.txt"), "{}", app.message());
}

/// f.txt: "one" a b c, "two" b -> B; the worktree changes c to C.
fn file_repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    commit_file(&g, "f.txt", "a\nb\nc\n", "one");
    commit_file(&g, "f.txt", "a\nB\nc\n", "two");
    write(&g, "f.txt", "a\nB\nC\n");
    (td, g)
}

fn visit_line(app: &mut App, line: usize) {
    goto(app, "modified   f.txt");
    app.press("RET").unwrap();
    assert_eq!(app.header_line(), "f.txt");
    for _ in 1..line {
        app.press("C-n").unwrap();
    }
    assert_eq!(app.file_line(), Some(line));
}

#[test]
fn log_buffer_file_follow() {
    let (_td, g) = file_repo();
    let mut app = App::new(g);
    visit_line(&mut app, 1);
    press(&mut app, &["C-c", "M-g", "l"]);
    assert!(matches!(app.mode(), BufferMode::Log { .. }));
    assert!(!app.buffer_args().contains(&"--follow".to_string()));
    app.press("q").unwrap();
    press(&mut app, &["C-c", "M-g", "C-u", "l"]);
    assert!(matches!(app.mode(), BufferMode::Log { .. }));
    assert!(app.buffer_args().contains(&"--follow".to_string()), "{:?}", app.buffer_args());
}

#[test]
fn edit_line_commit_removal() {
    let (_td, g) = file_repo();
    let mut app = App::new(g.clone());
    visit_line(&mut app, 2);
    git(&g, &["stash", "-q"]);
    // the line B was added by "two"; the commit of the removed b is "one"
    press(&mut app, &["C-c", "M-g", "C-u", "e"]);
    assert!(g.gitdir.join("rebase-merge").exists(), "{}", app.message());
    assert_eq!(g.git_string(&["log", "-1", "--format=%s"]).unwrap(), "one");
}

#[test]
fn commit_extend_and_reword_keep_date() {
    let (_td, g, _) = repo_with_commit();
    let date = |g: &Git| g.git_string(&["log", "-1", "--format=%cI"]).unwrap();
    g.git_interactive_env(&[("GIT_COMMITTER_DATE", "2001-02-03T04:05:06+00:00")], &["commit", "-q", "--amend", "--no-edit"])
        .unwrap();
    let old = date(&g);
    assert!(old.starts_with("2001-02-03"), "{old}");
    let mut app = App::new(g.clone());
    write(&g, "file.txt", "more\n");
    git(&g, &["add", "file.txt"]);
    press(&mut app, &["c", "C-u", "e"]);
    assert_eq!(g.git_string(&["show", "HEAD:file.txt"]).unwrap(), "more");
    assert_eq!(date(&g), old, "{}", app.message());
    // reword edits the message in the built-in buffer: confirm it
    press(&mut app, &["c", "C-u", "w", "C-c", "C-c"]);
    assert_eq!(date(&g), old, "{}", app.message());
    press(&mut app, &["c", "w", "C-c", "C-c"]);
    assert_ne!(date(&g), old);
}

#[test]
fn stash_untracked_by_prefix() {
    let (_td, g, _) = repo_with_commit();
    write(&g, ".gitignore", "ign.txt\n");
    write(&g, "ign.txt", "ignored\n");
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g.clone());
    // one prefix argument: --include-untracked
    app.script_answers(&["one"]);
    press(&mut app, &["z", "C-u", "z"]);
    assert!(!exists(&g, ".gitignore"), "{}", app.message());
    assert!(exists(&g, "ign.txt"));
    // two: --all
    write(&g, "file.txt", "changed\n");
    app.script_answers(&["two"]);
    press(&mut app, &["z", "C-u", "C-u", "z"]);
    assert!(!exists(&g, "ign.txt"), "{}", app.message());
}

#[test]
fn read_instead_of_thing_at_point() {
    let (_td, g) = two_commits();
    let mut app = App::new(g.clone());
    press(&mut app, &["l", "l"]);
    let head = g.git_string(&["rev-parse", "--short", "HEAD"]).unwrap();
    goto_by(&mut app, &head, |t| t.contains(&head));
    // magit-show-commit: the commit at point, else one read
    let before = app.prompts().len();
    press(&mut app, &["d", "c"]);
    assert_eq!(app.prompts().len(), before);
    assert!(matches!(app.mode(), BufferMode::Revision { rev, .. } if rev.starts_with(&head)), "{:?}", app.mode());
    app.press("q").unwrap();
    // RET (magit-show-commit) likewise
    goto_by(&mut app, &head, |t| t.contains(&head));
    app.script_answers(&["HEAD~1"]);
    press(&mut app, &["C-u", "RET"]);
    assert_eq!(last_prompt(&app), "Show commit");
    assert!(matches!(app.mode(), BufferMode::Revision { rev, .. } if rev == "HEAD~1"), "{:?}", app.mode());
    app.press("q").unwrap();
    app.script_answers(&["HEAD~1"]);
    press(&mut app, &["d", "C-u", "c"]);
    assert_eq!(last_prompt(&app), "Show commit");
    assert!(matches!(app.mode(), BufferMode::Revision { rev, .. } if rev == "HEAD~1"), "{:?}", app.mode());
    // magit-diff-working-tree / -staged: against a commit read
    app.script_answers(&["HEAD~1"]);
    press(&mut app, &["d", "C-u", "w"]);
    assert_eq!(last_prompt(&app), "Diff working tree and commit");
    assert!(matches!(app.mode(), BufferMode::Diff(s) if s.range.as_deref() == Some("HEAD~1")), "{:?}", app.mode());
    app.script_answers(&["HEAD~1"]);
    press(&mut app, &["d", "C-u", "s"]);
    assert_eq!(last_prompt(&app), "Diff index and commit");
    assert!(
        matches!(app.mode(), BufferMode::Diff(s) if s.range.as_deref() == Some("HEAD~1") && s.typ == "staged"),
        "{:?}",
        app.mode()
    );
}

#[test]
fn show_stash_read() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "one\n");
    git(&g, &["stash", "-q"]);
    write(&g, "file.txt", "two\n");
    git(&g, &["stash", "-q"]);
    let mut app = App::new(g);
    goto(&mut app, "Stashes (2)");
    app.press("TAB").unwrap();
    goto_by(&mut app, "stash@{0}", |t| t.starts_with("stash@{0}"));
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Stash { stash } if stash == "stash@{0}"), "{:?}", app.mode());
    app.press("q").unwrap();
    goto_by(&mut app, "stash@{0}", |t| t.starts_with("stash@{0}"));
    app.script_answers(&["stash@{1}"]);
    press(&mut app, &["C-u", "RET"]);
    assert_eq!(last_prompt(&app), "Show stash");
    assert!(matches!(app.mode(), BufferMode::Stash { stash } if stash == "stash@{1}"), "{:?}", app.mode());
}
