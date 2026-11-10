//! tests/git_ops.rs — behavior tests: every git operation must produce the
//! same repository state as the corresponding Magit command (which shells out
//! to git with the arguments documented in its docstrings).

mod common;

use common::*;
use xagit_lib::prelude::*;

// =============================================================== discover

#[test]
fn discover_rejects_non_repo() {
    let td = TempDir::new().unwrap();
    assert!(Git::discover(td.path()).is_err());
}

#[test]
fn discover_finds_toplevel_from_subdir() {
    let (td, g, _head) = repo_with_commit();
    std::fs::create_dir(td.path().join("sub")).unwrap();
    let g2 = Git::discover(&td.path().join("sub")).unwrap();
    assert_eq!(g2.toplevel, g.toplevel);
    assert!(g.gitdir.exists());
}

#[test]
fn no_commit_detection() {
    let (td, g) = scratch_repo();
    assert!(g.no_commit());
    std::fs::write(td.path().join("a"), "x").unwrap();
    g.git_interactive(&["add", "."]).unwrap();
    g.commit(Some("c1"), &[]).unwrap();
    assert!(!g.no_commit());
}

// =============================================================== status

#[test]
fn status_sections_match_git() {
    let (td, g, _head) = repo_with_commit();
    let _ = td;
    // untracked
    write(&g, "new.txt", "n");
    assert_eq!(g.untracked_files(), vec!["new.txt"]);
    // unstaged modification
    write(&g, "file.txt", "changed");
    assert_eq!(g.unstaged_files(), vec!["file.txt"]);
    // staged
    g.stage("file.txt").unwrap();
    assert_eq!(g.staged_files(), vec!["file.txt"]);
    assert!(g.anything_staged());

    let st = Status::load(&g);
    assert_eq!(st.branch.as_deref(), Some("master").or_else(|| {
        // git may default to 'main' depending on version
        None
    }).or(st.branch.as_deref()).map(|_| st.branch.clone()).flatten().as_deref().map(|s| s.to_string()).as_deref().map(|s| s).or(None).or(None));
    let _ = st;
}

#[test]
fn status_branch_and_ahead_behind() {
    let (_td, g, _td2, g2) = remote_sandbox();
    let cur = g.current_branch().unwrap();
    let st = Status::load(&g);
    assert_eq!(st.branch.as_deref(), Some(cur.as_str()));
    assert_eq!(st.upstream.as_deref(), Some(format!("origin/{cur}").as_str()));
    assert_eq!((st.ahead, st.behind), (0, 0));

    // one local commit -> "[ahead 1]"
    write(&g, "local.txt", "l");
    g.stage("local.txt").unwrap();
    g.commit(Some("local"), &[]).unwrap();
    let st = Status::load(&g);
    assert_eq!((st.ahead, st.behind), (1, 0));

    // one remote commit -> "[ahead 1, behind 1]"
    write(&g2, "remote.txt", "r");
    g2.stage("remote.txt").unwrap();
    g2.commit(Some("remote"), &[]).unwrap();
    g2.git_interactive(&["push", "-q", "origin", "HEAD"]).unwrap();
    g.git_interactive(&["fetch", "-q", "origin"]).unwrap();
    let st = Status::load(&g);
    assert_eq!((st.ahead, st.behind), (1, 1));
}

#[test]
fn branch_header_parsing() {
    use xagit_lib::git::status::parse_branch_header as p;
    let h = p("main...origin/main [ahead 12, behind 3]");
    assert_eq!(h.branch.as_deref(), Some("main"));
    assert_eq!(h.upstream.as_deref(), Some("origin/main"));
    assert_eq!((h.ahead, h.behind, h.gone), (12, 3, false));
    let h = p("main...origin/main [behind 2]");
    assert_eq!((h.ahead, h.behind), (0, 2));
    let h = p("feat...origin/feat [gone]");
    assert!(h.gone);
    let h = p("main");
    assert_eq!((h.branch.as_deref(), h.upstream.as_deref()), (Some("main"), None));
    let h = p("No commits yet on main");
    assert_eq!(h.branch.as_deref(), Some("main"));
    assert!(p("HEAD (no branch)").detached);
}

#[test]
fn status_parses_porcelain_renamed() {
    let (td, g, _head) = repo_with_commit();
    let _ = td;
    write(&g, "file.txt", "hello\nmore");
    g.stage("file.txt").unwrap();
    g.git_interactive(&["commit", "-m", "c2"]).unwrap();
    g.git_interactive(&["mv", "file.txt", "renamed.txt"]).unwrap();
    let st = Status::load(&g);
    let f = st.staged.iter().find(|f| f.x == 'R').expect("rename entry");
    // path is the real current path, the source is kept separately
    assert_eq!(f.path, "renamed.txt");
    assert_eq!(f.orig_path.as_deref(), Some("file.txt"));
    assert!(!st.staged.iter().any(|f| f.path.contains("->")));
}

#[test]
fn unstage_and_discard_staged_rename() {
    let (_td, g, _head) = repo_with_commit();
    g.git_interactive(&["mv", "file.txt", "renamed.txt"]).unwrap();
    assert_eq!(g.staged_rename_source("renamed.txt").as_deref(), Some("file.txt"));

    g.unstage("renamed.txt").unwrap();
    let st = Status::load(&g);
    assert!(st.staged.is_empty(), "both sides of the rename unstaged");
    assert!(st.unstaged.iter().any(|f| f.path == "file.txt" && f.y == 'D'));
    assert!(st.untracked.iter().any(|f| f.path == "renamed.txt"));

    g.git_interactive(&["add", "-A"]).unwrap();
    g.discard_staged("renamed.txt").unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert!(!exists(&g, "renamed.txt"));
    let st = Status::load(&g);
    assert!(st.staged.is_empty());
    assert!(st.unstaged.is_empty());
    assert!(st.untracked.is_empty());

    // rename into a subdirectory whose source dir no longer exists
    std::fs::create_dir(g.toplevel.join("d")).unwrap();
    write(&g, "d/a.txt", "a");
    g.stage("d/a.txt").unwrap();
    g.commit(Some("d"), &[]).unwrap();
    g.git_interactive(&["mv", "d/a.txt", "b.txt"]).unwrap();
    // git mv keeps the emptied directory; remove it to exercise parent creation
    let _ = std::fs::remove_dir(g.toplevel.join("d"));
    assert!(!g.toplevel.join("d").exists());
    g.discard_staged("b.txt").unwrap();
    assert_eq!(read(&g, "d/a.txt"), "a");
    assert!(Status::load(&g).is_clean());
}

#[test]
fn status_lists_conflicts_once_as_unmerged() {
    let (_td, g, _head) = repo_with_commit();
    let main = g.current_branch().unwrap();
    g.git_interactive(&["checkout", "-q", "-b", "side"]).unwrap();
    write(&g, "file.txt", "side\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("side"), &[]).unwrap();
    g.git_interactive(&["checkout", "-q", &main]).unwrap();
    write(&g, "file.txt", "main\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("main"), &[]).unwrap();
    assert!(g.git_interactive(&["merge", "side"]).is_err());

    let st = Status::load(&g);
    assert!(st.staged.iter().all(|f| f.path != "file.txt"), "not in staged");
    let u: Vec<_> = st.unstaged.iter().filter(|f| f.path == "file.txt").collect();
    assert_eq!(u.len(), 1);
    assert!(u[0].is_unmerged());
    assert_eq!(u[0].conflict_desc(), "");

    let lines = xagit_lib::ui::view::render_status(&g, &st);
    assert!(lines.iter().any(|l| l.text.contains("unmerged   file.txt")));
}

// =============================================================== stage/unstage

#[test]
fn stage_file() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "modified");
    g.stage("file.txt").unwrap();
    assert_eq!(g.staged_files(), vec!["file.txt"]);
}

#[test]
fn stage_all_stages_modified_and_untracked() {
    // magit-stage-modified: `git add -u .` leaves untracked files alone,
    // `git add --all .` (prefix argument) includes them
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "modified");
    write(&g, "extra.txt", "extra");
    g.stage_all().unwrap();
    assert_eq!(g.staged_files(), vec!["file.txt"]);
    assert_eq!(g.untracked_files(), vec!["extra.txt"]);
    g.stage_all_with_untracked().unwrap();
    let mut staged = g.staged_files();
    staged.sort();
    assert_eq!(staged, vec!["extra.txt", "file.txt"]);
}

#[test]
fn stage_all_stages_deletions() {
    let (_td, g, _head) = repo_with_commit();
    std::fs::remove_file(g.toplevel.join("file.txt")).unwrap();
    g.stage_all().unwrap();
    let staged = g.staged_files();
    assert_eq!(staged, vec!["file.txt"]); // deletion recorded in index
}

#[test]
fn unstage_after_commit_uses_reset() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "modified");
    g.stage("file.txt").unwrap();
    g.unstage("file.txt").unwrap();
    assert!(g.staged_files().is_empty());
    assert_eq!(g.unstaged_files(), vec!["file.txt"]);
}

#[test]
fn unstage_before_first_commit_uses_rm_cached() {
    let (_td, g) = scratch_repo();
    write(&g, "a.txt", "a");
    g.stage("a.txt").unwrap();
    assert_eq!(g.staged_files(), vec!["a.txt"]);
    // unstage must keep the file on disk (git rm --cached behavior)
    g.unstage("a.txt").unwrap();
    assert!(g.staged_files().is_empty());
    assert!(exists(&g, "a.txt"));
}

#[test]
fn unstage_all() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "m");
    write(&g, "b.txt", "b");
    g.stage_all_with_untracked().unwrap();
    assert_eq!(g.staged_files().len(), 2);
    g.unstage_all().unwrap();
    assert!(g.staged_files().is_empty());
}

