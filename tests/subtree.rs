//! magit-subtree (O) with magit-subtree-import and -export.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;

/// A repository with one commit (file a) to import from.
fn source() -> (TempDir, Git, String) {
    let td = TempDir::new().unwrap();
    git_init(td.path());
    let g = Git::discover(td.path()).unwrap();
    write(&g, "a", "1\n");
    g.git_interactive(&["add", "a"]).unwrap();
    g.commit(Some("source"), &[]).unwrap();
    let url = td.path().to_string_lossy().into_owned();
    (td, g, url)
}

#[test]
fn menus() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g);
    app.press("O").unwrap();
    assert_eq!(app.menu_keys(), ["i", "e"]);
    app.press("i").unwrap();
    assert_eq!(app.menu_keys(), ["-P", "-m", "-s", "a", "c", "m", "f"]);
    let p = app.transient_popup().join("\n");
    assert!(p.contains("Add commit") && p.contains("Pull"), "{p}");
    press(&mut app, &["C-g", "O", "e"]);
    assert_eq!(app.menu_keys(), ["-P", "-a", "-b", "-o", "-i", "-j", "p", "s"]);
    // --onto= (magit-transient-read-revision): the current branch the default
    let branch = current_branch(&app);
    app.script_answers(&[""]);
    press(&mut app, &["-", "o"]);
    assert_eq!(app.prompts().last().unwrap(), "--onto=");
    let p = app.transient_popup().join("\n");
    assert!(p.contains(&format!("--onto={branch}")), "{p}");
}

fn current_branch(app: &App) -> String {
    let g = Git::discover(app.toplevel()).unwrap();
    g.git_string(&["symbolic-ref", "--short", "HEAD"]).unwrap()
}

#[test]
fn add_from_a_repository() {
    let (_td, g, _) = repo_with_commit();
    let (_std, _sg, url) = source();
    let mut app = App::new(g.clone());
    app.script_answers(&["vendor", &url, "HEAD"]);
    press(&mut app, &["O", "i", "a"]);
    assert_eq!(app.prompts(), ["Add subtree: ", "From repository", "Ref"]);
    assert_eq!(read(&g, "vendor/a"), "1\n", "{}", app.message());
    assert!(g.git_string(&["log", "-1", "--format=%B"]).unwrap().contains("git-subtree-dir: vendor"));
}

#[test]
fn add_commit_with_the_prefix_argument_and_squash() {
    let (_td, g, _) = repo_with_commit();
    let (_std, _sg, url) = source();
    g.git_interactive(&["fetch", "-q", &url, "HEAD"]).unwrap();
    let commit = g.rev_parse("FETCH_HEAD").unwrap();
    let mut app = App::new(g.clone());
    app.script_answers(&["lib", &commit]);
    press(&mut app, &["O", "i", "-", "P", "-", "s", "c"]);
    // the prefix is not read again
    assert_eq!(app.prompts(), ["--prefix=: ", "Commit"]);
    assert_eq!(read(&g, "lib/a"), "1\n", "{}", app.message());
    // --squash: the commit is merged as a squashed one
    assert_eq!(g.git_string(&["rev-list", "--count", "HEAD"]).unwrap(), "3");
}

#[test]
fn prefix_outside_of_the_repository() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    app.script_answers(&["/elsewhere/lib"]);
    press(&mut app, &["O", "e", "s"]);
    let top = g.toplevel.to_string_lossy().trim_end_matches('/').to_string();
    assert_eq!(app.message(), format!("/elsewhere/lib isn't inside the repository at {top}/"));
}

#[test]
fn merge_and_split() {
    let (_td, g, _) = repo_with_commit();
    let (_std, sg, url) = source();
    let mut app = App::new(g.clone());
    app.script_answers(&["vendor", &url, "HEAD"]);
    press(&mut app, &["O", "i", "a"]);
    // m: merge a newer commit into the subtree
    write(&sg, "a", "2\n");
    sg.git_interactive(&["commit", "-qam", "two"]).unwrap();
    g.git_interactive(&["fetch", "-q", &url, "HEAD"]).unwrap();
    let commit = g.rev_parse("FETCH_HEAD").unwrap();
    app.script_answers(&["vendor", &commit]);
    press(&mut app, &["O", "i", "m"]);
    assert_eq!(app.prompts()[3..], ["Merge into subtree: ", "Commit"]);
    assert_eq!(read(&g, "vendor/a"), "2\n", "{}", app.message());
    // s: split shows the commit of the extracted history
    app.script_answers(&["vendor", "HEAD"]);
    press(&mut app, &["O", "e", "s"]);
    assert_eq!(app.prompts()[5..], ["Split subtree: ", "Commit"]);
    let split = g.git_string(&["rev-parse", &format!("{}^{{tree}}", app.message().trim())]).unwrap();
    assert_eq!(split, g.git_string(&["rev-parse", "HEAD:vendor"]).unwrap());
}
