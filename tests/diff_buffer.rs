//! Diff, revision and stash buffers (magit-diff-mode, magit-revision-mode,
//! magit-stash-mode): their layout, folding, the diffstat, applying and
//! reversing committed changes, visiting files and burying the buffer.

mod common;

use common::*;
use ngit_lib::git::diff;
use ngit_lib::prelude::*;
use ngit_lib::ui::app::{App, BufferMode};

fn numbered(n: usize) -> String {
    (1..=n).map(|i| format!("l{i}\n")).collect()
}

/// "twenty" adds file.txt (l1..l20) and gone.txt; HEAD ("second", with a
/// body) changes l2 to L2 and deletes gone.txt.
fn history_repo() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    write(&g, "file.txt", &numbered(20));
    write(&g, "gone.txt", "bye\n");
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", "twenty"]).unwrap();
    write(&g, "file.txt", &numbered(20).replace("l2\n", "L2\n"));
    g.git_interactive(&["rm", "-q", "gone.txt"]).unwrap();
    g.git_interactive(&["commit", "-q", "-am", "second\n\nbody line"]).unwrap();
    (td, g)
}

fn show_head(app: &mut App) {
    app.script_answers(&["HEAD"]);
    app.press("d").unwrap();
    app.press("c").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { .. }), "{:?}", app.mode());
}

fn texts(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

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
    panic!("no visible line starting with {prefix:?} in {:#?}", texts(app));
}

fn has(app: &App, line: &str) -> bool {
    app.visible_lines().iter().any(|l| l.text == line)
}

#[test]
fn revision_buffer_layout() {
    let (_td, g) = history_repo();
    let mut app = App::new(g.clone());
    show_head(&mut app);
    assert_eq!(app.header_line(), "commit HEAD");
    let t = texts(&app);
    let full = g.git_string(&["rev-parse", "HEAD"]).unwrap();
    let parent = g.git_string(&["rev-parse", "--short", "HEAD^"]).unwrap();
    assert_eq!(t[0], format!("master {full}"));
    assert!(t[1].starts_with("Author:     "), "{t:#?}");
    assert!(t[2].starts_with("AuthorDate: "), "{t:#?}");
    assert!(t[3].starts_with("Commit:     "), "{t:#?}");
    assert!(t[4].starts_with("CommitDate: "), "{t:#?}");
    assert_eq!(t[5], format!("Parent:     {parent} twenty"));
    let msg = t.iter().position(|l| l == "second").expect("summary line");
    assert_eq!(&t[msg..msg + 5], ["second", "", "body line", "", "2 files changed, 1 insertion(+), 2 deletions(-)"]);
    assert!(has(&app, "file.txt | 2 +-"));
    assert!(has(&app, "modified   file.txt"));
    assert!(has(&app, "@@ -1,5 +1,5 @@"));
    // the sections of deleted files start out hidden
    assert!(has(&app, "deleted    gone.txt"));
    assert!(!has(&app, "-bye"));
    goto(&mut app, "deleted    gone.txt");
    app.press("TAB").unwrap();
    assert!(has(&app, "-bye"));
    // TAB folds a file section
    goto(&mut app, "modified   file.txt");
    app.press("TAB").unwrap();
    assert!(!has(&app, "@@ -1,5 +1,5 @@"));
    app.press("TAB").unwrap();
    assert!(has(&app, "@@ -1,5 +1,5 @@"));
}

#[test]
fn empty_message_and_rename() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "a.txt", &numbered(20));
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", "base"]).unwrap();
    g.git_interactive(&["mv", "a.txt", "b.txt"]).unwrap();
    write(&g, "b.txt", &numbered(20).replace("l2\n", "L2\n"));
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "--allow-empty-message", "-m", ""]).unwrap();
    let mut app = App::new(g);
    show_head(&mut app);
    let t = texts(&app);
    // git prints a single newline for an empty message
    let c = t.iter().position(|l| l.starts_with("Contained:")).unwrap();
    assert!(!t.iter().any(|l| l == "(no message)"), "{t:#?}");
    assert_eq!(&t[c + 1..c + 4], ["", "", "1 file changed, 1 insertion(+), 1 deletion(-)"], "{t:#?}");
    let r = t.iter().position(|l| l == "renamed    a.txt -> b.txt").expect("rename heading");
    assert_eq!(&t[r + 1..r + 4], ["rename from a.txt", "rename to b.txt", "@@ -1,5 +1,5 @@"]);
}