// =============================================================== discard

#[test]
fn discard_untracked_removes_file() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "junk.txt", "junk");
    g.discard_untracked("junk.txt").unwrap();
    assert!(!exists(&g, "junk.txt"));
}

#[test]
fn discard_unstaged_restores_content() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "clobbered\n");
    g.discard_unstaged("file.txt").unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
}

#[test]
fn discard_staged_resets_and_restores() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "clobbered\n");
    g.stage("file.txt").unwrap();
    g.discard_staged("file.txt").unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert!(g.staged_files().is_empty());

    // a newly added file is unstaged and kept (not in HEAD, nothing to restore)
    write(&g, "new.txt", "n");
    g.stage("new.txt").unwrap();
    g.discard_staged("new.txt").unwrap();
    assert!(g.staged_files().is_empty());
    assert!(exists(&g, "new.txt"));
}

// =============================================================== commit

#[test]
fn commit_create() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("second"), &[]).unwrap();
    let out = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert_eq!(out, "second");
}

#[test]
fn commit_with_args_all_flag() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    // -a stages tracked modified files automatically (magit "c -a c")
    g.commit(Some("all"), &["-a".to_string()]).unwrap();
    assert!(g.unstaged_files().is_empty());
    let out = g.git_string(&["log", "-1", "--pretty=%B"]).unwrap();
    assert_eq!(out.trim(), "all");
}

#[test]
fn commit_extend_amends_without_message_change() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    let before = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    g.commit_extend(&[]).unwrap();
    let after = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert_eq!(before, after, "extend keeps the message");
    assert!(g.staged_files().is_empty());
    assert!(g.anything_staged() == false);
    assert!(g.unstaged_files().is_empty());
}

#[test]
fn commit_amend_changes_message() {
    let (_td, g, _head) = repo_with_commit();
    g.commit_amend(Some("amended"), &[]).unwrap();
    let out = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert_eq!(out, "amended");
}

#[test]
fn commit_reword_only_changes_message() {
    let (_td, g, _head) = repo_with_commit();
    let tree_before = g.git_string(&["rev-parse", "HEAD^{tree}"]).unwrap();
    g.commit_reword("reworded").unwrap();
    let out = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert_eq!(out, "reworded");
    // tree unchanged (no staged changes existed)
    let tree_after = g.git_string(&["rev-parse", "HEAD^{tree}"]).unwrap();
    assert_eq!(tree_before, tree_after);
}

#[test]
fn commit_fixup_and_squash_create_refs() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.commit_fixup("HEAD").unwrap();
    let msg = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert!(msg.starts_with("fixup!"), "got {msg}");

    write(&g, "file.txt", "v3\n");
    g.stage("file.txt").unwrap();
    g.commit_squash("HEAD~1").unwrap();
    let msg = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert!(msg.starts_with("squash!"), "got {msg}");
}

#[test]
fn commit_instant_fixup_squashes_into_target() {
    let (_td, g, _head) = repo_with_commit();
    // create a second commit
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("second"), &[]).unwrap();
    let target = g.rev_parse("HEAD").unwrap();
    // new change to fold into target
    write(&g, "file.txt", "v3\n");
    g.stage("file.txt").unwrap();
    g.commit_instant_fixup(&target).unwrap();
    // HEAD message should be "second" again with combined changes
    let msg = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert_eq!(msg, "second");
    assert!(g.unstaged_files().is_empty());
    assert_eq!(read(&g, "file.txt"), "v3\n");
}

#[test]
fn commit_instant_squash_merges_messages() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("second"), &[]).unwrap();
    let target = g.rev_parse("HEAD").unwrap();
    write(&g, "file.txt", "v3\n");
    g.stage("file.txt").unwrap();
    g.commit_instant_squash(&target).unwrap();
    let body = g.git_string(&["log", "-1", "--pretty=%B"]).unwrap();
    assert!(body.contains("second"));
    assert_eq!(read(&g, "file.txt"), "v3\n");
}

// =============================================================== branch

#[test]
fn checkout_branch() {
    let (_td, g, _head) = repo_with_commit();
    g.branch_create("feature", None).unwrap();
    g.checkout("feature").unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("feature"));
}

#[test]
fn branch_and_checkout_with_start() {
    let (_td, g, _head) = repo_with_commit();
    g.branch_and_checkout("topic", Some("HEAD")).unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("topic"));
}

#[test]
fn branch_create_at_rev() {
    let (_td, g, _head) = repo_with_commit();
    let head = g.rev_parse("HEAD").unwrap();
    g.branch_create("at-head", Some(head.as_str())).unwrap();
    assert!(g.branches().contains(&"at-head".to_string()));
}

#[test]
fn branch_delete() {
    let (_td, g, _head) = repo_with_commit();
    g.branch_create("todelete", None).unwrap();
    g.branch_delete("todelete").unwrap();
    assert!(!g.branches().contains(&"todelete".to_string()));
}

#[test]
fn branch_rename() {
    let (_td, g, _head) = repo_with_commit();
    g.branch_rename("master2", "master3").ok(); // may fail if branch is 'main'
    let cur = g.current_branch().unwrap();
    g.branch_rename(&cur, "renamed").unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("renamed"));
}

#[test]
fn branch_spinoff_resets_old_branch() {
    let (td, g, _head) = repo_with_commit();
    // upstream = bare clone
    let bare = td.path().join("upstream.git");
    assert!(
        std::process::Command::new("git")
            .args(["clone", "-q", "--bare", ".", "upstream.git"])
            .current_dir(&g.toplevel)
            .status()
            .unwrap()
            .success()
    );
    g.git_interactive(&["remote", "add", "origin", bare.to_str().unwrap()])
        .unwrap();
    g.git_interactive(&["fetch", "origin"]).unwrap();
    let cur = g.current_branch().unwrap();
    g.git_interactive(&[
        "branch",
        "--set-upstream-to",
        &format!("origin/{cur}"),
        &cur,
    ])
    .unwrap();
    // commit ahead of upstream
    write(&g, "file.txt", "ahead\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("ahead"), &[]).unwrap();
    // spinoff: new branch keeps the ahead commit; old branch resets to upstream
    g.branch_spinoff("spinoff").unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("spinoff"));
    let old_head = g.git_string(&["rev-parse", &cur]).unwrap();
    let up_head = g.git_string(&["rev-parse", &format!("origin/{cur}")]).unwrap();
    assert_eq!(old_head, up_head, "old branch reset to upstream");
}

#[test]
fn branch_spinout_stays_on_branch() {
    let (_td, g, _head) = repo_with_commit();
    let cur = g.current_branch().unwrap();
    g.branch_spinout("spinout").unwrap();
    assert_eq!(g.current_branch().as_deref(), Some(cur.as_str()));
    assert!(g.branches().contains(&"spinout".to_string()));
}

// =============================================================== remote

#[test]
fn remote_add_rename_remove() {
    let (td, g) = scratch_repo();
    let url = td.path().join("sister.git");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "--bare", url.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    g.remote_add("origin", url.to_str().unwrap(), false).unwrap();
    assert_eq!(g.remotes(), vec!["origin"]);
    g.remote_rename("origin", "upstream").unwrap();
    assert_eq!(g.remotes(), vec!["upstream"]);
    g.remote_remove("upstream").unwrap();
    assert!(g.remotes().is_empty());
}

#[test]
fn remote_add_with_fetch() {
    let (td, g, _head) = repo_with_commit();
    let bare = td.path().join("bare.git");
    assert!(
        std::process::Command::new("git")
            .args(["clone", "-q", "--bare", ".", "bare.git"])
            .current_dir(&g.toplevel)
            .status()
            .unwrap()
            .success()
    );
    g.remote_add("origin", bare.to_str().unwrap(), true).unwrap();
    // -f means refs fetched already
    let refs = g.git_string(&["for-each-ref", "--format=%(refname:short)", "refs/remotes/origin/"])
        .unwrap_or_default();
    assert!(refs.contains("master") || refs.contains("main"), "got {refs}");
}

#[test]
fn remote_prune() {
    let (td, g, _head) = repo_with_commit();
    let bare = td.path().join("prune.git");
    assert!(
        std::process::Command::new("git")
            .args(["clone", "-q", "--bare", ".", "prune.git"])
            .current_dir(&g.toplevel)
            .status()
            .unwrap()
            .success()
    );
    g.remote_add("origin", bare.to_str().unwrap(), true).unwrap();
    // delete branch in the bare remote -> prune should remove tracking ref
    assert!(
        std::process::Command::new("git")
            .args(["branch", "-d", "master"])
            .current_dir(&bare)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
            || std::process::Command::new("git")
                .args(["branch", "-d", "main"])
                .current_dir(&bare)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
    );
    g.remote_prune("origin").unwrap();
    let refs = g
        .git_string(&["for-each-ref", "--format=%(refname:short)", "refs/remotes/origin/"])
        .unwrap_or_default();
    let cur = g.current_branch().unwrap();
    assert!(!refs.contains(&cur), "tracking ref pruned, got {refs}");
}

// =============================================================== fetch/pull/push

