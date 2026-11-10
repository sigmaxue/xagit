//! magit-fetch (f) and magit-pull (F) like Magit: the push-remote and
//! upstream suffixes (their descriptions and `:if`), fetching a branch,
//! a refspec, all remotes and the submodules, pulling a remote branch or
//! into the upstream, and the `:incompatible` arguments of a prefix.

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;
use xagit_lib::ui::options::Options;

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn press(app: &mut App, keys: &[&str]) {
    for k in keys {
        app.press(k).unwrap();
    }
}

fn config(g: &Git, key: &str) -> String {
    g.git_string(&["config", key]).unwrap_or_default().trim().to_string()
}

fn rev(g: &Git, r: &str) -> String {
    g.git_string(&["rev-parse", "--verify", "-q", r]).unwrap_or_default().trim().to_string()
}

fn commit(g: &Git, file: &str) -> String {
    write(g, file, file);
    git(g, &["add", file]);
    git(g, &["commit", "-qm", file]);
    rev(g, "HEAD")
}

/// A repository `up` with the branches master and feature, and a clone
/// of it (origin; master tracking origin/master).
fn cloned() -> (TempDir, Git, TempDir, Git) {
    let (utd, up, _) = repo_with_commit();
    git(&up, &["branch", "feature"]);
    let td = TempDir::new().unwrap();
    let dir = td.path().join("clone");
    let ok = std::process::Command::new("git")
        .args(["clone", "-q", &up.toplevel.to_string_lossy(), &dir.to_string_lossy()])
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let g = Git::discover(&dir).unwrap();
    git(&g, &["config", "user.name", "t"]);
    git(&g, &["config", "user.email", "t@t"]);
    (utd, up, td, g)
}

fn popup(app: &App) -> String {
    app.transient_popup().join("\n")
}

#[test]
fn fetch_descriptions() {
    let (_utd, _up, _td, g) = cloned();
    let mut app = App::new(g.clone());
    app.press("f").unwrap();
    let p = popup(&app);
    assert!(p.contains("p pushRemote, setting that"), "{p}");
    assert!(p.contains("u origin"), "{p}");
    assert!(!p.contains("Fetch full history"), "-u is at level 7");
    app.press("C-g").unwrap();
    git(&g, &["config", "branch.master.pushRemote", "bogus"]);
    app.press("f").unwrap();
    assert!(popup(&app).contains("p pushRemote, replacing invalid"));
    app.press("C-g").unwrap();
    git(&g, &["config", "branch.master.pushRemote", "origin"]);
    app.press("f").unwrap();
    assert!(popup(&app).contains("p origin"));
    app.press("C-g").unwrap();
    git(&g, &["checkout", "-q", "--detach"]);
    app.press("f").unwrap();
    assert!(popup(&app).contains("p pushDefault, setting that"));
    // the only remote is the current remote
    assert!(popup(&app).contains("u origin"));
    app.press("C-g").unwrap();

    // no remote: no u
    let (_td2, solo, _) = repo_with_commit();
    let mut app = App::new(solo);
    app.press("f").unwrap();
    assert!(!app.menu_keys().contains(&"u".to_string()));
    assert!(app.menu_keys().contains(&"p".to_string()));
}

#[test]
fn fetch_from_pushremote_and_upstream() {
    let (_utd, up, _td, g) = cloned();
    let new = commit(&up, "a.txt");
    let mut app = App::new(g.clone());
    app.script_answers(&["origin"]);
    press(&mut app, &["f", "p"]);
    assert_eq!(app.prompts().last().unwrap(), "Set branch.master.pushRemote and fetch from there");
    assert_eq!(config(&g, "branch.master.pushRemote"), "origin");
    assert_eq!(rev(&g, "origin/master"), new, "{}", app.message());
    // now valid: not read again
    let newer = commit(&up, "b.txt");
    let n = app.prompts().len();
    press(&mut app, &["f", "p"]);
    assert_eq!(app.prompts().len(), n);
    assert_eq!(rev(&g, "origin/master"), newer);
    // u: the upstream's remote, -t passed on
    git(&up, &["tag", "v1"]);
    let newest = commit(&up, "c.txt");
    press(&mut app, &["f", "-", "t", "u"]);
    assert_eq!(rev(&g, "origin/master"), newest, "{}", app.message());
    assert!(!rev(&g, "v1").is_empty());
    // e: magit-read-remote
    let last = commit(&up, "d.txt");
    app.script_answers(&[""]);
    press(&mut app, &["f", "e"]);
    assert_eq!(app.prompts().last().unwrap(), "Fetch remote");
    assert_eq!(rev(&g, "origin/master"), last);
}