#[test]
fn jump_between_diffstat_and_diff() {
    let (_td, g) = history_repo();
    let mut app = App::new(g);
    show_head(&mut app);
    goto(&mut app, "+L2");
    app.press("j").unwrap();
    assert_eq!(app.point_text(), "file.txt | 2 +-");
    app.press("j").unwrap();
    assert_eq!(app.point_text(), "modified   file.txt");
    goto(&mut app, "second");
    app.press("j").unwrap();
    assert_eq!(app.point_text(), "2 files changed, 1 insertion(+), 2 deletions(-)");
}

#[test]
fn unstage_and_reverse_committed_hunk() {
    let (_td, g) = history_repo();
    let mut app = App::new(g.clone());
    show_head(&mut app);
    goto(&mut app, "+L2");
    // u on a committed change: magit-reverse-in-index
    app.script_answers(&["y"]);
    app.press("u").unwrap();
    let cached = g.git_string(&["diff", "--cached"]).unwrap();
    assert!(cached.contains("-L2\n+l2"), "{cached}");
    assert_eq!(read(&g, "file.txt"), numbered(20).replace("l2\n", "L2\n"));
    // v: magit-reverse in the working tree
    goto(&mut app, "+L2");
    app.script_answers(&["y"]);
    app.press("v").unwrap();
    assert_eq!(read(&g, "file.txt"), numbered(20));
    // committed changes cannot be discarded, applying them again fails
    goto(&mut app, "+L2");
    app.press("k").unwrap();
    assert_eq!(app.message(), "Cannot discard committed changes");
}

#[test]
fn apply_and_reverse_require_the_right_type() {
    let (_td, g) = history_repo();
    write(&g, "file.txt", &numbered(20).replace("l5\n", "L5\n"));
    let mut app = App::new(g);
    app.press("d").unwrap();
    app.press("u").unwrap();
    assert_eq!(app.header_line(), "Unstaged changes");
    goto(&mut app, "+L5");
    app.press("a").unwrap();
    assert_eq!(app.message(), "Change is already in the working tree");
    app.press("v").unwrap();
    assert_eq!(app.message(), "Cannot reverse unstaged changes");
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
}

#[test]
fn stash_buffer_and_apply_hunk() {
    let (_td, g) = history_repo();
    write(&g, "file.txt", &numbered(20).replace("l2\n", "L2\n").replace("l9\n", "L9\n"));
    g.git_interactive(&["stash", "-q"]).unwrap();
    let mut app = App::new(g.clone());
    app.script_answers(&["stash@{0}"]);
    app.press("d").unwrap();
    app.press("t").unwrap();
    assert!(matches!(app.mode(), BufferMode::Stash { .. }));
    assert!(app.header_line().starts_with("Stash@{0} WIP on master: "), "{}", app.header_line());
    let t = texts(&app);
    assert_eq!(t[0], "Unstaged", "{t:#?}");
    assert_eq!(t[1], "modified   file.txt", "{t:#?}");
    // no diffstat (stash buffers drop --stat), empty parts are left out
    assert!(!t.iter().any(|l| l == "Staged" || l == "Untracked files"), "{t:#?}");
    goto(&mut app, "+L9");
    app.press("j").unwrap();
    assert_eq!(app.message(), "No diffstat in this buffer");
    app.press("a").unwrap();
    assert_eq!(read(&g, "file.txt"), numbered(20).replace("l2\n", "L2\n").replace("l9\n", "L9\n"));
    // q returns to the status buffer, refreshed
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    assert!(has(&app, "Unstaged changes (1)"), "{:#?}", texts(&app));
}