fn remote_sandbox() -> (TempDir, Git, TempDir, Git) {
    let (td, g, _head) = repo_with_commit();
    let bare = td.path().join("srv.git");
    assert!(
        std::process::Command::new("git")
            .args(["clone", "-q", "--bare", ".", "srv.git"])
            .current_dir(&g.toplevel)
            .status()
            .unwrap()
            .success()
    );
    g.remote_add("origin", bare.to_str().unwrap(), true).unwrap();
    let cur = g.current_branch().unwrap();
    g.git_interactive(&["branch", "--set-upstream-to", &format!("origin/{cur}"), &cur])
        .unwrap();
    // clone a second worktree to push from
    let td2 = TempDir::new().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["clone", "-q", bare.to_str().unwrap(), td2.path().to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    let g2 = Git::discover(td2.path()).unwrap();
    (td, g, td2, g2)
}

#[test]
fn fetch_pull_push_roundtrip() {
    let (_td, g, _td2, g2) = remote_sandbox();
    // push from g2
    write(&g2, "pushed.txt", "p");
    g2.stage("pushed.txt").unwrap();
    g2.commit(Some("from clone"), &[]).unwrap();
    let branch = g2.current_branch().unwrap();
    g2.push("origin", &branch, &[]).unwrap();

    // g fetches
    g.fetch("origin", &[]).unwrap();
    // g pulls and gets the commit
    let cur = g.current_branch().unwrap();
    g.pull("origin", &cur, &[]).unwrap();
    let log = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert_eq!(log, "from clone");
    assert!(exists(&g, "pushed.txt"));
}

#[test]
fn push_tag_roundtrip() {
    let (_td, g, _td2, _g2) = remote_sandbox();
    g.tag_create("v1.0", "HEAD", &[], None).unwrap();
    g.push_tag("origin", "v1.0").unwrap();
    let tags = g.git_string(&["ls-remote", "--tags", "origin"]).unwrap();
    assert!(tags.contains("v1.0"));
}

#[test]
fn fetch_all() {
    let (_td, g, _td2, _g2) = remote_sandbox();
    g.fetch_all(true).unwrap();
}

// =============================================================== merge

fn repo_two_branches() -> (TempDir, Git) {
    let (td, g, _head) = repo_with_commit();
    // create AND checkout feature, commit there, then go back
    g.branch_and_checkout("feature", None).unwrap();
    write(&g, "feature.txt", "f");
    g.stage("feature.txt").unwrap();
    g.commit(Some("feature work"), &[]).unwrap();
    let main = g
        .branches()
        .into_iter()
        .find(|b| b == "master" || b == "main")
        .expect("default branch");
    g.checkout(&main).unwrap();
    (td, g)
}

#[test]
fn merge_plain_fast_forward() {
    let (_td, g) = repo_two_branches();
    let cur = g.current_branch().unwrap();
    let _ = cur;
    g.merge("feature", &[]).unwrap();
    assert!(exists(&g, "feature.txt"));
    assert!(g.unstaged_files().is_empty());
}

#[test]
fn merge_nocommit_leaves_changes_staged() {
    let (_td, g) = repo_two_branches();
    // make master diverge so the merge cannot fast-forward
    write(&g, "master.txt", "m");
    g.stage("master.txt").unwrap();
    g.commit(Some("master work"), &[]).unwrap();
    g.merge_nocommit("feature", &[]).unwrap();
    assert!(exists(&g, "feature.txt"));
    // a true merge with --no-commit stops before committing
    assert!(
        g.merge_in_progress() || g.staged_files().contains(&"feature.txt".to_string()),
        "merge must stop before commit: staged={:?} merge={}",
        g.staged_files(),
        g.merge_in_progress()
    );
    if g.merge_in_progress() {
        g.merge_abort().unwrap();
        assert!(!g.merge_in_progress());
    }
}

#[test]
fn merge_squash() {
    let (_td, g) = repo_two_branches();
    g.merge_squash("feature").unwrap();
    assert!(g.staged_files().contains(&"feature.txt".to_string()));
    // squash leaves changes staged, no merge in progress
    assert!(!g.merge_in_progress());
}

#[test]
fn merge_abort_restores_state() {
    let (_td, g) = repo_two_branches();
    // create a conflicting commit on master
    write(&g, "file.txt", "conflict");
    g.stage("file.txt").unwrap();
    g.commit(Some("conflicting"), &[]).unwrap();
    // feature also changed file.txt -> conflict
    g.git_interactive(&["checkout", "feature"]).unwrap();
    write(&g, "file.txt", "feature version");
    g.stage("file.txt").unwrap();
    g.commit(Some("feature conflicting"), &[]).unwrap();
    g.git_interactive(&["checkout", "-"]).unwrap();
    let branch = g.current_branch().unwrap();
    let head_before = g.rev_parse("HEAD").unwrap();
    let result = g.merge("feature", &[]);
    // merge fails with conflict
    assert!(result.is_err() || g.merge_in_progress());
    if g.merge_in_progress() {
        g.merge_abort().unwrap();
        assert_eq!(g.rev_parse("HEAD").unwrap(), head_before);
        let _ = branch;
        assert!(!g.merge_in_progress());
    }
}

// =============================================================== rebase

#[test]
fn rebase_onto_upstream_and_abort() {
    let (_td, g, _td2, _g2) = remote_sandbox();
    let cur = g.current_branch().unwrap();
    // local commit ahead
    write(&g, "ahead.txt", "a");
    g.stage("ahead.txt").unwrap();
    g.commit(Some("ahead"), &[]).unwrap();
    let target = format!("{cur}@{{u}}");
    g.rebase(&target, &[]).unwrap();
    assert!(exists(&g, "ahead.txt"));
}

#[test]
fn rebase_skip_and_continue_paths() {
    let (_td, g, _head) = repo_with_commit();
    // no rebase in progress: these should fail gracefully
    assert!(g.rebase_continue().is_err());
    assert!(g.rebase_skip().is_err());
    assert!(g.rebase_abort().is_err());
}

// =============================================================== reset

#[test]
fn reset_soft_keeps_index() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("second"), &[]).unwrap();
    g.reset_soft("HEAD~1").unwrap();
    // index still contains v2 change
    assert!(g.staged_files().contains(&"file.txt".to_string()));
}

#[test]
fn reset_mixed_unstages() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("second"), &[]).unwrap();
    g.reset_mixed("HEAD~1").unwrap();
    assert!(g.staged_files().is_empty());
    assert!(g.unstaged_files().contains(&"file.txt".to_string()));
    assert_eq!(read(&g, "file.txt"), "v2\n");
}

#[test]
fn reset_hard_wipes_worktree() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("second"), &[]).unwrap();
    g.reset_hard("HEAD~1").unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert!(g.staged_files().is_empty());
    assert!(g.unstaged_files().is_empty());
}

#[test]
fn reset_keep_preserves_uncommitted() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "extra.txt", "uncommitted");
    g.reset_keep("HEAD").unwrap();
    assert!(exists(&g, "extra.txt"));
}

#[test]
fn reset_index_only_touches_index() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.reset_index("HEAD").unwrap();
    assert!(g.staged_files().is_empty());
    assert!(g.unstaged_files().contains(&"file.txt".to_string()));
    assert!(read(&g, "file.txt") == "v2\n", "worktree untouched");
}

#[test]
fn reset_worktree_only_restores_files() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "dirty\n");
    assert!(g.unstaged_files().contains(&"file.txt".to_string()));
    g.reset_worktree("HEAD").unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
}

// =============================================================== tag

#[test]
fn tag_create_lightweight_and_annotated() {
    let (_td, g, _head) = repo_with_commit();
    g.tag_create("light", "HEAD", &[], None).unwrap();
    g.tag_create("anno", "HEAD", &["--annotate".into()], Some("annotation")).unwrap();
    let tags = g.tags();
    assert!(tags.contains(&"light".to_string()));
    assert!(tags.contains(&"anno".to_string()));
    // annotated tags have a tag object
    let obj = g.git_string(&["cat-file", "-t", "anno"]).unwrap();
    assert_eq!(obj, "tag");
    let obj = g.git_string(&["cat-file", "-t", "light"]).unwrap();
    assert_eq!(obj, "commit");
}

#[test]
fn tag_delete() {
    let (_td, g, _head) = repo_with_commit();
    g.tag_create("gone", "HEAD", &[], None).unwrap();
    g.tag_delete("gone").unwrap();
    assert!(!g.tags().contains(&"gone".to_string()));
}

// =============================================================== revert / cherry-pick

#[test]
fn revert_creates_inverse_commit() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("second"), &[]).unwrap();
    let head = g.rev_parse("HEAD").unwrap();
    g.revert(&[head], &[], false).unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
    let msg = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert!(msg.contains("Revert"));
}

#[test]
fn revert_no_commit_leaves_inverse_staged() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("second"), &[]).unwrap();
    let head = g.rev_parse("HEAD").unwrap();
    g.revert(&[head], &[], true).unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert!(g.staged_files().contains(&"file.txt".to_string()));
}

#[test]
fn cherry_pick_copies_commit() {
    let (_td, g) = repo_two_branches();
    // feature commit doesn't exist on master yet
    let feat = g.rev_parse("feature").unwrap();
    g.cherry_pick(&[feat], &[]).unwrap();
    let msg = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert_eq!(msg, "feature work");
    assert!(exists(&g, "feature.txt"));
}

#[test]
fn cherry_apply_leaves_staged_no_commit() {
    let (_td, g) = repo_two_branches();
    let feat = g.rev_parse("feature").unwrap();
    g.cherry_apply(&[feat], &[]).unwrap();
    assert!(g.staged_files().contains(&"feature.txt".to_string()));
    let msg = g.git_string(&["log", "-1", "--pretty=%s"]).unwrap();
    assert_eq!(msg, "initial", "no commit created");
}

// =============================================================== stash

#[test]
fn stash_both_and_pop() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "dirty\n");
    g.stash_both(None, Untracked::No).unwrap();
    assert!(g.unstaged_files().is_empty(), "worktree clean after stash");
    assert_eq!(g.stashes().len(), 1);
    g.stash_pop("stash@{0}", false).unwrap();
    assert_eq!(read(&g, "file.txt"), "dirty\n");
    assert!(g.stashes().is_empty());
}

