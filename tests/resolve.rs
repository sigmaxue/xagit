//! The built-in conflict resolution view: `e` (ediff dwim) with unmerged
//! conflicts opens a view where a/b/d choose A(ours)/B(theirs)/both, u
//! undecides, n/p move, and C-c C-c writes the file back and stages it.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};
use xagit_lib::ui::resolve::Choice;
use xagit_lib::ui::theme::{Face, Style, Theme};

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

/// A repository with a real merge conflict: master and topic both changed
/// the same lines of f.txt, then `git rebase master` on topic.
fn conflict_repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    git(&g, &["config", "merge.conflictStyle", "merge"]);
    write(&g, "f.txt", "top\nbase line\nmid\nbottom\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "base"]);
    git(&g, &["checkout", "-q", "-b", "topic"]);
    write(&g, "f.txt", "top\nTOPIC line\nmid\nbottom\n");
    git(&g, &["commit", "-q", "-am", "topic"]);
    git(&g, &["checkout", "-q", "master"]);
    write(&g, "f.txt", "top\nMASTER line\nmid\nbottom\n");
    git(&g, &["commit", "-q", "-am", "master"]);
    git(&g, &["checkout", "-q", "topic"]);
    // rebase topic onto master conflicts in f.txt
    let _ = g.git_string(&["rebase", "master"]);
    (td, g)
}

fn open(app: &mut App) {
    app.press("e").unwrap();
    assert_eq!(app.mode(), &BufferMode::Resolve, "{}", app.message());
}

#[test]
fn e_opens_only_with_conflicts() {
    let (_td, g) = conflict_repo();
    assert!(!g.unmerged_files().is_empty(), "expected a conflict");
    let mut app = App::new(g.clone());
    open(&mut app);
    assert_eq!(app.resolve_choices().unwrap().len(), 1);
    // without conflicts `e` does not open the view
    git(&g, &["rebase", "--abort"]);
    let mut clean = App::new(g);
    clean.press("e").unwrap();
    assert_ne!(clean.mode(), &BufferMode::Resolve);
}

#[test]
fn choose_a_b_both_and_save() {
    let (_td, g) = conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    // conflict markers are still in the worktree file until C-c C-c
    assert!(read(&g, "f.txt").contains("<<<<<<<"));

    // rebase: the target (master) is the RIGHT column (B); the left (A)
    // is the branch being rebased (topic)
    // a: keep A = topic
    press(&mut app, &["a"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::A]);
    let out = app.resolve_result().unwrap();
    assert!(out.contains("TOPIC line") && !out.contains("MASTER line"), "a: {out:?}");

    // r (restore) then b: keep B = the target master
    press(&mut app, &["r"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::Undecided]);
    press(&mut app, &["b"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::B]);
    let out = app.resolve_result().unwrap();
    assert!(out.contains("MASTER line") && !out.contains("TOPIC line"), "b: {out:?}");

    // + (ediff-combine-diffs): keep both
    press(&mut app, &["+"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::Both]);
    let out = app.resolve_result().unwrap();
    assert!(out.contains("MASTER line") && out.contains("TOPIC line"), "+: {out:?}");

    // C-c C-c writes back and stages the file
    press(&mut app, &["C-c", "C-c"]);
    assert_ne!(app.mode(), &BufferMode::Resolve);
    let body = read(&g, "f.txt");
    assert!(!body.contains("<<<<<<<") && !body.contains("=======") && !body.contains(">>>>>>>"), "markers left: {body:?}");
    assert!(body.contains("MASTER line") && body.contains("TOPIC line"), "both: {body:?}");
    assert!(g.unmerged_files().is_empty(), "file should be staged/resolved: {}", app.message());
}

#[test]
fn multiple_conflicts_and_movement() {
    let (_td, g) = two_conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    assert_eq!(app.resolve_choices().unwrap().len(), 2);
    assert_eq!(app.resolve_cursor(), Some(0));
    press(&mut app, &["a"]);
    // like ediff-copy-A-to-C, point stays on the difference; n moves on
    assert_eq!(app.resolve_cursor(), Some(0));
    press(&mut app, &["n", "b"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::A, Choice::B]);
    // n/p move between conflicts
    press(&mut app, &["p"]);
    assert_eq!(app.resolve_cursor(), Some(0));
    press(&mut app, &["n"]);
    assert_eq!(app.resolve_cursor(), Some(1));
    press(&mut app, &["C-c", "C-c"]);
    let body = read(&g, "f.txt");
    assert!(!body.contains("<<<<<<<"), "{body:?}");
}