#[test]
fn stash_buffer_untracked_part() {
    let (_td, g) = history_repo();
    write(&g, "new.txt", "new\n");
    g.git_interactive(&["stash", "-q", "-u"]).unwrap();
    let mut app = App::new(g);
    goto(&mut app, "Stashes");
    app.press("TAB").unwrap();
    app.press("n").unwrap();
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Stash { .. }), "{:?}", app.mode());
    let t = texts(&app);
    assert_eq!(t[0], "Untracked files", "{t:#?}");
    assert_eq!(t[1], "new file   new.txt", "{t:#?}");
}

#[test]
fn visit_old_and_new_side_of_a_commit() {
    let (_td, g) = history_repo();
    write(&g, "file.txt", &format!("top\n{}", numbered(20).replace("l2\n", "L2\n")));
    let parent = g.git_string(&["rev-parse", "--short", "HEAD^"]).unwrap();
    let mut app = App::new(g);
    show_head(&mut app);
    // a removed line visits the old version (REV^, abbreviated), at its line
    goto(&mut app, "-l2");
    app.press("RET").unwrap();
    assert_eq!(app.header_line(), format!("file.txt.~{parent}~"));
    assert_eq!(texts(&app)[..2], ["l1", "l2"]);
    assert_eq!(app.file_line(), Some(2));
    app.press("q").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { .. }));
    // an added line visits the new version (REV)
    goto(&mut app, "+L2");
    app.press("RET").unwrap();
    assert_eq!(app.header_line(), "file.txt.~HEAD~");
    assert_eq!(app.file_line(), Some(2));
    app.press("q").unwrap();
    // C-RET visits the worktree file, at the line moved by the
    // uncommitted change above it
    goto(&mut app, "+L2");
    app.press("C-RET").unwrap();
    assert_eq!(app.header_line(), "file.txt");
    assert_eq!(texts(&app)[0], "top");
    assert_eq!(app.file_line(), Some(3));
    app.press("q").unwrap();
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
}

#[test]
fn diff_options_read_values() {
    let (_td, g) = history_repo();
    let mut app = App::new(g);
    show_head(&mut app);
    // -U reads a number; the hunk then has that much context
    app.press("D").unwrap();
    app.script_answers(&["x"]);
    app.press("-").unwrap();
    app.press("U").unwrap();
    assert_eq!(app.message(), "Please enter a non-negative integer");
    app.script_answers(&["0"]);
    app.press("-").unwrap();
    app.press("U").unwrap();
    assert!(app.active_switches().contains(&"-U0".to_string()), "{:?}", app.active_switches());
    // -A offers the algorithms ("unspecified" unsets)
    app.script_answers(&["pat"]);
    app.press("-").unwrap();
    app.press("A").unwrap();
    assert!(app.active_switches().contains(&"--diff-algorithm=patience".to_string()));
    app.script_answers(&["hist"]);
    app.press("-").unwrap();
    app.press("A").unwrap();
    assert!(app.active_switches().contains(&"--diff-algorithm=histogram".to_string()));
    assert!(!app.active_switches().contains(&"--diff-algorithm=patience".to_string()));
    app.script_answers(&["u"]);
    app.press("-").unwrap();
    app.press("A").unwrap();
    assert!(!app.active_switches().iter().any(|a| a.starts_with("--diff-algorithm")));
    // -M rejects what is not a similarity index
    app.script_answers(&["half"]);
    app.press("-").unwrap();
    app.press("M").unwrap();
    assert!(app.message().starts_with("Please enter a percentage"), "{}", app.message());
    app.press("g").unwrap();
    assert!(texts(&app).iter().any(|l| l.starts_with("@@ -2 +2 @@")), "{:#?}", texts(&app));
    // typing an option that is set unsets it
    app.press("D").unwrap();
    app.press("-").unwrap();
    app.press("U").unwrap();
    assert!(!app.active_switches().iter().any(|a| a.starts_with("-U")));
    app.press("g").unwrap();
    assert!(texts(&app).iter().any(|l| l.starts_with("@@ -1,5 +1,5 @@")), "{:#?}", texts(&app));
}