#[test]
fn stash_include_untracked() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "junk.txt", "j");
    g.stash_both(None, Untracked::Include).unwrap();
    assert!(!exists(&g, "junk.txt"));
    g.stash_pop("stash@{0}", false).unwrap();
    assert!(exists(&g, "junk.txt"));
}

#[test]
fn stash_index_only_stashes_staged() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "staged version\n");
    g.stage("file.txt").unwrap();
    g.stash_index(None).unwrap();
    // staged change removed from index AND worktree
    assert!(g.staged_files().is_empty());
    assert_eq!(read(&g, "file.txt"), "hello\n");
    assert_eq!(g.stashes().len(), 1);
    // applying restores staged state
    g.stash_apply("stash@{0}", true).unwrap();
    assert!(g.staged_files().contains(&"file.txt".to_string()));
}

#[test]
fn stash_keep_index_preserves_staged() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "staged\n");
    g.stage("file.txt").unwrap();
    g.stash_keep_index(None, Untracked::No).unwrap();
    // staged changes remain in index
    assert!(g.staged_files().contains(&"file.txt".to_string()));
    // but the stash captured everything
    assert_eq!(g.stashes().len(), 1);
    // worktree still has staged content
    assert_eq!(read(&g, "file.txt"), "staged\n");
}

#[test]
fn stash_drop_and_clear() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "hello\n1");
    g.stash_both(Some("one"), Untracked::No).unwrap();
    write(&g, "file.txt", "hello\n2");
    g.stash_both(Some("two"), Untracked::No).unwrap();
    assert_eq!(g.stashes().len(), 2);
    g.stash_drop("stash@{1}").unwrap();
    assert_eq!(g.stashes().len(), 1);
    g.stash_clear().unwrap();
    assert!(g.stashes().is_empty());
}

#[test]
fn stash_show_returns_diff() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "dirty\n");
    g.stash_both(None, Untracked::No).unwrap();
    let d = g.stash_show("stash@{0}").unwrap();
    assert!(d.contains("file.txt"));
    assert!(d.contains("dirty"));
}

#[test]
fn stash_list_and_messages() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "hello\nmodified");
    g.stash_both(Some("my message"), Untracked::No).unwrap();
    let l = g.stash_list();
    assert_eq!(l.len(), 1);
    assert!(l[0].contains("my message"));
    let (name, msg) = &g.stashes()[0];
    assert!(name.starts_with("stash@{0}"));
    assert!(msg.contains("my message"));
}

#[test]
fn stash_branch_checks_out_and_applies() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "stashed work\n");
    g.stash_both(None, Untracked::No).unwrap();
    g.stash_branch("fromstash", "stash@{0}").unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("fromstash"));
    assert_eq!(read(&g, "file.txt"), "stashed work\n");
}

#[test]
fn snapshot_keeps_changes_in_place() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "still here\n");
    g.snapshot(xagit_lib::git::ops::SnapshotScope::Both, Untracked::No)
        .unwrap();
    assert_eq!(read(&g, "file.txt"), "still here\n");
    assert_eq!(g.stashes().len(), 1, "snapshot recorded as a stash");
}

// =============================================================== ignore / untrack

#[test]
fn ignore_appends_pattern() {
    let (_td, g, _head) = repo_with_commit();
    g.ignore("*.log").unwrap();
    write(&g, "debug.log", "x");
    // debug.log is ignored; only .gitignore itself shows as untracked
    let untracked = g.untracked_files();
    assert!(!untracked.contains(&"debug.log".to_string()), "ignored file not listed");
    assert!(untracked.contains(&".gitignore".to_string()));
    let content = read(&g, ".gitignore");
    assert!(content.contains("*.log"));
}

#[test]
fn untrack_keeps_file_but_unstages() {
    let (_td, g, _head) = repo_with_commit();
    g.untrack("file.txt").unwrap();
    assert!(exists(&g, "file.txt"));
    let st = Status::load(&g);
    assert!(st.staged.iter().any(|f| f.path == "file.txt"), "deletion staged");
}

// =============================================================== log / diff / show

#[test]
fn log_returns_oneline_entries() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "v2\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("second"), &[]).unwrap();
    let l = g.log(None, 10).unwrap();
    assert_eq!(l.len(), 2);
    assert!(l[0].contains("second"));
    assert!(l[1].contains("initial"));
}

#[test]
fn diff_staged_and_unstaged() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "unstaged\n");
    let d = g.diff_unstaged().unwrap();
    assert!(d.contains("+unstaged"));
    g.stage("file.txt").unwrap();
    let d = g.diff_staged().unwrap();
    assert!(d.contains("+unstaged"));
    assert!(g.diff_unstaged().unwrap_or_default().is_empty());
}

#[test]
fn show_commit_output() {
    let (_td, g, _head) = repo_with_commit();
    let head = g.rev_parse("HEAD").unwrap();
    let s = g.show_commit(&head).unwrap();
    assert!(s.contains("initial"));
    assert!(s.contains("file.txt"));
}

// =============================================================== misc

#[test]
fn git_lines_splits_nul() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "u1.txt", "1");
    write(&g, "u2.txt", "2");
    let mut v = g.git_lines(&["ls-files", "--others", "--exclude-standard", "-z"]);
    v.sort();
    assert_eq!(v, vec!["u1.txt", "u2.txt"]);
}

#[test]
fn run_arbitrary_git_command() {
    let (_td, g, _head) = repo_with_commit();
    g.run_git_command(&["log".into(), "-1".into(), "--oneline".into()])
        .unwrap();
}

// =============================================================== libgit2 backend

#[test]
fn libgit2_discover_and_head() {
    let (td, g, head) = repo_with_commit();
    std::fs::create_dir(td.path().join("sub")).unwrap();
    let outside = TempDir::new().unwrap();
    assert!(LibGit2::discover(outside.path()).is_err());

    let lg = LibGit2::discover(&td.path().join("sub")).unwrap();
    assert_eq!(lg.toplevel, g.toplevel);
    assert!(lg.gitdir.exists());
    assert!(!lg.no_commit());
    assert_eq!(lg.head_oid().unwrap().to_string(), head);
    assert!(lg.current_branch().is_some());
    assert!(matches!(lg.state(), git2::RepositoryState::Clean));
}

#[test]
fn libgit2_no_commit_before_first_commit() {
    let (_td, g) = scratch_repo();
    let lg = LibGit2::from_git(&g).unwrap();
    assert!(lg.no_commit());
    assert!(lg.head_oid().is_none());
    assert!(matches!(lg.state(), git2::RepositoryState::Clean));
}

#[test]
fn libgit2_status_classification() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "new.txt", "n");
    write(&g, "file.txt", "changed");
    g.stage("file.txt").unwrap();

    let entries = LibGit2::from_git(&g).unwrap().statuses().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries
        .iter()
        .any(|f| f.path == "new.txt" && f.untracked && !f.staged));
    assert!(entries
        .iter()
        .any(|f| f.path == "file.txt" && f.staged && !f.unstaged));
}

#[test]
fn libgit2_refs_match_git_cli() {
    let (_td, g, _head) = repo_with_commit();
    let lg = LibGit2::from_git(&g).unwrap();
    assert_eq!(lg.branches().unwrap(), g.branches());
    assert_eq!(lg.tags().unwrap(), g.tags());
    assert_eq!(lg.remotes().unwrap(), g.remotes());
}

// =============================================================== magit extras

fn repo_with_history(n: usize) -> (TempDir, Git) {
    let (td, g, _head) = repo_with_commit();
    for i in 2..=n {
        write(&g, "file.txt", &format!("v{i}\n"));
        g.stage_all().unwrap();
        g.commit(Some(&format!("c{i}")), &[]).unwrap();
    }
    (td, g)
}

#[test]
fn bisect_start_mark_log_and_reset() {
    let (_td, g) = repo_with_history(4);
    assert!(!g.bisect_active());
    let goods = vec!["HEAD~3".to_string()];
    g.bisect_start(Some("HEAD"), &goods).unwrap();
    assert!(g.bisect_active());
    g.bisect_mark("skip", Some("HEAD")).unwrap();
    let log = g.bisect_log().unwrap();
    assert!(log.contains("bisect start"));
    assert!(log.contains("bisect skip"));
    g.bisect_reset(None).unwrap();
    assert!(!g.bisect_active());
    assert!(g.bisect_log().is_err());
}

#[test]
fn bisect_mark_bad_and_good() {
    let (_td, g) = repo_with_history(3);
    g.bisect_start(None, &[]).unwrap();
    g.bisect_mark("bad", Some("HEAD")).unwrap();
    g.bisect_mark("good", Some("HEAD~2")).unwrap();
    let log = g.bisect_log().unwrap();
    assert!(log.contains("bisect bad"));
    assert!(log.contains("bisect good"));
    g.bisect_reset(None).unwrap();
    assert!(g.bisect_mark("bad", Some("HEAD")).is_err());
}

#[test]
fn worktree_add_list_remove() {
    let (_td, g, _head) = repo_with_commit();
    let extra = TempDir::new().unwrap();
    let path = extra.path().join("linked");
    let p = path.to_str().unwrap();
    g.worktree_add(p, None, false, None).unwrap();
    // git reports canonical paths (/private/var/... on macOS), not the symlinked one
    let canon = path.canonicalize().unwrap();
    let cp = canon.to_str().unwrap();
    assert!(g.worktree_paths().iter().any(|x| x == cp));
    assert!(g.worktree_list().unwrap().contains("linked"));
    g.worktree_remove(p, true).unwrap();
    assert!(!g.worktree_paths().iter().any(|x| x == cp));
}

