//! Status buffer: sections (movement, visibility), diff hunks and
//! hunk / region staging like magit-section.el and magit-apply.el.

mod common;

use common::*;
use ngit_lib::git::diff::{self, Hunk};
use ngit_lib::prelude::*;
use ngit_lib::ui::app::{App, BufferMode};

fn numbered(n: usize) -> String {
    (1..=n).map(|i| format!("l{i}\n")).collect()
}

/// file.txt with lines l1..l20 committed, then lines 2 and 19 changed:
/// two hunks (@@ -1,5 +1,5 @@ and @@ -16,5 +16,5 @@) plus an untracked file.
fn two_hunk_repo() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    write(&g, "file.txt", &numbered(20));
    g.git_interactive(&["commit", "-q", "-am", "twenty"]).unwrap();
    write(&g, "file.txt", &numbered(20).replace("l2\n", "L2\n").replace("l19\n", "L19\n"));
    write(&g, "new.txt", "new\n");
    (td, g)
}

/// Move point to the first visible line whose text starts with PREFIX.
fn goto(app: &mut App, prefix: &str) {
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-p").unwrap();
    }
    for _ in 0..n {
        if app.point_text().starts_with(prefix) {
            return;
        }
        app.press("C-n").unwrap();
    }
    let lines: Vec<&str> = app.visible_lines().iter().map(|l| l.text.as_str()).collect();
    panic!("no visible line starting with {prefix:?} in {lines:#?}");
}

fn visible(app: &App, prefix: &str) -> bool {
    app.visible_lines().iter().any(|l| l.text.starts_with(prefix))
}

fn cached_diff(g: &Git) -> String {
    g.git_string(&["diff", "--cached"]).unwrap()
}

fn worktree_diff(g: &Git) -> String {
    g.git_string(&["diff"]).unwrap()
}

#[test]
fn parse_diff_sections() {
    let text = "diff --git a.txt a.txt\nindex 1..2 100644\n--- a.txt\n+++ a.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n@@ -10,2 +10,3 @@ fn\n x\n+y\n z\ndiff --git old new\nsimilarity index 100%\nrename from old\nrename to new\n";
    let files = diff::parse(text);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "a.txt");
    assert_eq!(files[0].hunks.len(), 2);
    assert_eq!(files[0].hunks[1].header, "@@ -10,2 +10,3 @@ fn");
    assert_eq!(files[0].hunks[0].lines, vec![" a", "-b", "+B", " c"]);
    assert_eq!(files[1].path, "new");
    assert_eq!(files[1].orig.as_deref(), Some("old"));
    assert_eq!(files[1].status, "renamed");
}

#[test]
fn region_patch_keeps_only_selected_changes() {
    let h = Hunk {
        header: "@@ -1,4 +1,4 @@".into(),
        lines: vec![" a".into(), "-b".into(), "+B".into(), "-c".into(), "+C".into(), " d".into()],
    };
    // staging "-b +B": "-c" becomes context, "+C" is dropped
    assert_eq!(diff::region_hunk(&h, 1, 2, false), "@@ -1,4 +1,4 @@\n a\n-b\n+B\n c\n d\n");
    // unstaging (reverse) "-c +C": "+B" becomes context, "-b" is dropped
    assert_eq!(diff::region_hunk(&h, 3, 4, true), "@@ -1,4 +1,4 @@\n a\n B\n-c\n+C\n d\n");
}

#[test]
fn adjust_new_starts_like_magit() {
    let hunks = vec!["@@ -10,6 +11,7 @@\n".to_string(), "@@ -18,6 +20,7 @@\n".to_string()];
    let adj = diff::adjust_new_starts(&hunks);
    assert!(adj[0].starts_with("@@ -10,6 +10,7 @@"));
    assert!(adj[1].starts_with("@@ -18,6 +19,7 @@"));
}

#[test]
fn section_movement_and_toggle() {
    let (_td, g) = two_hunk_repo();
    let mut app = App::new(g);
    // magit-status-initial-section: the first section after the headers
    assert_eq!(app.point_text(), "Untracked files (1)");
    // untracked is hidden by default, so n goes to the next sibling
    assert!(!visible(&app, "new.txt"));
    app.press("n").unwrap();
    assert_eq!(app.point_text(), "Unstaged changes (1)");
    app.press("n").unwrap();
    assert_eq!(app.point_text(), "modified   file.txt");
    // file sections are collapsed in the status buffer
    assert!(!visible(&app, "@@ "));
    app.press("TAB").unwrap();
    assert!(visible(&app, "@@ -1,5 +1,5 @@"));
    assert!(visible(&app, "+L2"));
    app.press("n").unwrap();
    assert!(app.point_text().starts_with("@@ -1,5 +1,5 @@"));
    app.press("M-n").unwrap();
    assert!(app.point_text().starts_with("@@ -16,5 +16,5 @@"));
    app.press("p").unwrap();
    assert!(app.point_text().starts_with("@@ -1,5 +1,5 @@"));
    // inside a hunk body, p goes to the hunk heading
    app.press("C-n").unwrap();
    app.press("C-n").unwrap();
    app.press("p").unwrap();
    assert!(app.point_text().starts_with("@@ -1,5 +1,5 @@"));
    app.press("^").unwrap();
    assert_eq!(app.point_text(), "modified   file.txt");
    app.press("p").unwrap();
    assert_eq!(app.point_text(), "Unstaged changes (1)");
    app.press("p").unwrap();
    assert_eq!(app.point_text(), "Untracked files (1)");
    app.press("TAB").unwrap();
    assert!(visible(&app, "new.txt"));
    // hiding a section from inside its body moves point to its heading
    app.press("n").unwrap();
    assert_eq!(app.point_text(), "new.txt");
    app.press("^").unwrap();
    app.press("TAB").unwrap();
    assert_eq!(app.point_text(), "Untracked files (1)");
    assert!(!visible(&app, "new.txt"));
}

