//! magit-patch (W), magit-patch-create, magit-patch-apply and magit-am (w).

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{App, BufferMode};
use xagit_lib::ui::options::Options;

/// Commits c1..c3 writing f; returns their hashes.
fn repo() -> (TempDir, Git, Vec<String>) {
    let (td, g) = scratch_repo();
    let mut hashes = Vec::new();
    for i in 1..=3 {
        write(&g, "f", &format!("{i}\n"));
        g.git_interactive(&["add", "f"]).unwrap();
        g.commit(Some(&format!("c{i}")), &[]).unwrap();
        hashes.push(g.rev_parse("HEAD").unwrap());
    }
    (td, g, hashes)
}

fn patches(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.ends_with(".patch")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn menus() {
    let (_td, g, _) = repo();
    let mut app = App::new(g);
    app.press("W").unwrap();
    assert_eq!(app.menu_keys(), ["c", "w", "a", "s", "r"]);
    app.press("c").unwrap();
    assert_eq!(app.current_menu(), Some("patch-create-menu"));
    // the mail arguments are at level 6, interdiff etc. at level 5
    assert_eq!(app.menu_keys(), ["C-m b", "C-m v", "C-m d r", "C-m p", "C-m r", "C-m l", "C-m o", "-U", "-M", "-C", "-A", "--", "c"]);
    press(&mut app, &["C-g", "W", "a"]);
    assert_eq!(app.current_menu(), Some("patch-apply-menu"));
    assert_eq!(app.menu_keys(), ["-i", "-c", "-3", "a"]);
}

#[test]
fn create_a_single_commit() {
    let (_td, g, h) = repo();
    let mut app = App::new(g.clone());
    app.script_answers(&[&h[1]]);
    press(&mut app, &["W", "c", "c"]);
    assert_eq!(app.prompts(), ["Create patches for range or commit: "]);
    // only the commit given, not all those since it
    assert_eq!(patches(&g.toplevel), ["0001-c2.patch"], "{}", app.message());
}

#[test]
fn create_with_arguments() {
    let (_td, g, h) = repo();
    std::fs::create_dir(g.toplevel.join("out")).unwrap();
    let mut app = App::new(g.clone());
    app.script_answers(&["out", "2", "XX"]);
    // C-m is RET in a terminal; --rfc and --subject-prefix= are incompatible
    press(&mut app, &["W", "c", "C-m", "o", "RET", "v", "C-m", "p"]);
    let p = app.transient_popup().join("\n");
    assert!(p.contains("--output-directory=") && p.contains("--reroll-count=2"), "{p}");
    assert!(p.contains("--subject-prefix=XX"), "{p}");
    press(&mut app, &["C-m", "r"]);
    assert!(!app.transient_popup().join("\n").contains("--subject-prefix=XX"));
    app.script_answers(&[&format!("{}..HEAD", h[0])]);
    app.press("c").unwrap();
    let out = g.toplevel.join("out");
    assert_eq!(patches(&out), ["v2-0001-c2.patch", "v2-0002-c3.patch"], "{}", app.message());
    let text = std::fs::read_to_string(out.join("v2-0001-c2.patch")).unwrap();
    assert!(text.contains("Subject: [RFC PATCH v2 1/2] c2"), "{text}");
}

#[test]
fn create_with_cover_letter_visits_it() {
    let (_td, g, h) = repo();
    let mut app = App::new(g.clone());
    app.script_answers(&[&format!("{}..HEAD", h[1])]);
    press(&mut app, &["W", "c", "C-m", "l", "c"]);
    assert_eq!(patches(&g.toplevel), ["0000-cover-letter.patch", "0001-c3.patch"], "{}", app.message());
    // the cover letter is visited in xagit's built-in text buffer
    assert_eq!(app.mode(), &BufferMode::TextEdit);
    assert_eq!(app.text_edit_title().as_deref(), Some("0000-cover-letter.patch"));
    assert!(app.text_edit_text().unwrap().contains("*** SUBJECT HERE ***"), "{}", app.message());
    // C-c C-c writes the edited text back to the file
    app.text_edit_set("my cover letter\n");
    press(&mut app, &["C-c", "C-c"]);
    let edited = std::fs::read_to_string(g.toplevel.join("0000-cover-letter.patch")).unwrap();
    assert_eq!(edited, "my cover letter\n");
    assert_eq!(app.mode(), &BufferMode::Status);
}

#[test]
fn thread_style_and_base_readers() {
    let (_td, g, _) = repo();
    let mut app = App::new(g.clone());
    let (o, _) = Options::parse("transient-level magit-patch-create 6");
    app.set_options(o);
    app.script_keys(&["s"]);
    press(&mut app, &["W", "c", "C-m", "s"]);
    assert_eq!(app.prompts(), ["Thread style [d]eep, [s]hallow, or [C-g] to abort "]);
    app.script_answers(&[""]);
    press(&mut app, &["C-m", "b"]);
    assert_eq!(app.prompts()[1], "--base=");
    let p = app.transient_popup().join("\n");
    assert!(p.contains("--thread=shallow") && p.contains("--base=auto"), "{p}");
}

#[test]
fn apply_plain_patch_to_the_index() {
    let (_td, g, _) = repo();
    write(&g, "f", "4\n");
    let diff = g.git_output(&["diff"]).unwrap().stdout;
    std::fs::write(g.toplevel.join("p.diff"), diff).unwrap();
    g.git_interactive(&["checkout", "--", "f"]).unwrap();
    let mut app = App::new(g.clone());
    app.script_answers(&["p.diff"]);
    press(&mut app, &["W", "a", "-", "i", "a"]);
    assert_eq!(app.prompts(), ["Apply patch: "]);
    assert_eq!(g.git_string(&["diff", "--cached", "--name-only"]).unwrap(), "f", "{}", app.message());
}

#[test]
fn am_patches_and_progress() {
    let (_td, g, h) = repo();
    g.git_interactive(&["format-patch", "-q", "-o", "pp", &format!("{}..HEAD", h[0])]).unwrap();
    g.git_interactive(&["reset", "-q", "--hard", &h[0]]).unwrap();
    let mut app = App::new(g.clone());
    app.script_answers(&["pp/0001-c2.patch"]);
    press(&mut app, &["w", "w"]);
    assert_eq!(app.prompts(), ["Apply patch: "]);
    assert_eq!(g.git_string(&["log", "-1", "--format=%s"]).unwrap(), "c2", "{}", app.message());
    // a conflicting patch stops; w then continues, skips or aborts
    write(&g, "f", "other\n");
    g.git_interactive(&["commit", "-qam", "other"]).unwrap();
    app.script_answers(&["pp/0002-c3.patch"]);
    press(&mut app, &["w", "-", "3", "w"]);
    assert!(g.gitdir.join("rebase-apply/applying").exists(), "{}", app.message());
    press(&mut app, &["w"]);
    assert_eq!(app.menu_keys(), ["w", "s", "a"]);
    write(&g, "f", "resolved\n");
    app.press("w").unwrap();
    assert_eq!(app.message(), "Cannot continue due to unstaged changes");
    press(&mut app, &["w", "a"]);
    assert!(!g.gitdir.join("rebase-apply").exists());
    press(&mut app, &["w", "C-g"]);
    assert_eq!(g.git_string(&["log", "-1", "--format=%s"]).unwrap(), "other");
}

#[test]
fn request_pull() {
    let (_td, g, h) = repo();
    g.git_interactive(&["remote", "add", "origin", "https://example.com/r.git"]).unwrap();
    g.git_interactive(&["update-ref", "refs/remotes/origin/main", &h[2]]).unwrap();
    let mut app = App::new(g.clone());
    app.script_answers(&["origin", &h[0], &h[2]]);
    press(&mut app, &["W", "r"]);
    assert_eq!(app.prompts(), ["Remote", "Start", "End"]);
    let (_, text) = app.text_view().unwrap();
    assert!(text.contains("https://example.com/r.git"), "{text}");
}