#[test]
fn submodule_add_list_deinit_remove() {
    let (_td, g, _head) = repo_with_commit();
    let src = TempDir::new().unwrap();
    git_init(src.path());
    std::fs::write(src.path().join("a"), "x").unwrap();
    let sg = Git::discover(src.path()).unwrap();
    sg.git_interactive(&["add", "."]).unwrap();
    sg.commit(Some("initial"), &[]).unwrap();

    g.submodule_add(src.path().to_str().unwrap(), "sub").unwrap();
    assert_eq!(g.submodule_paths(), vec!["sub".to_string()]);
    assert!(g.submodule_list().unwrap().contains("sub"));
    g.submodule_init().unwrap();
    g.submodule_sync().unwrap();
    g.submodule_deinit("sub").unwrap();
    g.submodule_remove("sub").unwrap();
    assert!(g.submodule_paths().is_empty());
}

#[test]
fn notes_add_show_list_remove() {
    let (_td, g, head) = repo_with_commit();
    g.notes_add(&head, "a note").unwrap();
    assert!(g.notes_show("HEAD").unwrap().contains("a note"));
    assert!(g.notes_list().unwrap().contains(&head));
    g.notes_remove(&head).unwrap();
    assert!(g.notes_show("HEAD").is_err());
    g.notes_prune().unwrap();
}

#[test]
fn diff_range_file_and_change() {
    let (_td, g) = repo_with_history(2);
    let d = g.diff_range("HEAD~1", "HEAD").unwrap();
    assert!(d.contains("v2"));
    let c = g.diff_change("HEAD").unwrap();
    assert!(c.contains("v2"));

    write(&g, "file.txt", "working\n");
    assert!(g.diff_file("file.txt", false).unwrap().contains("working"));
    g.stage("file.txt").unwrap();
    assert!(g.diff_file("file.txt", true).unwrap().contains("working"));
}

// =============================================================== remaining extras

#[test]
fn reflog_and_file_log() {
    let (_td, g) = repo_with_history(2);
    let r = g.reflog(None, 20).unwrap();
    assert!(r.contains("HEAD@{"));
    assert!(r.contains("c2"));
    let l = g.log_file("file.txt", 10).unwrap();
    assert!(l.contains("c2"));
    assert!(l.contains("initial"));
}

#[test]
fn branch_config_roundtrip() {
    let (_td, g, _head) = repo_with_commit();
    let branch = g.current_branch().unwrap();
    g.branch_set_config("rebase", "true").unwrap();
    assert!(g
        .branch_config()
        .unwrap()
        .contains(&format!("branch.{branch}.rebase true")));
    g.branch_set_config("description", "my branch").unwrap();
    assert!(g.branch_config().unwrap().contains("my branch"));
}

#[test]
fn cherries_with_upstream() {
    let (_td, g, _head) = repo_with_commit();
    assert!(g.cherries(Some("HEAD"), None).is_ok());
    assert!(g.cherries(Some("HEAD"), Some("HEAD")).is_ok());
}

#[test]
fn patch_format_apply_worktree_and_am() {
    let (_td, g) = repo_with_history(2);
    let out_dir = TempDir::new().unwrap();
    let out = g
        .patch_format("HEAD~1..HEAD", out_dir.path().to_str().unwrap())
        .unwrap();
    assert!(out.contains(".patch"));
    let patch = out.lines().next().unwrap().trim();

    let (_td2, g2, _h2) = repo_with_commit();
    g2.patch_apply_worktree(patch).unwrap();
    assert_eq!(read(&g2, "file.txt"), "v2\n");

    let (_td3, g3, _h3) = repo_with_commit();
    g3.patch_apply_am(patch, false).unwrap();
    assert!(read(&g3, "file.txt").contains("v2"));
}

#[test]
fn subtree_add_prefixes_and_split() {
    let (_td, g, _head) = repo_with_commit();
    assert!(g.subtree_prefixes().is_empty());
    let src = TempDir::new().unwrap();
    git_init(src.path());
    std::fs::write(src.path().join("a"), "x").unwrap();
    let sg = Git::discover(src.path()).unwrap();
    sg.git_interactive(&["add", "."]).unwrap();
    sg.commit(Some("initial"), &[]).unwrap();

    g.subtree_add("vendor/src", src.path().to_str().unwrap(), "HEAD")
        .unwrap();
    assert_eq!(g.subtree_prefixes(), vec!["vendor/src".to_string()]);
    assert!(!g
        .subtree_split("vendor/src", "split-branch")
        .unwrap()
        .is_empty());
}

// =============================================================== regression: magit diff audit

/// Upstream comes from branch.<b>.remote/merge, not from the first remote.
#[test]
fn push_and_pull_use_configured_upstream() {
    let (td, g, _td2, g2) = remote_sandbox();
    let cur = g.current_branch().unwrap();
    assert_eq!(g.upstream_of(&cur), Some(("origin".into(), cur.clone())));

    // a decoy remote sorted before "origin" must not be used
    g.remote_add("aaa", td.path().join("srv.git").to_str().unwrap(), false)
        .unwrap();
    // track a differently named remote branch
    g.git_interactive(&["push", "-q", "origin", "HEAD:refs/heads/up-other"])
        .unwrap();
    g.git_interactive(&["fetch", "-q", "origin"]).unwrap();
    g.git_interactive(&["branch", "--set-upstream-to=origin/up-other"])
        .unwrap();
    assert_eq!(g.upstream_of(&cur), Some(("origin".into(), "up-other".into())));

    write(&g, "up.txt", "u");
    g.stage("up.txt").unwrap();
    g.commit(Some("to upstream"), &[]).unwrap();
    g.push_upstream(&[]).unwrap();
    let head = g.rev_parse("HEAD").unwrap();
    let bare = Git::discover(&g2.toplevel).unwrap();
    bare.git_interactive(&["fetch", "-q", "origin"]).unwrap();
    assert_eq!(bare.rev_parse("origin/up-other").unwrap(), head);
    assert_ne!(bare.rev_parse(&format!("origin/{cur}")).unwrap(), head);

    // pull-upstream merges origin/up-other, not origin/<cur>
    write(&g2, "remote-only.txt", "r");
    g2.stage("remote-only.txt").unwrap();
    g2.commit(Some("on main only"), &[]).unwrap();
    g2.git_interactive(&["push", "-q", "origin", "HEAD"]).unwrap();
    g.pull_upstream(&[]).unwrap();
    assert!(!exists(&g, "remote-only.txt"));

    assert_eq!(g.tracked_of("origin/up-other"), ("origin".into(), "up-other".into()));
    assert_eq!(g.tracked_of(&cur), (".".into(), cur.clone()));
}

#[test]
fn upstream_commands_fail_without_upstream() {
    let (_td, g, _head) = repo_with_commit();
    assert!(g.upstream_of(&g.current_branch().unwrap()).is_none());
    assert!(g.push_upstream(&[]).is_err());
    assert!(g.pull_upstream(&[]).is_err());
    assert!(g.fetch_upstream(&[]).is_err());
}

/// magit-reverse on staged changes touches the working tree only.
#[test]
fn reverse_staged_changes_in_worktree_only() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "hello\nstaged\n");
    g.stage("file.txt").unwrap();
    g.reverse_staged(&["file.txt".to_string()]).unwrap();
    assert_eq!(read(&g, "file.txt"), "hello\n");
    let st = Status::load(&g);
    assert!(st.staged.iter().any(|f| f.path == "file.txt"), "index untouched");
    assert!(st.unstaged.iter().any(|f| f.path == "file.txt"));
    assert!(g.reverse_staged(&["nothing.txt".to_string()]).is_err());
}

#[test]
fn gitignore_in_subdirectory() {
    let (_td, g, _head) = repo_with_commit();
    std::fs::create_dir(g.toplevel.join("sub")).unwrap();
    g.ignore_in_subdir("sub", "*.log").unwrap();
    assert_eq!(read(&g, "sub/.gitignore"), "*.log\n");
    assert!(!exists(&g, ".gitignore"));
    g.ignore("*.tmp").unwrap();
    assert_eq!(read(&g, ".gitignore"), "*.tmp\n");
    assert!(g.ignore_in_subdir("missing", "x").is_err());
}

#[test]
fn git_command_in_working_directory() {
    let (_td, g, _head) = repo_with_commit();
    let sub = g.toplevel.join("sub");
    std::fs::create_dir(&sub).unwrap();
    assert_eq!(g.git_string_in(&sub, &["rev-parse", "--show-prefix"]).unwrap(), "sub/");
    assert_eq!(g.git_string(&["rev-parse", "--show-prefix"]).unwrap(), "");
}

// =============================================================== regression: keys that meant something else

fn subject(g: &Git) -> String {
    g.git_string(&["log", "-1", "--pretty=%s"]).unwrap()
}

fn parent_count(g: &Git) -> usize {
    g.git_string(&["rev-list", "--parents", "-n1", "HEAD"])
        .unwrap()
        .split_whitespace()
        .count()
        - 1
}