#[test]
fn fetch_all_branch_and_refspec() {
    let (_utd, up, _td, g) = cloned();
    // a: fetch --all with the arguments (-p prunes)
    git(&up, &["branch", "-D", "feature"]);
    let mut app = App::new(g.clone());
    press(&mut app, &["f", "-", "p", "a"]);
    assert!(rev(&g, "refs/remotes/origin/feature").is_empty(), "{}", app.message());

    // o: a branch of a remote (or url)
    git(&up, &["checkout", "-q", "-b", "other"]);
    let other = commit(&up, "o.txt");
    git(&up, &["checkout", "-q", "master"]);
    app.script_answers(&["origin", "other"]);
    press(&mut app, &["f", "o"]);
    assert_eq!(app.prompts()[app.prompts().len() - 2..], ["Fetch from remote or url", "Fetch branch"]);
    assert_eq!(rev(&g, "FETCH_HEAD"), other, "{}", app.message());

    // r: a refspec (offered from ls-remote)
    app.script_answers(&["origin", "refs/heads/other:refs/remotes/origin/xyz"]);
    press(&mut app, &["f", "r"]);
    assert_eq!(app.prompts().last().unwrap(), "Fetch using refspec");
    assert_eq!(rev(&g, "refs/remotes/origin/xyz"), other, "{}", app.message());

    // an empty branch: nothing selected
    app.script_answers(&["origin", ""]);
    press(&mut app, &["f", "o"]);
    assert_eq!(app.message(), "Nothing selected");
}

#[test]
fn fetch_modules() {
    let (_utd, _up, _td, g) = cloned();
    let mut app = App::new(g.clone());
    // with a prefix argument the transient, with its value
    app.script_answers(&["magit-fetch-modules"]);
    press(&mut app, &["C-u", "M-x"]);
    assert_eq!(app.current_menu(), Some("fetch-modules-menu"));
    assert_eq!(app.active_switches(), ["--jobs=4", "--verbose"]);
    app.press("m").unwrap();
    assert_eq!(app.current_menu(), None);
    assert_eq!(app.message(), "fetched modules", "{}", app.message());
    // without, it fetches
    press(&mut app, &["f", "m"]);
    assert_eq!(app.current_menu(), None);
    assert_eq!(app.message(), "fetched modules");

    let (_td2, solo, _) = repo_with_commit();
    let mut app = App::new(solo);
    press(&mut app, &["f", "m"]);
    assert_eq!(app.message(), "Cannot fetch modules if super-repository has no remotes");
}

#[test]
fn pull_descriptions() {
    let (_utd, _up, _td, g) = cloned();
    let mut app = App::new(g.clone());
    app.press("F").unwrap();
    let p = popup(&app);
    assert!(p.contains("u origin/master"), "{p}");
    assert!(p.contains("p pushRemote, setting that"), "{p}");
    assert!(p.contains("r branch.<branch>.rebase"), "{p}");
    assert!(!p.contains("Autostash"), "-A is at level 7");
    assert!(!app.menu_keys().contains(&"U".to_string()));
    app.press("C-g").unwrap();
    git(&g, &["config", "branch.master.pushRemote", "origin"]);
    app.press("F").unwrap();
    assert!(popup(&app).contains("p origin/master"));
    app.press("C-g").unwrap();
    git(&g, &["checkout", "-q", "-b", "solo"]);
    git(&g, &["config", "branch.solo.pushRemote", "origin"]);
    app.press("F").unwrap();
    let p = popup(&app);
    assert!(p.contains("p pushRemote, replacing non-existent"), "{p}");
    assert!(p.contains("u @{upstream}, setting that"), "{p}");
    app.press("C-g").unwrap();
    git(&g, &["config", "branch.solo.remote", "origin"]);
    git(&g, &["config", "branch.solo.merge", "refs/heads/nope"]);
    app.press("F").unwrap();
    assert!(popup(&app).contains("u @{upstream}, replacing non-existent"));
    app.press("C-g").unwrap();
    // detached: no p, u or r
    git(&g, &["checkout", "-q", "--detach"]);
    app.press("F").unwrap();
    let keys = app.menu_keys();
    for k in ["p", "u", "r"] {
        assert!(!keys.contains(&k.to_string()), "{k} {keys:?}");
    }
    assert!(keys.contains(&"e".to_string()));
}

