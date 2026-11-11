//! magit-run (!), magit-ediff (E) and magit-git-mergetool: shell commands,
//! graphical launchers, inapt suffixes and the mergetool transient.

mod common;

use common::*;
use std::fs;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;

fn popup(app: &App) -> String {
    app.transient_popup().join("\n")
}

/// Never write to the user's ~/.gitconfig: the global git variables of
/// magit-git-mergetool go to a scratch config instead.
fn isolate() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("xagit-run-config-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gitconfig");
        fs::write(&file, "[user]\n\tname = t\n\temail = t@t\n").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &file);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    });
}

/// f.txt conflicts between master and other (like tests/smerge.rs).
fn conflicted() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    let git = |args: &[&str]| g.git_interactive(args).unwrap();
    write(&g, "f.txt", "a\nb\nc\nd\ne\nf\ng\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "f"]);
    git(&["checkout", "-q", "-b", "other"]);
    write(&g, "f.txt", "a\nb\nc\ntheirs\ne\nf\ng\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "theirs"]);
    git(&["checkout", "-q", "master"]);
    write(&g, "f.txt", "a\nb\nc\nours\ne\nf\ng\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "ours"]);
    assert!(g.git_interactive(&["merge", "-q", "other"]).is_err());
    (td, g)
}

#[test]
fn shell_command_output_and_process_log() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g);
    app.script_answers(&["echo xagit-run"]);
    press(&mut app, &["!", "S"]);
    assert!(app.prompts().iter().any(|p| p.starts_with("Async shell command in ")), "{:?}", app.prompts());
    let (title, text) = app.text_view().expect("command output shown");
    assert!(title.starts_with("Async shell command in "), "{title}");
    assert!(text.starts_with("$ echo xagit-run\n"), "{text}");
    assert!(text.contains("xagit-run\n"), "{text}");
    // magit-process-buffer: the command is recorded, queries are not
    press(&mut app, &["q", "$"]);
    let (title, text) = app.text_view().expect("process log shown");
    assert_eq!(title, "git process log");
    assert!(text.contains("echo xagit-run"), "{text}");
}

#[test]
fn git_command_initial_input_and_topdir() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "n.txt", "n\n");
    let mut app = App::new(g);
    // "git " is the initial input; typing the rest and RET runs it in
    // the repository root
    app.script_keys(&["s", "t", "a", "t", "u", "s", " ", "-", "-", "s", "h", "o", "r", "t", "RET"]);
    press(&mut app, &["!", "!"]);
    let (_, text) = app.text_view().expect("git output shown");
    assert!(text.starts_with("$ git status --short\n"), "{text}");
    assert!(text.contains("?? n.txt"), "{text}");
}

#[test]
fn shell_command_respects_working_directory() {
    let (td, g, _) = repo_with_commit();
    fs::create_dir(td.path().join("sub")).unwrap();
    let mut app = App::new(g);
    app.cwd = td.path().join("sub");

    app.script_answers(&["pwd"]);
    press(&mut app, &["!", "S"]);
    let out = app.text_view().unwrap().1.to_string();
    assert!(out.trim_end().ends_with("sub"), "{out}");
    press(&mut app, &["q"]);

    app.script_answers(&["pwd"]);
    press(&mut app, &["!", "s"]);
    let out = app.text_view().unwrap().1.to_string();
    assert!(!out.trim_end().ends_with("sub"), "{out}");
}

#[test]
fn ediff_inapt_suffixes() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    press(&mut app, &["E"]);
    let p = popup(&app);
    assert!(p.contains("(s Stage)") && p.contains("(m Resolve rest)") && p.contains("(M Resolve all conflicts)"), "{p}");
    app.press("m").unwrap();
    assert_eq!(
        app.message(),
        "Inapt command: `m' (Use `C-g' to abort, `?' for help) [magit-ediff-resolve-rest]"
    );
    assert_eq!(app.current_menu(), Some("ediff-menu"));
    press(&mut app, &["C-g"]);

    // modified files: Stage becomes apt, the conflict suffixes do not
    write(&g, "file.txt", "changed\n");
    press(&mut app, &["E"]);
    let p = popup(&app);
    assert!(p.contains("s Stage") && !p.contains("(s Stage)"), "{p}");
    assert!(p.contains("(m Resolve rest)"), "{p}");
    press(&mut app, &["C-g"]);
}