/// Two distant conflicting lines in the same file.
fn two_conflict_repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    write(&g, "f.txt", "1\n2\n3\n4\n5\n6\n7\n8\n9\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "base"]);
    git(&g, &["checkout", "-q", "-b", "topic"]);
    write(&g, "f.txt", "TOPIC-1\n2\n3\n4\n5\n6\n7\n8\nTOPIC-9\n");
    git(&g, &["commit", "-q", "-am", "topic"]);
    git(&g, &["checkout", "-q", "master"]);
    write(&g, "f.txt", "MASTER-1\n2\n3\n4\n5\n6\n7\n8\nMASTER-9\n");
    git(&g, &["commit", "-q", "-am", "master"]);
    git(&g, &["checkout", "-q", "topic"]);
    let _ = g.git_string(&["rebase", "master"]);
    (td, g)
}

#[test]
fn two_columns_with_diff_highlight() {
    let (_td, g) = conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    // render all rows at a wide width
    let view = app.resolve_view(40, 80);
    let theme = Theme::default();
    // the target (master) is on the RIGHT (B); topic is on the LEFT (A)
    let pair = view.iter().find(|(text, _)| text.contains("MASTER line") && text.contains("│"));
    assert!(pair.is_some(), "no side-by-side row: {:?}", view.iter().map(|(t, _)| t).collect::<Vec<_>>());
    assert!(view.iter().any(|(t, _)| t.contains("A: topic") && t.contains("B: master")), "{:?}", view.iter().map(|(t, _)| t).collect::<Vec<_>>());
    // A's line (topic) uses Added, B's (master) uses Removed
    let (_, spans) = pair.unwrap();
    let added = theme.style(Face::Added);
    let removed = theme.style(Face::Removed);
    let has_added = spans.iter().any(|(s, st)| s.contains("TOPIC line") && *st == added);
    let has_removed = spans.iter().any(|(s, st)| s.contains("MASTER line") && *st == removed);
    assert!(has_added, "A line not highlighted: {spans:?}");
    assert!(has_removed, "B line not highlighted: {spans:?}");
    // common (unchanged) lines, when present, carry the Context style
    let _ = Style::default();
}

#[test]
fn shows_undifferenced_context_lines() {
    let (_td, g) = conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    // render the whole file (body large enough for all display lines)
    let view = app.resolve_view(100, 80);
    let theme = Theme::default();
    let context_style = theme.style(Face::Context);
    // "top", "mid", "bottom" are unchanged lines outside the conflict:
    // they must appear identically in BOTH columns, without diff highlight
    for text in ["top", "mid", "bottom"] {
        let row = view.iter().find(|(t, spans)| {
            t.contains("│")
                && spans.len() >= 3
                && spans.iter().any(|(s, _)| s.trim() == text)
                && spans.iter().filter(|(s, _)| s.trim() == text).count() >= 2
        });
        assert!(row.is_some(), "context line {text:?} not shown in both columns: {:?}", view.iter().map(|(t, _)| t).collect::<Vec<_>>());
        // both column spans carry the plain Context style (no Added/Removed)
        let (_, spans) = row.unwrap();
        let col_spans: Vec<_> = spans.iter().filter(|(s, _)| s.trim() == text || s.starts_with(text)).collect();
        assert!(col_spans.iter().all(|(_, st)| *st == context_style), "context {text:?} highlighted: {col_spans:?}");
    }
    // the conflict's two different lines are still highlighted
    assert!(view.iter().any(|(t, _)| t.contains("MASTER line") && t.contains("│")));
    assert!(view.iter().any(|(t, _)| t.contains("TOPIC line") && t.contains("│")));
}

#[test]
fn free_page_scrolling() {
    let (_td, g) = two_conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    assert_eq!(app.resolve_top(), Some(0));
    // arrows scroll a single line
    app.press("down").unwrap();
    assert_eq!(app.resolve_top(), Some(1));
    app.press("down").unwrap();
    assert_eq!(app.resolve_top(), Some(2));
    app.press("up").unwrap();
    assert_eq!(app.resolve_top(), Some(1));
    // SPC pages down, DEL pages back (a page is the A/B window's height,
    // so it returns to where it was unless SPC hit the bottom)
    app.press("SPC").unwrap();
    let after_page = app.resolve_top().unwrap();
    assert!(after_page > 1, "SPC did not scroll");
    app.press("DEL").unwrap();
    assert!(app.resolve_top().unwrap() <= 1, "{:?}", app.resolve_top());
    app.press("DEL").unwrap();
    assert_eq!(app.resolve_top(), Some(0));
    // n jumps to the next conflict and scrolls its header to the top
    press(&mut app, &["n"]);
    assert_eq!(app.resolve_cursor(), Some(1));
    assert!(app.resolve_top().unwrap() > 0);
}

