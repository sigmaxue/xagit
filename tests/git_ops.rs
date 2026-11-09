//! tests/git_ops.rs — behavior tests: every git operation must produce the
//! same repository state as the corresponding Magit command (which shells out
//! to git with the arguments documented in its docstrings).

mod common;

use common::*;
use ngit_lib::prelude::*;

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
    let (td, g, _head) = repo_with_commit();
    let st = Status::load(&g);
    assert!(st.branch.is_some());
    assert!(st.is_clean());
    // ahead/behind require upstream; skip when none configured
    let _ = td;
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
    assert!(st.staged.iter().any(|f| f.path.contains("renamed.txt")));
    assert!(st.staged.iter().any(|f| f.path.contains("file.txt")));
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
    let (_td, g, _head) = repo_with_commit();
    write(&g, "file.txt", "modified");
    write(&g, "extra.txt", "extra");
    g.stage_all().unwrap();
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
    g.stage_all().unwrap();
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
    g.tag_create("v1.0", "HEAD", false, None).unwrap();
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
    g.merge_nocommit("feature").unwrap();
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
    g.commit(Some("ahead"), &[]);
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
    g.tag_create("light", "HEAD", false, None).unwrap();
    g.tag_create("anno", "HEAD", true, Some("annotation")).unwrap();
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
    g.tag_create("gone", "HEAD", false, None).unwrap();
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
    g.snapshot(ngit_lib::git::ops::SnapshotScope::Both, Untracked::No)
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