#[test]
fn pull_from_upstream_and_pushremote() {
    let (_utd, up, _td, g) = cloned();
    let new = commit(&up, "a.txt");
    let mut app = App::new(g.clone());
    press(&mut app, &["F", "u"]);
    assert_eq!(rev(&g, "HEAD"), new, "{}", app.message());
    // without an upstream it is read and set
    git(&g, &["checkout", "-q", "-b", "solo", "HEAD~"]);
    app.script_answers(&["origin/master"]);
    press(&mut app, &["F", "u"]);
    assert_eq!(app.prompts().last().unwrap(), "Set upstream of solo and pull from there");
    assert_eq!(config(&g, "branch.solo.merge"), "refs/heads/master");
    assert_eq!(rev(&g, "HEAD"), new, "{}", app.message());
    // p: the push-remote is read and set
    let newer = commit(&up, "b.txt");
    app.script_answers(&["origin"]);
    git(&g, &["checkout", "-q", "master"]);
    press(&mut app, &["F", "p"]);
    assert_eq!(app.prompts().last().unwrap(), "Set branch.master.pushRemote and pull from there");
    assert_eq!(config(&g, "branch.master.pushRemote"), "origin");
    assert_eq!(rev(&g, "HEAD"), newer, "{}", app.message());
}

#[test]
fn pull_branch_and_into_upstream() {
    let (_utd, up, _td, g) = cloned();
    git(&up, &["checkout", "-q", "feature"]);
    let f = commit(&up, "f.txt");
    git(&up, &["checkout", "-q", "master"]);
    git(&g, &["fetch", "-q", "origin"]);
    let mut app = App::new(g.clone());
    app.script_answers(&["origin/feature"]);
    press(&mut app, &["F", "e"]);
    assert_eq!(app.prompts().last().unwrap(), "Pull");
    assert_eq!(rev(&g, "HEAD"), f, "{}", app.message());

    // U (level 7): the local upstream updated from its upstream
    let new = commit(&up, "n.txt");
    git(&g, &["checkout", "-q", "-b", "topic"]);
    git(&g, &["branch", "--set-upstream-to", "master", "topic"]);
    let mut app = App::new(g.clone());
    let (o, _) = Options::parse("transient-level magit-pull 7");
    app.set_options(o);
    app.press("F").unwrap();
    assert!(popup(&app).contains("U origin/master"), "{}", popup(&app));
    assert!(popup(&app).contains("-A Autostash"));
    app.press("U").unwrap();
    assert_eq!(rev(&g, "refs/heads/master"), new, "{}", app.message());
    assert_eq!(g.current_branch().as_deref(), Some("topic"));
    // master without upstream: no U
    git(&g, &["branch", "--unset-upstream", "master"]);
    app.press("F").unwrap();
    assert!(!app.menu_keys().contains(&"U".to_string()));
}

#[test]
fn incompatible_arguments() {
    let (_utd, _up, _td, g) = cloned();
    let mut app = App::new(g.clone());
    // pull: --rebase= is an option of choices
    app.script_answers(&["true"]);
    press(&mut app, &["F", "-", "r"]);
    assert_eq!(app.active_switches(), ["--rebase=true"]);
    press(&mut app, &["-", "f"]);
    assert_eq!(app.active_switches(), ["--ff-only"]);
    app.script_answers(&["false"]);
    press(&mut app, &["-", "r"]);
    assert_eq!(app.active_switches(), ["--ff-only", "--rebase=false"]);
    // a :choices option set is unset by its key
    press(&mut app, &["-", "r"]);
    assert_eq!(app.active_switches(), ["--ff-only"]);
    app.script_answers(&["merges"]);
    press(&mut app, &["-", "r"]);
    assert_eq!(app.prompts().last().unwrap(), "--rebase=");
    assert_eq!(app.active_switches(), ["--rebase=merges"]);
    app.press("C-g").unwrap();
    // merge
    press(&mut app, &["m", "-", "f", "-", "n"]);
    assert_eq!(app.active_switches(), ["--no-ff"]);
    press(&mut app, &["-", "f"]);
    assert_eq!(app.active_switches(), ["--ff-only"]);
    app.press("C-g").unwrap();
    // cherry-pick: --ff (the default) and -x
    press(&mut app, &["A"]);
    assert_eq!(app.active_switches(), ["--ff"]);
    press(&mut app, &["-", "x"]);
    assert_eq!(app.active_switches(), ["-x"]);
}