#[test]
fn cancel_leaves_file_untouched() {
    let (_td, g) = conflict_repo();
    let before = read(&g, "f.txt");
    let mut app = App::new(g.clone());
    open(&mut app);
    press(&mut app, &["+"]);
    // declining the confirm keeps editing
    app.script_answers(&["n"]);
    press(&mut app, &["C-c", "C-k"]);
    assert_eq!(app.mode(), &BufferMode::Resolve);
    // confirming cancels without writing
    app.script_answers(&["y"]);
    press(&mut app, &["C-c", "C-k"]);
    assert_ne!(app.mode(), &BufferMode::Resolve);
    assert_eq!(read(&g, "f.txt"), before);
    assert!(!g.unmerged_files().is_empty());
}

#[test]
fn undecided_keeps_markers() {
    let (_td, g) = conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    press(&mut app, &["a", "r"]); // decide then restore
    assert_eq!(app.resolve_choices().unwrap(), [Choice::Undecided]);
    press(&mut app, &["C-c", "C-c"]);
    // markers are retained, file stays unresolved
    assert!(read(&g, "f.txt").contains("<<<<<<<"));
    assert!(!g.unmerged_files().is_empty());
    assert!(app.message().contains("unresolved"), "{}", app.message());
}

/// q: "Conflict resolution finished; save FILE?" (smerge-ediff quit hook).
#[test]
fn q_asks_to_save() {
    let (_td, g) = conflict_repo();
    let before = read(&g, "f.txt");
    // no: the file stays as it was
    let mut app = App::new(g.clone());
    open(&mut app);
    press(&mut app, &["a"]);
    app.script_answers(&["no"]);
    press(&mut app, &["q"]);
    assert!(app.prompts().last().unwrap().starts_with("Conflict resolution finished; save f.txt?"), "{:?}", app.prompts());
    assert_ne!(app.mode(), &BufferMode::Resolve);
    assert_eq!(read(&g, "f.txt"), before);
    assert!(!g.unmerged_files().is_empty());
    // yes: saved, and staged because no conflict markers remain
    let mut app = App::new(g.clone());
    open(&mut app);
    press(&mut app, &["b"]);
    app.script_answers(&["yes"]);
    press(&mut app, &["q"]);
    let body = read(&g, "f.txt");
    assert!(body.contains("MASTER line") && !body.contains("<<<<<<< "), "{body:?}");
    assert!(g.unmerged_files().is_empty(), "{}", app.message());
}

/// j jumps to difference N (numeric prefix), ! recomputes from disk.
#[test]
fn jump_and_recompute() {
    let (_td, g) = two_conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    press(&mut app, &["C-u", "2", "j"]);
    assert_eq!(app.resolve_cursor(), Some(1), "{}", app.message());
    press(&mut app, &["j"]);
    assert_eq!(app.resolve_cursor(), Some(0));
    // the file is changed behind the view: ! picks up the new conflicts
    press(&mut app, &["a"]);
    let text = read(&g, "f.txt");
    let first_end = text.find(">>>>>>>").unwrap();
    let line_end = first_end + text[first_end..].find('\n').unwrap() + 1;
    let start = text.find("<<<<<<<").unwrap();
    write(&g, "f.txt", &format!("{}MASTER-1\n{}", &text[..start], &text[line_end..]));
    press(&mut app, &["!"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::Undecided]);
    assert!(app.message().contains("1 difference"), "{}", app.message());
}

/// r r refuses with unresolved conflicts (unstaged changes), like
/// magit-rebase-continue.
#[test]
fn rebase_continue_refuses_with_conflicts() {
    let (_td, g) = conflict_repo();
    let mut app = App::new(g.clone());
    press(&mut app, &["r", "r"]);
    assert_eq!(app.message(), "Cannot continue rebase with unstaged changes");
    assert!(g.rebase_in_progress());
    // resolved and staged: r r continues
    open(&mut app);
    press(&mut app, &["b", "C-c", "C-c"]);
    git(&g, &["config", "core.editor", "true"]);
    press(&mut app, &["r", "r"]);
    assert!(!g.rebase_in_progress(), "{}", app.message());
    // no rebase: No rebase in progress
    let mut app = App::new(g.clone());
    press(&mut app, &["r", "r"]);
    assert_ne!(app.message(), "rebase continued");
}

