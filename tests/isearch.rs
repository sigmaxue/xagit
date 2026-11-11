//! C-s / C-r: searching the buffer shown (status, refs, log, ...).

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::{status_binding, App, Binding, BufferMode};
use xagit_lib::ui::keys::key_from_label;

fn search(app: &mut App, query: &str) {
    app.script_answers(&[query]);
    app.press("C-s").unwrap();
}

fn texts(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

/// One staged file and one unstaged file: two visible lines with ".txt".
fn status_repo() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    write(&g, "notes.txt", "notes\n");
    g.git_interactive(&["add", "notes.txt"]).unwrap();
    write(&g, "file.txt", "changed\n");
    (td, g)
}

/// Two commits whose subjects share a word, with a known author name.
fn log_repo() -> (TempDir, Git) {
    let (td, g) = scratch_repo();
    g.git_interactive(&["config", "user.name", "Tester"]).unwrap();
    g.git_interactive(&["config", "user.email", "tester@example.com"]).unwrap();
    write(&g, "a.txt", "a\n");
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-qm", "common first"]).unwrap();
    write(&g, "b.txt", "b\n");
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-qm", "common second"]).unwrap();
    (td, g)
}

fn log_app(app: &mut App) {
    app.press("l").unwrap();
    app.press("l").unwrap();
    assert!(matches!(app.mode(), BufferMode::Log { .. }), "{:?}", app.mode());
}

#[test]
fn search_keys_are_bound() {
    let b = |l: &str| status_binding(&key_from_label(l));
    assert_eq!(b("C-s"), Some(Binding::IsearchForward));
    assert_eq!(b("C-r"), Some(Binding::IsearchBackward));
    // plain `s` is still stage
    assert_eq!(b("s"), Some(Binding::Command("stage")));
}

#[test]
fn status_search_is_case_insensitive_and_moves_point() {
    let (_td, g) = status_repo();
    let mut app = App::new(g);
    let matches: Vec<String> = texts(&app).into_iter().filter(|t| t.contains(".txt")).collect();
    assert!(matches.len() >= 2, "{matches:?}");

    search(&mut app, ".TXT");
    assert_eq!(app.search_query(), Some(".TXT"));
    assert_eq!(app.prompts().last().map(String::as_str), Some("Search: "));
    assert!(app.point_text().contains(".txt"), "{}", app.point_text());
    assert!(app.message().contains("matches for"), "{}", app.message());
}

#[test]
fn search_repeats_backward_and_wraps_around() {
    let (_td, g) = status_repo();
    let mut app = App::new(g);
    search(&mut app, ".txt");
    let first = app.point_text().to_string();

    app.press("C-s").unwrap(); // repeat, no prompt
    let second = app.point_text().to_string();
    assert_ne!(first, second, "C-s must move to the next match");
    assert_eq!(app.prompts().len(), 1, "repeating must not read a new query");

    app.press("C-r").unwrap(); // previous match
    assert_eq!(app.point_text(), first);

    app.press("C-s").unwrap();
    app.press("C-s").unwrap(); // wrap around
    assert_eq!(app.point_text(), first);
}

#[test]
fn no_match_is_reported_and_c_g_clears_the_highlight() {
    let (_td, g) = status_repo();
    let mut app = App::new(g);
    search(&mut app, "no-such-text-anywhere");
    assert!(app.message().starts_with("[no match]"), "{}", app.message());
    assert_eq!(app.search_query(), Some("no-such-text-anywhere"));

    app.press("C-g").unwrap();
    assert_eq!(app.search_query(), None);
}

#[test]
fn a_prefix_argument_reads_a_new_query() {
    let (_td, g) = status_repo();
    let mut app = App::new(g);
    search(&mut app, ".txt");
    app.script_answers(&["Staged"]);
    app.press("C-u").unwrap();
    app.press("C-s").unwrap();
    assert_eq!(app.search_query(), Some("Staged"));
    assert!(app.point_text().contains("Staged"), "{}", app.point_text());
}

#[test]
fn refs_buffer_search_finds_a_branch() {
    let (_td, g, _) = repo_with_commit();
    g.git_interactive(&["branch", "feature"]).unwrap();
    let mut app = App::new(g);
    app.press("y").unwrap();
    assert!(matches!(app.mode(), BufferMode::Refs { .. }), "{:?}", app.mode());
    assert!(texts(&app).iter().any(|t| t.contains("feature")), "{:#?}", texts(&app));

    search(&mut app, "FEATURE");
    assert!(app.point_text().contains("feature"), "{}", app.point_text());
}

#[test]
fn log_buffer_search_moves_through_commits() {
    let (_td, g) = log_repo();
    let mut app = App::new(g);
    log_app(&mut app);
    let matches: Vec<String> = texts(&app).into_iter().filter(|t| t.contains("common")).collect();
    assert!(matches.len() >= 2, "{matches:?}");

    search(&mut app, "common");
    let first = app.point_text().to_string();
    assert!(first.contains("common"), "{first}");
    app.press("C-r").unwrap(); // backwards, wraps to the other commit
    assert_ne!(app.point_text(), first, "C-r must move to another match");
    assert!(app.point_text().contains("common"));
}

#[test]
fn log_margin_is_searchable() {
    let (_td, g) = log_repo();
    let mut app = App::new(g);
    log_app(&mut app);
    let margins: Vec<String> = app.visible_lines().iter().map(|l| l.margin.clone()).filter(|m| !m.is_empty()).collect();
    assert!(!margins.is_empty(), "the log buffer should show its author margin");
    assert!(margins.iter().any(|m| m.contains("Tester")), "{margins:?}");
    search(&mut app, "Tester");
    assert!(app.message().contains("matches for"), "{}", app.message());
    assert!(!app.point_text().contains("Tester"), "the hit came from the margin: {}", app.point_text());
}

#[test]
fn switching_buffers_clears_the_search() {
    let (_td, g) = status_repo();
    let mut app = App::new(g);
    search(&mut app, ".txt");
    assert!(app.search_query().is_some());
    app.press("y").unwrap();
    assert_eq!(app.search_query(), None);
}

#[test]
fn text_views_report_that_search_is_unavailable() {
    let (_td, g) = status_repo();
    let mut app = App::new(g);
    app.press("$").unwrap(); // process buffer (a text view)
    assert!(app.text_view().is_some());
    app.press("C-s").unwrap();
    assert!(app.message().contains("only available"), "{}", app.message());
    assert_eq!(app.search_query(), None);
}