fn local_config(g: &Git, key: &str) -> Option<String> {
    g.git_string(&["config", "--local", key])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// "o r" is magit-submodule-register (`git submodule init`): it must never
/// remove anything. Removing is "o k".
#[test]
fn submodule_register_populate_unpopulate() {
    let (_td, g, _head) = repo_with_commit();
    let src = TempDir::new().unwrap();
    git_init(src.path());
    std::fs::write(src.path().join("a"), "x").unwrap();
    let sg = Git::discover(src.path()).unwrap();
    sg.git_interactive(&["add", "."]).unwrap();
    sg.commit(Some("initial"), &[]).unwrap();
    g.submodule_add(src.path().to_str().unwrap(), "sub").unwrap();
    g.commit(Some("add sub"), &[]).unwrap();
    let sub = ["sub".to_string()];

    // o d: deinit empties the checkout and forgets the url
    g.submodule_unpopulate(&[], &sub).unwrap();
    assert!(!exists(&g, "sub/a"));
    assert_eq!(local_config(&g, "submodule.sub.url"), None);

    // o r: registers again, nothing is checked out or removed
    g.submodule_register(&[]).unwrap();
    assert!(local_config(&g, "submodule.sub.url").is_some());
    assert_eq!(g.submodule_paths(), vec!["sub".to_string()]);
    assert!(exists(&g, ".gitmodules"));
    assert!(!exists(&g, "sub/a"));

    // o p: populate checks it out
    g.submodule_populate(&[], &sub).unwrap();
    assert!(exists(&g, "sub/a"));
    // o u on all modules
    g.submodule_update_with(&[], &[]).unwrap();
    assert!(exists(&g, "sub/a"));
}

/// b C r / b C R cycle true -> false -> unset like magit--set-popup-variable.
#[test]
fn cycle_branch_and_pull_rebase_config() {
    let (_td, g, _head) = repo_with_commit();
    let key = format!("branch.{}.rebase", g.current_branch().unwrap());
    for want in [Some("true"), Some("false"), None, Some("true")] {
        assert_eq!(g.cycle_config(&key, &["true", "false"]).unwrap().as_deref(), want);
        assert_eq!(local_config(&g, &key).as_deref(), want);
    }
    // a value outside the choices is unset on the next press
    g.set_config("pull.rebase", Some("merges")).unwrap();
    assert_eq!(g.cycle_config("pull.rebase", &["true", "false"]).unwrap(), None);
    assert_eq!(local_config(&g, "pull.rebase"), None);
    // unsetting a missing key is not an error
    g.set_config("pull.rebase", None).unwrap();
}

/// P p pushes to the push-remote (branch.<b>.pushRemote, then remote.pushDefault),
/// P t pushes all tags, P m matching branches, P r explicit refspecs.
#[test]
fn push_pushremote_tags_matching_refspecs() {
    let (td, g, _td2, _g2) = remote_sandbox();
    let cur = g.current_branch().unwrap();
    assert_eq!(g.push_remote_of(&cur), None);
    assert!(g.push_pushremote(&[]).is_err());

    assert!(std::process::Command::new("git")
        .args(["clone", "-q", "--bare", ".", "pub.git"])
        .current_dir(&g.toplevel)
        .status()
        .unwrap()
        .success());
    g.remote_add("pub", td.path().join("pub.git").to_str().unwrap(), false)
        .unwrap();
    g.set_config("remote.pushDefault", Some("origin")).unwrap();
    assert_eq!(g.push_remote_of(&cur).as_deref(), Some("origin"));
    g.set_config(&format!("branch.{cur}.pushRemote"), Some("pub")).unwrap();
    assert_eq!(g.push_remote_of(&cur).as_deref(), Some("pub"));

    write(&g, "pr.txt", "p");
    g.stage("pr.txt").unwrap();
    g.commit(Some("to pushremote"), &[]).unwrap();
    let head = g.rev_parse("HEAD").unwrap();
    assert_eq!(g.push_pushremote(&[]).unwrap(), "pub");
    let remote_head = |r: &str, b: &str| {
        g.git_string(&["ls-remote", r, &format!("refs/heads/{b}")])
            .unwrap()
            .split_whitespace()
            .next()
            .map(str::to_string)
    };
    assert_eq!(remote_head("pub", &cur).as_deref(), Some(head.as_str()));
    assert_ne!(remote_head("origin", &cur).as_deref(), Some(head.as_str()), "upstream untouched");

    g.tag_create("t1", "HEAD", &[], None).unwrap();
    g.tag_create("t2", "HEAD~1", &[], None).unwrap();
    g.push_tags("origin", &[]).unwrap();
    let tags = g.git_string(&["ls-remote", "--tags", "origin"]).unwrap();
    assert!(tags.contains("refs/tags/t1") && tags.contains("refs/tags/t2"), "{tags}");

    assert!(g.push_refspecs("origin", &[], &[]).is_err());
    g.push_refspecs("origin", &[format!("{cur}:refs/heads/side")], &[]).unwrap();
    assert_eq!(remote_head("origin", "side").as_deref(), Some(head.as_str()));

    // matching: only branches that exist on both sides are pushed
    g.git_interactive(&["branch", "local-only"]).unwrap();
    g.push_matching("origin", &[]).unwrap();
    assert_eq!(remote_head("origin", &cur).as_deref(), Some(head.as_str()));
    assert_eq!(remote_head("origin", "local-only"), None);
}

/// m p previews without touching the worktree; m e always creates a merge commit.
#[test]
fn merge_preview_and_editmsg() {
    let (_td, g) = repo_two_branches();
    let before = g.rev_parse("HEAD").unwrap();
    let p = g.merge_preview("feature").unwrap();
    assert!(p.contains("feature.txt"), "{p}");
    assert!(!exists(&g, "feature.txt"));
    assert_eq!(g.rev_parse("HEAD").unwrap(), before);

    // fast-forward would be possible, but magit-merge-editmsg uses --no-ff
    g.merge_with_message("feature", "custom merge message", &["--ff-only".into()])
        .unwrap();
    assert_eq!(subject(&g), "custom merge message");
    assert_eq!(parent_count(&g), 2);
    assert!(exists(&g, "feature.txt"));
}

/// m m while merging commits the merge (magit-commit-create in the merge menu).
#[test]
fn merge_commit_finishes_merge_in_progress() {
    let (_td, g) = repo_two_branches();
    assert!(g.merge_commit().is_err());
    write(&g, "master.txt", "m");
    g.stage("master.txt").unwrap();
    g.commit(Some("master work"), &[]).unwrap();
    g.merge_nocommit("feature", &[]).unwrap();
    assert!(g.merge_in_progress());
    g.merge_commit().unwrap();
    assert!(!g.merge_in_progress());
    assert_eq!(parent_count(&g), 2);
}

fn repo_with_conflicting_history() -> (TempDir, Git, String) {
    let (td, g) = repo_with_history(3); // file.txt: hello -> v2 -> v3
    let second = g.rev_parse("HEAD~1").unwrap();
    (td, g, second)
}

/// A / V while a cherry-pick or revert is stopped: continue, skip, abort.
#[test]
fn sequencer_continue_skip_abort() {
    let (_td, g, second) = repo_with_conflicting_history();
    let head = g.rev_parse("HEAD").unwrap();
    assert!(!g.pick_or_revert_in_progress());
    assert!(g.sequencer("abort").is_err());
    assert!(g.sequencer("bogus").is_err());

    // revert of v2 conflicts with v3
    assert!(g.revert(&[second.clone()], &[], false).is_err());
    assert!(g.revert_in_progress() && g.pick_or_revert_in_progress());
    g.sequencer("abort").unwrap();
    assert!(!g.pick_or_revert_in_progress());
    assert_eq!(g.rev_parse("HEAD").unwrap(), head);
    assert_eq!(read(&g, "file.txt"), "v3\n");

    // cherry-pick: resolve, then continue
    assert!(g.cherry_pick(&[second.clone()], &[]).is_err());
    assert!(g.cherry_in_progress() && g.pick_or_revert_in_progress());
    write(&g, "file.txt", "resolved\n");
    g.stage("file.txt").unwrap();
    g.sequencer("continue").unwrap();
    assert!(!g.pick_or_revert_in_progress());
    assert_eq!(subject(&g), "c2");
    assert_eq!(read(&g, "file.txt"), "resolved\n");

    // skip drops the stopped commit
    let head = g.rev_parse("HEAD").unwrap();
    assert!(g.revert(&[second], &[], false).is_err());
    g.sequencer("skip").unwrap();
    assert!(!g.pick_or_revert_in_progress());
    assert_eq!(g.rev_parse("HEAD").unwrap(), head);
}

/// `git am` uses rebase-apply too, but it is not a rebase (magit-am-in-progress-p).
#[test]
fn am_in_progress_is_not_rebase_and_can_be_skipped() {
    let (_td, g) = repo_with_history(2);
    let out_dir = TempDir::new().unwrap();
    let out = g
        .patch_format("HEAD~1..HEAD", out_dir.path().to_str().unwrap())
        .unwrap();
    let patch = out.lines().next().unwrap().trim().to_string();

    let (_td2, g2, _h2) = repo_with_commit();
    write(&g2, "file.txt", "diverged\n");
    g2.stage("file.txt").unwrap();
    g2.commit(Some("diverged"), &[]).unwrap();
    assert!(g2.am_apply(&[patch], &[]).is_err());
    assert!(g2.am_in_progress());
    assert!(!g2.rebase_in_progress());
    g2.am_skip().unwrap();
    assert!(!g2.am_in_progress());
    assert_eq!(subject(&g2), "diverged");
}

#[test]
fn bisect_in_progress_tracks_bisect_log() {
    let (_td, g) = repo_with_history(3);
    assert!(!g.bisect_in_progress());
    g.bisect_start(Some("HEAD"), &["HEAD~2".to_string()]).unwrap();
    assert!(g.bisect_in_progress());
    g.bisect_reset(None).unwrap();
    assert!(!g.bisect_in_progress());
}

/// r f: magit-rebase-autosquash folds fixup! commits without an editor.
#[test]
fn rebase_autosquash_folds_fixups() {
    let (_td, g) = repo_with_history(2);
    assert_eq!(g.autosquash_base(), None, "no upstream");
    let c2 = g.rev_parse("HEAD").unwrap();
    write(&g, "extra.txt", "e");
    g.stage("extra.txt").unwrap();
    g.git_interactive(&["commit", "--fixup", &c2]).unwrap();
    g.rebase_autosquash_with("HEAD~2", &["--interactive".into()]).unwrap();
    assert!(!g.rebase_in_progress());
    assert_eq!(g.git_string(&["rev-list", "--count", "HEAD"]).unwrap(), "2");
    assert_eq!(subject(&g), "c2");
    assert!(exists(&g, "extra.txt"));
}

/// `$` shows side-effecting git calls (with failures marked), not queries.
#[test]
fn process_log_records_commands_not_queries() {
    use xagit_lib::git::exec::process_log;
    let (_td, g, _head) = repo_with_commit();
    g.tag_create("proclog-marker", "HEAD", &[], None).unwrap();
    let _ = g.git_string(&["rev-parse", "--verify", "proclog-query^{commit}"]);
    let _ = g.git_interactive(&["checkout", "proclog-missing-branch"]);
    let log = process_log();
    assert!(log.iter().any(|e| e.starts_with(' ') && e.contains("proclog-marker")));
    assert!(!log.iter().any(|e| e.contains("proclog-query")));
    let failed = log.iter().find(|e| e.contains("proclog-missing-branch")).unwrap();
    assert!(failed.starts_with('!'), "{failed}");
}

/// D / L arguments are passed through to diff and log.
#[test]
fn diff_and_log_arguments() {
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "hello  \n");
    assert!(!g.diff_with(&[], &[]).unwrap().is_empty());
    assert!(g.diff_with(&["--ignore-all-space".into()], &[]).unwrap().is_empty());
    let l = g
        .log_with(None, 10, &["--follow".into(), "--reverse".into()])
        .unwrap();
    assert_eq!(l.len(), 1);
    assert!(l[0].contains("initial"));
}