#[test]
fn diff_refresh_changes_only_the_buffer_shown() {
    let (_td, g) = history_repo();
    let mut app = App::new(g);
    show_head(&mut app);
    assert!(has(&app, "file.txt | 2 +-"));
    // D starts with the arguments of the buffer; toggling --stat and
    // refreshing removes the diffstat
    app.press("D").unwrap();
    assert!(app.active_switches().contains(&"--stat".to_string()), "{:?}", app.active_switches());
    for k in stat_switch_key(&app).chars() {
        app.press(&k.to_string()).unwrap();
    }
    app.press("g").unwrap();
    assert!(!has(&app, "file.txt | 2 +-"));
    assert!(has(&app, "modified   file.txt"));
    // new buffers still start with the default arguments
    app.press("q").unwrap();
    show_head(&mut app);
    assert!(has(&app, "file.txt | 2 +-"));
}

#[test]
fn diff_menu_arguments_apply_to_the_new_buffer_only() {
    let (_td, g) = history_repo();
    write(&g, "file.txt", &numbered(20));
    let mut app = App::new(g);
    // `D` in the status buffer: magit-status-mode's default arguments, and
    // no "--stat" switch (magit-diff-argument-predicate)
    app.press("D").unwrap();
    assert_eq!(app.active_switches(), ["--ignore-submodules=none", "--no-ext-diff"]);
    app.press("-").unwrap();
    app.press("s").unwrap();
    assert_eq!(app.message(), "Unbound suffix: `- s' (Use `C-g' to abort, `?' for help) [nil]");
    app.press("q").unwrap();
    // `d -s u`: the diff buffer uses the transient's arguments
    app.press("d").unwrap();
    assert_eq!(app.active_switches(), ["--no-ext-diff", "--stat"]);
    app.press("-").unwrap();
    app.press("s").unwrap();
    app.press("u").unwrap();
    assert_eq!(app.header_line(), "Unstaged changes");
    assert!(!has(&app, "file.txt | 2 +-"), "{:#?}", texts(&app));
    assert!(has(&app, "modified   file.txt"));
    // `d` in a diff buffer starts with that buffer's arguments
    app.press("d").unwrap();
    assert_eq!(app.active_switches(), ["--no-ext-diff"]);
    app.press("q").unwrap();
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    // ... but the default for new buffers did not change
    app.press("d").unwrap();
    app.press("u").unwrap();
    assert!(has(&app, "file.txt | 2 +-"), "{:#?}", texts(&app));
}

/// The key of the "--stat" switch in the diff transients.
fn stat_switch_key(_app: &App) -> String {
    use ngit_lib::ui::transient::{registry, Action};
    let menus = registry();
    let menu = &menus["diff-refresh-menu"];
    for group in &menu.groups {
        for e in group {
            if matches!(&e.action, Action::Switch(s) if s == "--stat") {
                return e.key.clone();
            }
        }
    }
    panic!("no --stat switch");
}

#[test]
fn hunk_header_drops_mode_and_rename_lines() {
    let raw = "diff --git x.sh y.sh\nold mode 100644\nnew mode 100755\nsimilarity index 90%\nrename from x.sh\nrename to y.sh\nindex 1111111..2222222\n--- x.sh\n+++ y.sh\n@@ -1 +1 @@\n-a\n+b\n";
    let files = diff::parse(raw);
    assert_eq!(files.len(), 1);
    let f = &files[0];
    assert_eq!(f.modes.as_deref(), Some("old mode 100644\nnew mode 100755"));
    assert_eq!(f.rename.as_deref(), Some("rename from x.sh\nrename to y.sh"));
    let h = f.hunk_header();
    assert!(!h.contains("mode"), "{h}");
    assert!(!h.contains("rename"), "{h}");
    assert!(h.contains("\n--- y.sh\n+++ y.sh\n"), "{h}");
}
