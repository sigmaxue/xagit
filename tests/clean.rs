//! magit-clean (M-x, a disabled command): disabled-command-function asks
//! with read-multiple-choice, then yes-or-no-p and git clean -f -d
//! [-x|-X] by the prefix argument.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;
use xagit_lib::ui::options::Options;

const USE: &str = "Use this command? ([N]o, [Y]es, SPC (space bar) only once, ! use and enable all): ";

fn setup() -> (tempfile::TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    write(&g, ".gitignore", "*.log\n");
    g.git_interactive(&["add", ".gitignore"]).unwrap();
    g.commit(Some("ignore"), &[]).unwrap();
    write(&g, "u.txt", "u\n");
    std::fs::create_dir(g.toplevel.join("dir")).unwrap();
    write(&g, "dir/v.txt", "v\n");
    write(&g, "x.log", "x\n");
    (td, g)
}

fn exists(g: &Git, f: &str) -> bool {
    g.toplevel.join(f).exists()
}

fn mx(app: &mut App, prefix: &[&str], answers: &[&str]) -> Vec<String> {
    let before = app.prompts().len();
    app.script_answers(answers);
    for k in prefix {
        app.press(k).unwrap();
    }
    app.press("M-x").unwrap();
    app.prompts()[before..].to_vec()
}

#[test]
fn disabled() {
    let (_td, g) = setup();
    let mut app = App::new(g.clone());
    // n: not called
    assert_eq!(mx(&mut app, &[], &["magit-clean", "n"]), ["M-x ", USE]);
    assert!(exists(&g, "u.txt"));
    // another key is an invalid choice, ? shows the help again
    let p = mx(&mut app, &[], &["magit-clean", "x", "?", " ", "no"]);
    let invalid = format!("Invalid choice.  {USE}");
    assert_eq!(p, ["M-x ", USE, &invalid, USE, "Remove untracked files? (yes or no) "]);
    assert!(exists(&g, "u.txt"));
    // SPC: just once
    let p = mx(&mut app, &[], &["magit-clean", " ", "yes"]);
    assert_eq!(p[1..], [USE, "Remove untracked files? (yes or no) "]);
    assert!(!exists(&g, "u.txt") && !exists(&g, "dir/v.txt") && exists(&g, "x.log") && exists(&g, "file.txt"));
    // C-g quits
    app.script_answers(&["magit-clean"]);
    app.script_keys(&["C-g"]);
    app.press("M-x").unwrap();
    assert_eq!(app.message(), "Quit");
    assert_eq!(app.prompts().last().map(String::as_str), Some(USE));
    // !: all disabled commands run
    write(&g, "u.txt", "u\n");
    let p = mx(&mut app, &[], &["magit-clean", "!", "no"]);
    assert_eq!(p[1..], [USE, "Remove untracked files? (yes or no) "]);
    let p = mx(&mut app, &[], &["magit-clean", "yes"]);
    assert_eq!(p, ["M-x ", "Remove untracked files? (yes or no) "]);
    assert!(!exists(&g, "u.txt") && exists(&g, "x.log"));
}

#[test]
fn prefix_argument() {
    let (_td, g) = setup();
    let mut app = App::new(g.clone());
    let (o, _) = Options::parse("disabled-command-function nil");
    app.set_options(o);
    // C-u: the ignored files too (-x)
    let p = mx(&mut app, &["C-u"], &["magit-clean", "yes"]);
    assert_eq!(p, ["C-u M-x ", "Remove untracked and ignored files? (yes or no) "]);
    assert!(!exists(&g, "u.txt") && !exists(&g, "x.log"));
    // C-u C-u: just the ignored files (-X)
    write(&g, "u.txt", "u\n");
    write(&g, "x.log", "x\n");
    let p = mx(&mut app, &["C-u", "C-u"], &["magit-clean", "yes"]);
    assert_eq!(p, ["16 M-x ", "Remove ignored files? (yes or no) "]);
    assert!(exists(&g, "u.txt") && !exists(&g, "x.log"));
    // other arguments ask about the ignored files but remove the untracked ones
    write(&g, "x.log", "x\n");
    let p = mx(&mut app, &["C-2"], &["magit-clean", "yes"]);
    assert_eq!(p, ["2 M-x ", "Remove ignored files? (yes or no) "]);
    assert!(!exists(&g, "u.txt") && exists(&g, "x.log"));
    let p = mx(&mut app, &["M--"], &["magit-clean", "no"]);
    assert_eq!(p[0], "- M-x ");
}

#[test]
fn enable_command() {
    let (td, g) = setup();
    let file = td.path().join("options");
    std::fs::write(&file, "disable-command magit-clean\nmagit-wip-mode nil").unwrap();
    let mut app = App::new(g.clone());
    app.load_options(&file);
    // y, not for future sessions
    let p = mx(&mut app, &[], &["magit-clean", "y", "n", "no"]);
    assert_eq!(p[1..], [USE, "Enable command for future editing sessions also? (y or n) ", "Remove untracked files? (yes or no) "]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "disable-command magit-clean\nmagit-wip-mode nil");
    let p = mx(&mut app, &[], &["magit-clean", "no"]);
    assert_eq!(p[1..], ["Remove untracked files? (yes or no) "]);
    // y, saved in the options file
    let mut app = App::new(g.clone());
    app.load_options(&file);
    mx(&mut app, &[], &["magit-clean", "y", "y", "no"]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "magit-wip-mode nil\nenable-command magit-clean\n");
    let mut app = App::new(g.clone());
    app.load_options(&file);
    let p = mx(&mut app, &[], &["magit-clean", "yes"]);
    assert_eq!(p, ["M-x ", "Remove untracked files? (yes or no) "]);
    assert!(!exists(&g, "u.txt"));
    // without an options file y doesn't ask
    let mut app = App::new(g.clone());
    let p = mx(&mut app, &[], &["magit-clean", "y", "no"]);
    assert_eq!(p[1..], [USE, "Remove untracked files? (yes or no) "]);
}

#[test]
fn wip() {
    let (_td, g) = setup();
    write(&g, "file.txt", "changed\n");
    let mut app = App::new(g.clone());
    let (o, _) = Options::parse("magit-wip-mode t\nenable-command magit-clean");
    app.set_options(o);
    mx(&mut app, &[], &["magit-clean", "yes"]);
    let s = g.git_string(&["log", "-1", "--format=%s", "refs/wip/wtree/refs/heads/master"]).unwrap();
    assert_eq!(s.trim(), "autosave tracked files before clean");
    assert!(!exists(&g, "u.txt"));
}