/// The sides are named after the branches, and a third window (C) shows
/// the merge result as it will be saved.
#[test]
fn branch_names_and_result_window() {
    let (_td, g) = conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    // rebase topic onto master: the target master is on the RIGHT (B),
    // the branch being rebased (topic) is on the LEFT (A)
    let (a, b) = app.resolve_side_names_shown().unwrap();
    assert!(a.starts_with("topic (") && a.contains("topic"), "{a}");
    assert_eq!(b, "master");
    let texts = |app: &App| app.resolve_view(30, 100).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    let v = texts(&app);
    let c = v.iter().position(|t| t.contains("C: merge result")).expect("C window");
    // undecided: C still shows the conflict markers
    assert!(v[c..].iter().any(|t| t.contains("<<<<<<<")), "{v:?}");
    // a: C shows A's (topic) line in place of the conflict, with context
    press(&mut app, &["a"]);
    let v = texts(&app);
    let c = v.iter().position(|t| t.contains("C: merge result")).unwrap();
    let below = &v[c + 1..];
    assert!(below.iter().any(|t| t.contains("TOPIC line")), "{v:?}");
    assert!(!below.iter().any(|t| t.contains("MASTER line") || t.contains("<<<<<<<")), "{v:?}");
    assert!(below.iter().any(|t| t.trim() == "top") && below.iter().any(|t| t.trim() == "bottom"), "{v:?}");
    // the header and message name the chosen branch
    assert!(v.iter().any(|t| t.contains("Conflict 1") && t.contains("A: topic")), "{v:?}");
    assert!(app.message().contains("A: topic"), "{}", app.message());
    // +: both, in order
    press(&mut app, &["+"]);
    let v = texts(&app);
    let c = v.iter().position(|t| t.contains("C: merge result")).unwrap();
    let below: Vec<&String> = v[c + 1..].iter().collect();
    // both: A (topic, left) comes before B (master, right)
    let tp = below.iter().position(|t| t.contains("TOPIC line")).expect("A in C");
    let m = below.iter().position(|t| t.contains("MASTER line")).expect("B in C");
    assert!(tp < m, "{below:?}");
}

