//! `xagit --help` answers on stdout before the TUI is started.

use std::process::Command;

fn xagit() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xagit"))
}

#[test]
fn help_prints_usage_and_succeeds() {
    let out = xagit().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Usage: xagit [PATH]"), "{text}");
    assert!(text.contains("xagit --help"), "{text}");
    assert!(text.contains("xagit --version"), "{text}");
    assert!(text.contains("C-x C-c"), "{text}");
    assert!(text.contains("XAGIT_COLOR"), "{text}");
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn help_does_not_need_a_repository() {
    let out = xagit()
        .arg("--help")
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Usage: xagit [PATH]"));
}

#[test]
fn help_is_recognized_after_the_path() {
    let out = xagit()
        .args(["/definitely/not/a/repository", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Usage: xagit [PATH]"));
}

#[test]
fn a_path_is_not_mistaken_for_help() {
    // A path alone starts the TUI (no Usage: output).  Here there is no
    // terminal, so it exits, but not because the path is not a repository.
    let out = xagit().arg("/definitely/not/a/repository").output().unwrap();
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("Usage:"));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("xagit: "), "{err}");
    assert!(!err.contains("rev-parse"), "{err}");
    assert!(!err.to_ascii_lowercase().contains("not a git repository"), "{err}");
}

#[test]
fn starting_outside_a_repository_shows_the_hint() {
    let td = tempfile::tempdir().unwrap();
    let out = xagit().current_dir(td.path()).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("rev-parse"), "{err}");
    assert!(!err.to_ascii_lowercase().contains("not a git repository"), "{err}");
    // the TUI draws its first frame before a terminal-less input loop fails
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Outside a Git repository"), "{text}");
    assert!(!text.contains("Usage:"), "{text}");
}

#[test]
fn version_prints_the_package_version() {
    let out = xagit().arg("--version").output().unwrap();
    assert!(out.status.success());
    // Cargo.toml declares 1.0.0; main.rs derives VERSION from it.
    assert_eq!(String::from_utf8_lossy(&out.stdout), "v1.0.0\n");
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn version_needs_no_repository_and_works_after_a_path() {
    let out = xagit()
        .args(["/definitely/not/a/repository", "--version"])
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "v1.0.0\n");
}

#[test]
fn help_wins_when_both_flags_are_given() {
    let out = xagit().args(["--version", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("xagit - a Magit-style git TUI"), "{text}");
    assert!(!text.contains("v1.0.0"), "{text}");
}