#[test]
fn visibility_survives_refresh() {
    let (_td, g) = two_hunk_repo();
    let mut app = App::new(g);
    goto(&mut app, "modified   file.txt");
    app.press("TAB").unwrap();
    app.press("g").unwrap();
    assert!(visible(&app, "@@ -1,5 +1,5 @@"));
    assert_eq!(app.point_text(), "modified   file.txt");
}

#[test]
fn cycle_and_show_level() {
    let (_td, g) = two_hunk_repo();
    let mut app = App::new(g);
    goto(&mut app, "Unstaged changes");
    // children hidden and they have children: show headings (all of it)
    app.press("C-<tab>").unwrap();
    assert!(visible(&app, "@@ -1,5 +1,5 @@"));
    // nothing hidden below: hide the section
    app.press("C-<tab>").unwrap();
    assert!(!visible(&app, "modified   file.txt"));
    // hidden: show it with its children hidden
    app.press("C-<tab>").unwrap();
    assert!(visible(&app, "modified   file.txt"));
    assert!(!visible(&app, "@@ "));

    // 3 on the file shows its hunk headings (level 3), 4 also their bodies;
    // 1 hides the top-level section
    goto(&mut app, "modified   file.txt");
    app.press("3").unwrap();
    assert!(visible(&app, "@@ -16,5 +16,5 @@"));
    assert!(!visible(&app, "+L19"));
    app.press("4").unwrap();
    assert!(visible(&app, "+L19"));
    app.press("1").unwrap();
    assert_eq!(app.point_text(), "Unstaged changes (1)");
    assert!(!visible(&app, "modified   file.txt"));
    // M-2: every top-level section shown, their children hidden
    app.press("M-2").unwrap();
    assert!(visible(&app, "new.txt"));
    assert!(visible(&app, "modified   file.txt"));
    assert!(!visible(&app, "@@ "));
}

#[test]
fn stage_and_unstage_hunk() {
    let (_td, g) = two_hunk_repo();
    let mut app = App::new(g.clone());
    goto(&mut app, "modified   file.txt");
    app.press("TAB").unwrap();
    goto(&mut app, "@@ -1,5 +1,5 @@");
    app.press("s").unwrap();
    let staged = cached_diff(&g);
    assert!(staged.contains("+L2") && !staged.contains("L19"), "{staged}");
    let unstaged = worktree_diff(&g);
    assert!(unstaged.contains("+L19") && !unstaged.contains("L2\n"), "{unstaged}");
    assert!(visible(&app, "Staged changes (1)"));

    // the second hunk is staged with magit's new-start adjustment; the
    // file section stays expanded across the refresh
    goto(&mut app, "@@ -16,5 +16,5 @@");
    app.press("s").unwrap();
    assert!(cached_diff(&g).contains("+L19"));
    assert!(worktree_diff(&g).is_empty());

    // unstage the first hunk again from the staged section
    goto(&mut app, "Staged changes");
    app.press("n").unwrap();
    app.press("TAB").unwrap();
    goto(&mut app, "@@ -1,5 +1,5 @@");
    app.press("u").unwrap();
    let staged = cached_diff(&g);
    assert!(!staged.contains("+L2\n") && staged.contains("+L19"), "{staged}");
    assert!(worktree_diff(&g).contains("+L2"));
}

#[test]
fn stage_region_and_discard_region() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", &numbered(6));
    g.git_interactive(&["commit", "-q", "-am", "six"]).unwrap();
    write(&g, "file.txt", &numbered(6).replace("l2\n", "L2\n").replace("l4\n", "L4\n"));
    let mut app = App::new(g.clone());
    goto(&mut app, "modified   file.txt");
    app.press("TAB").unwrap();
    // select "-l2" .. "+L2"
    goto(&mut app, "-l2");
    app.press("C-SPC").unwrap();
    app.press("C-n").unwrap();
    assert_eq!(app.point_text(), "+L2");
    app.press("s").unwrap();
    let staged = cached_diff(&g);
    assert!(staged.contains("-l2\n+L2") && !staged.contains("L4"), "{staged}");
    let unstaged = worktree_diff(&g);
    assert!(unstaged.contains("-l4\n+L4") && !unstaged.contains("+L2"), "{unstaged}");

    // discard "-l4" .. "+L4" in the worktree (confirmed)
    goto(&mut app, "-l4");
    app.press("C-SPC").unwrap();
    app.press("C-n").unwrap();
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(read(&g, "file.txt"), numbered(6).replace("l2\n", "L2\n"));
    assert!(worktree_diff(&g).is_empty());
}