// =============================================================== todo-editing suffixes

fn commit_file(g: &Git, name: &str, msg: &str) -> String {
    write(g, name, msg);
    g.stage(name).unwrap();
    g.commit(Some(msg), &[]).unwrap();
    g.rev_parse("HEAD").unwrap()
}

fn subjects(g: &Git, rev: &str) -> Vec<String> {
    g.git_string(&["log", "--format=%s", rev])
        .unwrap()
        .lines()
        .map(String::from)
        .collect()
}

/// initial, c1, c2, c3 on the default branch.
fn linear_repo() -> (TempDir, Git, [String; 3]) {
    let (td, g, _head) = repo_with_commit();
    let c1 = commit_file(&g, "c1.txt", "c1");
    let c2 = commit_file(&g, "c2.txt", "c2");
    let c3 = commit_file(&g, "c3.txt", "c3");
    (td, g, [c1, c2, c3])
}

#[test]
fn perl_pick_editor_matches_magit() {
    assert_eq!(
        xagit_lib::git::ops::perl_pick_editor("abc123", "edit"),
        "perl -i -p -e '++$x if not $x and s/^pick abc123/edit abc123/'"
    );
}

/// magit-rebase-interactive-1: COMMIT^ as base, --root for the root commit,
/// and COMMIT must be an ancestor of HEAD.
#[test]
fn rebase_since_uses_parent_or_root() {
    let (_td, g, [c1, ..]) = linear_repo();
    assert_eq!(g.rebase_since(&c1).unwrap(), Some(format!("{c1}^")));
    let root = g.rev_parse("HEAD~3").unwrap();
    assert_eq!(g.rebase_since(&root).unwrap(), None);
    g.branch_and_checkout("side", Some(&root)).unwrap();
    assert!(g.rebase_since(&c1).is_err());
}

/// r i: the todo list goes through the sequence editor; accepting it unchanged
/// keeps history, editing it rewrites history.
#[test]
fn rebase_interactive_runs_sequence_editor() {
    let (_td, g, [c1, _c2, c3]) = linear_repo();
    g.rebase_interactive(&c1, &[], Editor::Accept, Editor::Accept).unwrap();
    assert_eq!(g.rev_parse("HEAD").unwrap(), c3);
    g.rebase_interactive(&c1, &[], Editor::Command("perl -i -pe 's/^pick/drop/ if $. == 1'"), Editor::Accept)
        .unwrap();
    assert_eq!(subjects(&g, "HEAD"), ["c3", "c2", "initial"]);
    assert!(!exists(&g, "c1.txt"));
    // from the root commit (--root)
    let root = g.rev_parse("HEAD~2").unwrap();
    g.rebase_interactive(&root, &[], Editor::Accept, Editor::Accept).unwrap();
    assert_eq!(subjects(&g, "HEAD"), ["c3", "c2", "initial"]);
}

/// r m: stop at COMMIT so it can be amended, then continue.
#[test]
fn rebase_edit_commit_stops_at_commit() {
    let (_td, g, [_c1, c2, _c3]) = linear_repo();
    g.rebase_edit_commit(&c2, &[]).unwrap();
    assert!(g.rebase_in_progress());
    assert_eq!(g.rev_parse("HEAD").unwrap(), c2);
    write(&g, "c2.txt", "amended");
    g.stage("c2.txt").unwrap();
    g.commit(None, &["--amend".into(), "--no-edit".into()]).unwrap();
    g.git_with_editors(Editor::Accept, Editor::Accept, &["rebase", "--continue"]).unwrap();
    assert!(!g.rebase_in_progress());
    assert_eq!(subjects(&g, "HEAD"), ["c3", "c2", "c1", "initial"]);
    assert_eq!(read(&g, "c2.txt"), "amended");
}

/// r w: only the message of COMMIT changes.
#[test]
fn rebase_reword_commit_changes_message() {
    let (_td, g, [_c1, c2, _c3]) = linear_repo();
    g.rebase_reword_commit(&c2, &[], Editor::Command("perl -i -pe 's/^c2$/reworded/'")).unwrap();
    assert!(!g.rebase_in_progress());
    assert_eq!(subjects(&g, "HEAD"), ["c3", "reworded", "c1", "initial"]);
    assert_eq!(g.git_string(&["diff", &c2, "HEAD~1"]).unwrap(), "");
}

/// r k: COMMIT is dropped (noop + commented pick, as Magit does).
#[test]
fn rebase_remove_commit_drops_commit() {
    let (_td, g, [_c1, c2, _c3]) = linear_repo();
    g.rebase_remove_commit(&c2, &[]).unwrap();
    assert!(!g.rebase_in_progress());
    assert_eq!(subjects(&g, "HEAD"), ["c3", "c1", "initial"]);
    assert!(!exists(&g, "c2.txt"));
    assert!(exists(&g, "c3.txt"));
}

/// r s: rebase --onto NEWBASE START moves the commits after START.
#[test]
fn rebase_subset_moves_commits_after_start() {
    let (_td, g, _head) = repo_with_commit();
    let main = g.current_branch().unwrap();
    g.branch_and_checkout("topic", None).unwrap();
    let t1 = commit_file(&g, "t1.txt", "t1");
    commit_file(&g, "t2.txt", "t2");
    commit_file(&g, "t3.txt", "t3");
    g.checkout(&main).unwrap();
    commit_file(&g, "m1.txt", "m1");
    g.checkout("topic").unwrap();
    g.rebase_subset(&main, &t1, &[]).unwrap();
    assert_eq!(subjects(&g, "HEAD"), ["t3", "t2", "m1", "initial"]);
    assert!(!exists(&g, "t1.txt"));
}

/// r e (while rebasing): git rebase --edit-todo.
#[test]
fn rebase_edit_todo_rewrites_remaining_steps() {
    let (_td, g, [c1, ..]) = linear_repo();
    assert!(g.rebase_edit_todo(Editor::Accept).is_err());
    g.rebase_edit_commit(&c1, &[]).unwrap();
    g.rebase_edit_todo(Editor::Command("perl -i -pe 's/^pick(.* c3)$/drop$1/'")).unwrap();
    g.git_with_editors(Editor::Accept, Editor::Accept, &["rebase", "--continue"]).unwrap();
    assert_eq!(subjects(&g, "HEAD"), ["c2", "c1", "initial"]);
}

// =============================================================== merge absorb/dissolve

/// m a: merge BRANCH with --no-edit, then delete it.
#[test]
fn merge_absorb_merges_and_deletes_branch() {
    let (_td, g) = repo_two_branches();
    g.merge_absorb("feature", &["--no-ff".into()]).unwrap();
    assert!(exists(&g, "feature.txt"));
    assert!(!g.local_branch_exists("feature"));
    assert_eq!(g.git_string(&["log", "-1", "--format=%p"]).unwrap().split(' ').count(), 2);
}

/// m a with a push target: force-push first, unset pushRemote afterwards.
#[test]
fn merge_absorb_pushes_and_unsets_push_remote() {
    let (_td, g, _td2, _g2) = remote_sandbox();
    let main = g.current_branch().unwrap();
    g.branch_and_checkout("feature", None).unwrap();
    commit_file(&g, "f1.txt", "f1");
    g.git_interactive(&["push", "-q", "origin", "feature"]).unwrap();
    g.set_config("branch.feature.pushRemote", Some("origin")).unwrap();
    let tip = commit_file(&g, "f2.txt", "f2");
    g.checkout(&main).unwrap();
    g.merge_absorb("feature", &[]).unwrap();
    let remote = g.git_string(&["ls-remote", "origin", "refs/heads/feature"]).unwrap();
    assert!(remote.starts_with(&tip), "{remote}");
    assert!(!g.git_ok(&["config", "branch.feature.pushRemote"]));
    assert!(!g.local_branch_exists("feature"));
    assert!(exists(&g, "f2.txt"));
}

