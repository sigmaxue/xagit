//! tests/common/mod.rs — shared test fixtures: scratch repos mirroring Magit test setup.
//!
//! Every integration test crate compiles this module and uses only part of
//! it, so unused helpers are expected here (and only here).
#![allow(dead_code)]

use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;
use std::fs;

/// Press a sequence of keys, panicking on any error.
pub fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

/// The name a few older test files use for `press`.
pub fn press_all(app: &mut App, keys: &[&str]) {
    press(app, keys);
}

/// Create an empty scratch repo (like magit's `setup` with no commits).
pub fn scratch_repo() -> (TempDir, Git) {
    let td = TempDir::new().unwrap();
    git_init(td.path());
    let g = Git::discover(td.path()).expect("tempdir should be a repo");
    // commands that run the user's editor (magit-run-git-with-editor)
    // accept the message git prepared
    g.git_interactive(&["config", "core.editor", "true"]).unwrap();
    (td, g)
}

/// git init at path (Git::discover requires an existing repo). The branch
/// is pinned: otherwise `init.defaultBranch` from the system/global config
/// decides it and the tests that expect `master` break on newer setups.
pub fn git_init(path: &std::path::Path) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "master"])
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    );
}

/// Create a scratch repo with one initial commit containing file.txt.
pub fn repo_with_commit() -> (TempDir, Git, String) {
    let (td, g) = scratch_repo();
    fs::write(td.path().join("file.txt"), "hello\n").unwrap();
    g.git_interactive(&["add", "."]).unwrap();
    g.commit(Some("initial"), &[]).unwrap();
    let head = g.rev_parse("HEAD").unwrap();
    (td, g, head)
}

pub fn write(g: &Git, path: &str, content: &str) {
    fs::write(g.toplevel.join(path), content).unwrap();
}

pub fn read(g: &Git, path: &str) -> String {
    fs::read_to_string(g.toplevel.join(path)).unwrap()
}

pub fn exists(g: &Git, path: &str) -> bool {
    g.toplevel.join(path).exists()
}