/// A merge names its sides after the current and the merged branch.
#[test]
fn merge_side_names() {
    let (_td, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    write(&g, "f.txt", "x\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "base"]);
    git(&g, &["checkout", "-q", "-b", "feature"]);
    write(&g, "f.txt", "feature\n");
    git(&g, &["commit", "-q", "-am", "feature"]);
    git(&g, &["checkout", "-q", "master"]);
    write(&g, "f.txt", "main\n");
    git(&g, &["commit", "-q", "-am", "main"]);
    let _ = g.git_string(&["merge", "feature"]);
    let mut app = App::new(g.clone());
    open(&mut app);
    assert_eq!(app.resolve_side_names_shown().unwrap(), ("master".to_string(), "feature".to_string()));
}

/// A rebase conflict whose block has two side-by-side rows on both sides.
fn multiline_conflict_repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    git(&g, &["config", "merge.conflictStyle", "merge"]);
    write(&g, "f.txt", "top\none\ntwo\nbottom\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "base"]);
    git(&g, &["checkout", "-q", "-b", "topic"]);
    write(&g, "f.txt", "top\nTOPIC-1\nTOPIC-2\nbottom\n");
    git(&g, &["commit", "-q", "-am", "topic"]);
    git(&g, &["checkout", "-q", "master"]);
    write(&g, "f.txt", "top\nMASTER-1\nMASTER-2\nbottom\n");
    git(&g, &["commit", "-q", "-am", "master"]);
    git(&g, &["checkout", "-q", "topic"]);
    let _ = g.git_string(&["rebase", "master"]);
    (td, g)
}

#[test]
fn refine_chooses_lines_individually() {
    let (_td, g) = multiline_conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    assert_eq!(app.resolve_refine(), Some(false));
    assert_eq!(app.resolve_line_count(), Some(2));

    // * enters line-wise mode; a decides only the first row
    press(&mut app, &["*"]);
    assert_eq!(app.resolve_refine(), Some(true));
    assert_eq!(app.resolve_row(), Some(0));
    press(&mut app, &["a"]);
    assert_eq!(app.resolve_line_choices().unwrap(), [Choice::A, Choice::Undecided]);
    // a partly decided conflict keeps the markers (and both sides)
    assert!(app.resolve_result().unwrap().contains("<<<<<<<"));
    let view: Vec<String> = app.resolve_view(40, 80).into_iter().map(|(t, _)| t).collect();
    assert!(view.iter().any(|t| t.contains("TOPIC-1") && t.contains(" A ")), "{view:?}");
    assert!(view.iter().any(|t| t.contains("TOPIC-2") && t.contains(" ? ")), "{view:?}");

    // n moves to the next LINE, not the next conflict
    press(&mut app, &["n"]);
    assert_eq!(app.resolve_cursor(), Some(0));
    assert_eq!(app.resolve_row(), Some(1));
    press(&mut app, &["b"]);
    assert_eq!(app.resolve_line_choices().unwrap(), [Choice::A, Choice::B]);
    let out = app.resolve_result().unwrap();
    assert!(out.contains("TOPIC-1") && out.contains("MASTER-2"), "{out:?}");
    assert!(!out.contains("MASTER-1") && !out.contains("TOPIC-2") && !out.contains("<<<<<<<"), "{out:?}");

    // d combines both sides of the current line
    press(&mut app, &["d"]);
    assert_eq!(app.resolve_line_choices().unwrap(), [Choice::A, Choice::Both]);
    let out = app.resolve_result().unwrap();
    assert!(out.contains("TOPIC-2") && out.contains("MASTER-2"), "{out:?}");

    // r restores the current line only
    press(&mut app, &["r"]);
    assert_eq!(app.resolve_line_choices().unwrap(), [Choice::A, Choice::Undecided]);
    assert!(app.resolve_result().unwrap().contains("<<<<<<<"));

    // * back to whole-conflict; a decides the whole block again
    press(&mut app, &["*"]);
    assert_eq!(app.resolve_refine(), Some(false));
    press(&mut app, &["a"]);
    assert_eq!(app.resolve_choices().unwrap(), [Choice::A]);
    let out = app.resolve_result().unwrap();
    assert!(out.contains("TOPIC-1") && out.contains("TOPIC-2") && !out.contains("<<<<<<<"), "{out:?}");
}

#[test]
fn refine_moves_across_conflicts_and_saves() {
    let (_td, g) = two_conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    assert_eq!(app.resolve_line_count(), Some(1));
    press(&mut app, &["*", "a"]);
    press(&mut app, &["n"]);
    assert_eq!(app.resolve_cursor(), Some(1));
    assert_eq!(app.resolve_row(), Some(0));
    press(&mut app, &["b"]);
    press(&mut app, &["C-c", "C-c"]);
    let body = read(&g, "f.txt");
    assert!(!body.contains("<<<<<<<"), "{body:?}");
    assert!(body.contains("TOPIC-1") && body.contains("MASTER-9"), "{body:?}");
    assert!(!body.contains("MASTER-1") && !body.contains("TOPIC-9"), "{body:?}");
    assert!(g.unmerged_files().is_empty(), "{}", app.message());
}

#[test]
fn partly_decided_conflict_keeps_markers() {
    let (_td, g) = multiline_conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    press(&mut app, &["*", "a"]);
    press(&mut app, &["C-c", "C-c"]);
    let body = read(&g, "f.txt");
    assert!(body.contains("<<<<<<<"), "{body:?}");
    assert!(body.contains("TOPIC-1") && body.contains("MASTER-1"), "both sides survive: {body:?}");
    assert!(app.message().contains("unresolved"), "{}", app.message());
    assert!(!g.unmerged_files().is_empty());
}

#[test]
fn finish_hints_the_operation_specific_continue_key() {
    // a rebase conflict resumes with `r r`
    let (_td1, g) = conflict_repo();
    let mut app = App::new(g.clone());
    open(&mut app);
    press(&mut app, &["a", "C-c", "C-c"]);
    assert!(app.message().contains("r r to continue"), "{}", app.message());

    // a merge conflict has no `r r`: commit or abort the merge
    let (_td2, g) = scratch_repo();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    write(&g, "f.txt", "x\n");
    git(&g, &["add", "."]);
    git(&g, &["commit", "-q", "-m", "base"]);
    git(&g, &["checkout", "-q", "-b", "feature"]);
    write(&g, "f.txt", "feature\n");
    git(&g, &["commit", "-q", "-am", "feature"]);
    git(&g, &["checkout", "-q", "master"]);
    write(&g, "f.txt", "main\n");
    git(&g, &["commit", "-q", "-am", "main"]);
    let _ = g.git_string(&["merge", "feature"]);
    let mut app = App::new(g.clone());
    open(&mut app);
    press(&mut app, &["a", "C-c", "C-c"]);
    assert!(app.message().contains("m m to commit"), "{}", app.message());
    assert!(!app.message().contains("r r"), "{}", app.message());
}