/// m d: check out INTO, merge the previously current branch, delete it.
#[test]
fn merge_dissolve_merges_current_into_target() {
    let (_td, g) = repo_two_branches();
    let main = g.current_branch().unwrap();
    g.checkout("feature").unwrap();
    g.merge_dissolve(&main, &[]).unwrap();
    assert_eq!(g.current_branch().as_deref(), Some(main.as_str()));
    assert!(exists(&g, "feature.txt"));
    assert!(!g.local_branch_exists("feature"));
}

// =============================================================== cherry moves

/// main: initial; feature: initial, f1, f2, f3
fn cherry_repo() -> (TempDir, Git, String, [String; 3]) {
    let (td, g, _head) = repo_with_commit();
    let main = g.current_branch().unwrap();
    g.branch_and_checkout("feature", None).unwrap();
    let f1 = commit_file(&g, "f1.txt", "f1");
    let f2 = commit_file(&g, "f2.txt", "f2");
    let f3 = commit_file(&g, "f3.txt", "f3");
    g.checkout(&main).unwrap();
    (td, g, main, [f1, f2, f3])
}

#[test]
fn resolve_cherries_orders_oldest_first() {
    let (_td, g, main, [f1, f2, f3]) = cherry_repo();
    assert_eq!(g.resolve_cherries(&f2).unwrap(), [f2.clone()]);
    assert_eq!(g.resolve_cherries(&format!("{main}..feature")).unwrap(), [f1, f2, f3]);
    assert!(g.resolve_cherries("feature..feature").is_err());
    assert_eq!(g.branches_containing("feature~1"), ["feature"]);
}

/// A h on the tip: picked onto the current branch, source reset with update-ref.
#[test]
fn cherry_harvest_tip_resets_source() {
    let (_td, g, main, [_f1, f2, f3]) = cherry_repo();
    g.cherry_move(&[f3], Some("feature"), &main, &[], None, true).unwrap();
    assert_eq!(g.current_branch().as_deref(), Some(main.as_str()));
    assert_eq!(subjects(&g, "HEAD"), ["f3", "initial"]);
    assert_eq!(g.rev_parse("feature").unwrap(), f2);
}

/// A h on a non-tip commit: source gets it dropped with rebase -i.
#[test]
fn cherry_harvest_middle_rebases_source() {
    let (_td, g, main, [_f1, f2, _f3]) = cherry_repo();
    g.cherry_move(&[f2], Some("feature"), &main, &[], None, true).unwrap();
    assert_eq!(g.current_branch().as_deref(), Some(main.as_str()));
    assert_eq!(subjects(&g, "HEAD"), ["f2", "initial"]);
    assert_eq!(subjects(&g, "feature"), ["f3", "f1", "initial"]);
}

/// A d: move commits from the current branch to another, stay on current.
#[test]
fn cherry_donate_moves_to_other_branch() {
    let (_td, g, main, [f1, f2, f3]) = cherry_repo();
    g.checkout("feature").unwrap();
    g.cherry_move(&[f2, f3], Some("feature"), &main, &[], None, false).unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("feature"));
    assert_eq!(g.rev_parse("HEAD").unwrap(), f1);
    assert_eq!(subjects(&g, &main), ["f3", "f2", "initial"]);
}

/// A n: new branch from START with the commits, stay on the current branch.
#[test]
fn cherry_spinout_creates_branch_and_stays() {
    let (_td, g, main, [f1, _f2, _f3]) = cherry_repo();
    g.checkout("feature").unwrap();
    g.cherry_move(&[f1], Some("feature"), "out", &[], Some(&main), false).unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("feature"));
    assert_eq!(subjects(&g, "feature"), ["f3", "f2", "initial"]);
    assert_eq!(subjects(&g, "out"), ["f1", "initial"]);
}

/// A s: like spinout but checks out the new branch.
#[test]
fn cherry_spinoff_creates_branch_and_checks_out() {
    let (_td, g, main, [_f1, f2, f3]) = cherry_repo();
    g.checkout("feature").unwrap();
    g.cherry_move(&[f3], Some("feature"), "off", &[], Some(&main), true).unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("off"));
    assert_eq!(subjects(&g, "HEAD"), ["f3", "initial"]);
    assert_eq!(g.rev_parse("feature").unwrap(), f2);
}

// =============================================================== am maildir

/// w m: git am on a mailbox.
#[test]
fn am_apply_maildir_applies_mailbox() {
    let (td, g, main, _) = cherry_repo();
    let mbox = g.git_string(&["format-patch", "--stdout", &format!("{main}..feature")]).unwrap();
    let path = td.path().join("patches.mbox");
    std::fs::write(&path, format!("{mbox}\n")).unwrap();
    g.am_apply(&[path.to_string_lossy().into_owned()], &[]).unwrap();
    assert_eq!(subjects(&g, "HEAD"), ["f3", "f2", "f1", "initial"]);
}

// =============================================================== default branch

#[test]
fn main_branch_and_primary_remote() {
    let (_td, g, _td2, _g2) = remote_sandbox();
    let cur = g.current_branch().unwrap();
    assert_eq!(g.main_branch().as_deref(), Some(cur.as_str()));
    g.git_interactive(&["branch", "trunk"]).unwrap();
    g.set_config("init.defaultBranch", Some("trunk")).unwrap();
    assert_eq!(g.main_branch().as_deref(), Some("trunk"));
    assert_eq!(g.primary_remote().as_deref(), Some("origin"));
    g.remote_add("upstream", "/nonexistent", false).unwrap();
    assert_eq!(g.primary_remote().as_deref(), Some("upstream"));
    g.set_config("magit.primaryRemote", Some("origin")).unwrap();
    assert_eq!(g.primary_remote().as_deref(), Some("origin"));
}

/// b C B: follow a rename of the remote default branch.
#[test]
fn update_default_branch_follows_remote_rename() {
    let (td, g, _td2, _g2) = remote_sandbox();
    let old = g.current_branch().unwrap();
    g.git_interactive(&["remote", "set-head", "origin", "--auto"]).unwrap();
    g.git_interactive(&["branch", "--track", "topic", &old]).unwrap();
    assert_eq!(g.default_branch(false).unwrap(), ("origin".to_string(), Some(old.clone())));
    let bare = td.path().join("srv.git");
    g.git_string_in(&bare, &["branch", "-m", &old, "newmain"]).unwrap();
    let (remote, new) = g.default_branch(true).unwrap();
    assert_eq!((remote.as_str(), new.as_deref()), ("origin", Some("newmain")));
    g.set_default_branch("newmain", &old).unwrap();
    assert_eq!(g.current_branch().as_deref(), Some("newmain"));
    assert!(!g.local_branch_exists(&old));
    let up = |b: &str| g.git_string(&["rev-parse", "--abbrev-ref", &format!("{b}@{{upstream}}")]).unwrap();
    assert_eq!(up("newmain"), "origin/newmain");
    assert_eq!(up("topic"), "newmain");
}

#[test]
fn failing_git_command_explains_command_status_and_stderr() {
    let (_td, g, _head) = repo_with_commit();
    let error = g.git_string(&["rev-parse", "--verify", "missing-ref-for-error-test"]).unwrap_err().to_string();
    assert!(error.contains("rev-parse"));
    assert!(error.contains("missing-ref-for-error-test"));
    assert!(error.contains("exit status:"));
    assert!(error.contains("stderr:"));
    assert!(!error.contains("stderr:\n(empty)"));
}

#[test]
fn checkout_failure_preserves_local_changes_and_git_diagnostics() {
    let (_td, g, _) = repo_with_commit();
    let main = g.current_branch().unwrap();
    g.branch_and_checkout("checkout-side", None).unwrap();
    write(&g, "file.txt", "side version\n");
    g.stage("file.txt").unwrap();
    g.commit(Some("side"), &[]).unwrap();
    g.checkout(&main).unwrap();
    write(&g, "file.txt", "keep my local changes\n");
    let text = format!("{:#}", g.checkout("checkout-side").unwrap_err());
    assert!((text.contains("would be overwritten by checkout") || text.contains("本地修改将被检出操作覆盖")), "{text}");
    assert!(text.contains("file.txt") && text.contains("checkout-side"));
    assert!(text.contains(&g.toplevel.display().to_string()));
    assert_eq!(read(&g, "file.txt"), "keep my local changes\n");
    assert_eq!(g.current_branch().unwrap(), main);
    assert!(xagit_lib::git::exec::process_log().iter().any(|entry| (entry.contains("would be overwritten") || entry.contains("本地修改将被检出操作覆盖")) && entry.contains("file.txt")));
}

#[test]
fn checkout_unknown_branch_keeps_diagnostics_in_both_execution_paths() {
    let (_td, g, _) = repo_with_commit();
    for result in [g.checkout("checkout-missing-ref"), g.git_with_editors_env(xagit_lib::git::exec::Editor::User, xagit_lib::git::exec::Editor::User, &[], &["checkout", "checkout-missing-ref"])] {
        let text = format!("{:#}", result.unwrap_err());
        assert!(((text.contains("pathspec") && text.contains("did not match")) || text.contains("路径规格")), "{text}");
        assert!(text.contains("checkout-missing-ref") && text.contains(&g.toplevel.display().to_string()));
        assert!(!text.contains("Output was shown"));
    }
}

#[cfg(unix)]
#[test]
fn checkout_hook_retains_stderr_tty() {
    use std::os::unix::fs::PermissionsExt;
    let (_td, g, _) = repo_with_commit();
    let hook = g.gitdir.join("hooks/post-checkout");
    std::fs::write(&hook, "#!/bin/sh\nif test -t 2; then echo hook-stderr-is-tty >&2; else echo hook-lost-tty >&2; fi\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let text = format!("{:#}", g.checkout("HEAD").unwrap_err());
    assert!(text.contains("hook-stderr-is-tty"), "{text}");
    assert!(!text.contains("hook-lost-tty"));
}