#[test]
fn discard_hunk_needs_confirmation() {
    let (_td, g) = two_hunk_repo();
    let mut app = App::new(g.clone());
    goto(&mut app, "modified   file.txt");
    app.press("TAB").unwrap();
    goto(&mut app, "@@ -16,5 +16,5 @@");
    app.script_answers(&["n"]);
    app.press("k").unwrap();
    assert_eq!(app.message(), "Abort");
    assert!(read(&g, "file.txt").contains("L19"));
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    let content = read(&g, "file.txt");
    assert!(!content.contains("L19") && content.contains("L2\n"), "{content}");
}

#[test]
fn discard_staged_hunk_with_unstaged_changes() {
    let (_td, g) = two_hunk_repo();
    // stage both changes, then change line 19 again in the worktree
    g.git_interactive(&["add", "file.txt"]).unwrap();
    write(&g, "file.txt", &read(&g, "file.txt").replace("l10\n", "X10\n"));
    let mut app = App::new(g.clone());
    goto(&mut app, "Staged changes");
    app.press("n").unwrap();
    app.press("TAB").unwrap();
    goto(&mut app, "@@ -1,5 +1,5 @@");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    // removed from the index and from the worktree (--cached, then --reject)
    let staged = cached_diff(&g);
    assert!(!staged.contains("L2\n") && staged.contains("+L19"), "{staged}");
    let content = read(&g, "file.txt");
    assert!(content.starts_with("l1\nl2\n") && content.contains("X10"), "{content}");
}

#[test]
fn reverse_requires_staged_or_committed_change() {
    let (_td, g) = two_hunk_repo();
    let mut app = App::new(g.clone());
    goto(&mut app, "modified   file.txt");
    app.press("v").unwrap();
    assert_eq!(app.message(), "Cannot reverse unstaged changes");
    // magit-apply on unstaged changes
    goto(&mut app, "modified   file.txt");
    app.press("a").unwrap();
    assert_eq!(app.message(), "Change is already in the working tree");
    app.press("u").unwrap();
    assert_eq!(app.message(), "Already unstaged");
}

#[test]
fn stage_file_and_untracked_list() {
    let (_td, g) = two_hunk_repo();
    let mut app = App::new(g.clone());
    // s on the untracked section stages all untracked files
    assert_eq!(app.point_text(), "Untracked files (1)");
    app.press("s").unwrap();
    assert!(g.staged_files().contains(&"new.txt".to_string()));
    goto(&mut app, "modified   file.txt");
    app.press("s").unwrap();
    assert!(g.staged_files().contains(&"file.txt".to_string()));
    assert!(worktree_diff(&g).is_empty());
    // s on a staged file
    goto(&mut app, "Staged changes");
    app.press("n").unwrap();
    app.press("s").unwrap();
    assert_eq!(app.message(), "Already staged");
}

#[test]
fn visit_commit_and_hunk_line() {
    let (_td, g) = two_hunk_repo();
    let mut app = App::new(g);
    goto(&mut app, "Recent commits");
    app.press("TAB").unwrap();
    app.press("n").unwrap();
    assert!(app.point_text().ends_with("twenty"), "{}", app.point_text());
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { .. }), "{:?}", app.mode());
    assert!(app.header_line().starts_with("commit "), "{}", app.header_line());
    assert!(visible(&app, "twenty"));
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    assert!(app.point_text().ends_with("twenty"), "{}", app.point_text());

    // RET on a diff line visits the file at that line
    goto(&mut app, "modified   file.txt");
    app.press("TAB").unwrap();
    goto(&mut app, "+L19");
    app.press("RET").unwrap();
    assert_eq!(app.header_line(), "file.txt");
    assert_eq!(app.file_line(), Some(19));
}

#[test]
fn diff_context_keys() {
    let (_td, g) = two_hunk_repo();
    let mut app = App::new(g);
    goto(&mut app, "modified   file.txt");
    app.press("TAB").unwrap();
    app.press("+").unwrap();
    assert!(visible(&app, "@@ -1,6 +1,6 @@"), "{:#?}", app.visible_lines().iter().map(|l| &l.text).collect::<Vec<_>>());
    app.press("-").unwrap();
    app.press("-").unwrap();
    assert!(visible(&app, "@@ -1,4 +1,4 @@"));
    app.press("0").unwrap();
    assert!(visible(&app, "@@ -1,5 +1,5 @@"));
}

#[test]
fn headers_nest_under_the_first_header() {
    let (_td, g) = two_hunk_repo();
    g.git_interactive(&["tag", "v1"]).unwrap();
    let mut app = App::new(g);
    goto(&mut app, "Head:");
    assert!(visible(&app, "Tag:"));
    app.press("TAB").unwrap();
    assert!(!visible(&app, "Tag:"));
    assert!(app.point_text().starts_with("Head:"));
}