#[test]
fn ediff_resolve_suffixes_with_conflicts() {
    let (_td, g) = conflicted();
    let mut app = App::new(g);
    press(&mut app, &["E"]);
    let p = popup(&app);
    assert!(p.contains("m Resolve rest") && !p.contains("(m Resolve rest)"), "{p}");
    assert!(p.contains("M Resolve all conflicts") && !p.contains("(M Resolve all conflicts)"), "{p}");
    // magit-read-unmerged-file: "Resolve file", the conflicted file the
    // default. An unknown answer aborts before git mergetool is run.
    app.script_answers(&["no-such-file"]);
    app.press("m").unwrap();
    assert!(app.prompts().iter().any(|p| p.contains("Resolve file")), "{:?}", app.prompts());
    assert_eq!(app.message(), "[No match]");
}

#[test]
fn ediff_stage_reads_a_tracked_file() {
    let (_td, g, _) = repo_with_commit();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g);
    app.script_answers(&["no-such-file"]);
    press(&mut app, &["E", "s"]);
    assert!(app.prompts().iter().any(|p| p.contains("Selectively stage file")), "{:?}", app.prompts());
    assert_eq!(app.message(), "[No match]");
}

#[test]
fn mergetool_without_conflicts() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g);
    press(&mut app, &["!", "m"]);
    assert_eq!(app.message(), "There are no unresolved conflicts");
}

#[test]
fn mergetool_global_git_variables() {
    isolate();
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    press(&mut app, &["E", "C-u", "t"]);
    let p = popup(&app);
    assert!(p.contains("mergetool.hideResolved [true|false|(default:false)]"), "{p}");
    assert!(p.contains("mergetool.keepBackup [true|false|(default:true)]"), "{p}");

    // magit--git-variable:boolean: true, false, unset
    let value = || g.git_string(&["config", "--global", "mergetool.hideResolved"]).ok();
    press(&mut app, &["-", "r"]);
    assert_eq!(value().as_deref(), Some("true"));
    press(&mut app, &["-", "r"]);
    assert_eq!(value().as_deref(), Some("false"));
    press(&mut app, &["-", "r"]);
    assert_eq!(value(), None);

    // merge.guitool is read with magit--read-mergetool and unset again
    app.script_answers(&["vimdiff"]);
    press(&mut app, &["=", "t"]);
    assert_eq!(g.git_string(&["config", "--global", "merge.guitool"]).unwrap(), "vimdiff");
    // invoking it again unsets the variable (magit--git-variable)
    press(&mut app, &["=", "t"]);
    assert!(g.git_string(&["config", "--global", "merge.guitool"]).is_err());
}

#[test]
fn mergetool_transient_and_space_sequence() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g);
    // C-u t shows magit-git-mergetool's transient instead of running it
    press(&mut app, &["E", "C-u", "t"]);
    assert_eq!(app.current_menu(), Some("mergetool-menu"));
    assert_eq!(app.menu_keys(), ["-t", "=t", "=T", "-r", "-b", "-k", "-w", " m"]);

    // -t completes a tool of `git mergetool --tool-help': the candidate is
    // the tool name, not the description git prints after it
    app.script_keys(&["v", "TAB", "RET"]);
    press(&mut app, &["-", "t"]);
    assert!(popup(&app).contains("--tool=vimdiff"), "{}", popup(&app));
    // " m" is SPC then m
    app.press(" ").unwrap();
    assert_eq!(app.current_menu(), Some("mergetool-menu"));
    app.press("m").unwrap();
    assert_eq!(app.message(), "There are no unresolved conflicts");
}
