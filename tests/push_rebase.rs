//! magit-push (P) and the "onto" suffixes of magit-rebase (r) like Magit,
//! the `:multi-value` push options, and the arguments shared by the
//! committing prefixes (magit:--gpg-sign, magit:--signoff).

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

fn popup(app: &App) -> String {
    app.transient_popup().join("\n")
}

/// A repository with master pushed to the bare repository origin (no
/// upstream configured), and that bare repository.
fn with_bare() -> (TempDir, Git, TempDir, Bare) {
    let (td, g, _) = repo_with_commit();
    let btd = TempDir::new().unwrap();
    let dir = btd.path().join("bare.git");
    let ok = std::process::Command::new("git").args(["init", "-q", "--bare", &dir.to_string_lossy()]).status().unwrap();
    assert!(ok.success());
    git(&g, &["remote", "add", "origin", &dir.to_string_lossy()]);
    git(&g, &["push", "-q", "origin", "master"]);
    git(&g, &["fetch", "-q", "origin"]);
    (td, g, btd, Bare(dir))
}

struct Bare(std::path::PathBuf);

fn rev_bare(b: &Bare, r: &str) -> String {
    let out = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(&b.0)
        .args(["rev-parse", "--verify", "-q", r])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn push_descriptions() {
    let (_td, g, _btd, _bare) = with_bare();
    let mut app = App::new(g.clone());
    app.press("P").unwrap();
    let p = popup(&app);
    assert!(p.contains("p pushRemote, setting that"), "{p}");
    assert!(p.contains("u @{upstream}, creating it"), "{p}");
    assert!(!p.contains("Set push-options"), "-o is at level 5");
    app.press("C-g").unwrap();
    git(&g, &["config", "branch.master.pushRemote", "origin"]);
    git(&g, &["config", "branch.master.remote", "origin"]);
    git(&g, &["config", "branch.master.merge", "refs/heads/nope"]);
    app.press("P").unwrap();
    let p = popup(&app);
    assert!(p.contains("p origin/master"), "{p}");
    assert!(p.contains("u origin creating refs/heads/nope"), "{p}");
    app.press("C-g").unwrap();
    git(&g, &["checkout", "-q", "-b", "topic"]);
    git(&g, &["config", "branch.topic.pushRemote", "origin"]);
    app.press("P").unwrap();
    assert!(popup(&app).contains("p origin/topic, creating it"));
    app.press("C-g").unwrap();
    git(&g, &["config", "branch.topic.pushRemote", "bogus"]);
    app.press("P").unwrap();
    assert!(popup(&app).contains("p pushRemote, replacing invalid"));
    app.press("C-g").unwrap();
    git(&g, &["checkout", "-q", "--detach"]);
    app.press("P").unwrap();
    let keys = app.menu_keys();
    for k in ["p", "u", "e"] {
        assert!(!keys.contains(&k.to_string()), "{k} {keys:?}");
    }
    assert!(keys.contains(&"o".to_string()));
}

#[test]
fn push_current_to_pushremote_and_upstream() {
    let (_td, g, _btd, bare) = with_bare();
    git(&g, &["checkout", "-q", "-b", "topic"]);
    let c = commit(&g, "t.txt");
    let mut app = App::new(g.clone());
    // the push-remote read and set (set-and-push is in magit-no-confirm)
    app.script_answers(&["origin"]);
    press(&mut app, &["P", "p"]);
    assert_eq!(app.prompts(), ["Set branch.topic.pushRemote and push there"]);
    assert_eq!(config(&g, "branch.topic.pushRemote"), "origin");
    assert_eq!(rev_bare(&bare, "refs/heads/topic"), c, "{}", app.message());
    // valid now: not read again
    let c2 = commit(&g, "t2.txt");
    press(&mut app, &["P", "p"]);
    assert_eq!(app.prompts().len(), 1);
    assert_eq!(rev_bare(&bare, "refs/heads/topic"), c2);

    // u: the upstream read and set with --set-upstream
    git(&g, &["checkout", "-q", "-b", "other"]);
    let c3 = commit(&g, "o.txt");
    app.script_answers(&["origin/up"]);
    press(&mut app, &["P", "u"]);
    assert_eq!(app.prompts().last().unwrap(), "Set upstream of other and push there");
    assert_eq!(rev_bare(&bare, "refs/heads/up"), c3, "{}", app.message());
    assert_eq!(config(&g, "branch.other.merge"), "refs/heads/up");
    assert_eq!(config(&g, "branch.other.remote"), "origin");
    let c4 = commit(&g, "o2.txt");
    press(&mut app, &["P", "u"]);
    assert_eq!(app.prompts().len(), 2);
    assert_eq!(rev_bare(&bare, "refs/heads/up"), c4);
    // with set-and-push not in magit-no-confirm it is confirmed
    let (o, _) = Options::parse("magit-no-confirm nil");
    app.set_options(o);
    git(&g, &["checkout", "-q", "-b", "fourth"]);
    app.script_answers(&["origin", "y"]);
    press(&mut app, &["P", "p"]);
    assert_eq!(app.prompts().last().unwrap(), "Really use \"origin\" as push-remote and push \"fourth\" there? (y or n) ");
    assert!(!rev_bare(&bare, "refs/heads/fourth").is_empty(), "{}", app.message());
    // declined: nothing pushed
    git(&g, &["checkout", "-q", "-b", "third"]);
    app.script_answers(&["origin/third", "n"]);
    press(&mut app, &["P", "u"]);
    assert!(rev_bare(&bare, "refs/heads/third").is_empty());
    assert_eq!(app.message(), "Abort");
}

#[test]
fn push_elsewhere_other_refspecs_matching() {
    let (_td, g, _btd, bare) = with_bare();
    let master = rev(&g, "HEAD");
    git(&g, &["checkout", "-q", "-b", "topic"]);
    let c = commit(&g, "t.txt");
    let mut app = App::new(g.clone());
    // e: REMOTE/BRANCH (any), the current branch pushed there
    app.script_answers(&["origin/elsewhere"]);
    press(&mut app, &["P", "e"]);
    assert_eq!(app.prompts().last().unwrap(), "Push topic to");
    assert_eq!(rev_bare(&bare, "refs/heads/elsewhere"), c, "{}", app.message());
    app.script_answers(&["nowhere"]);
    press(&mut app, &["P", "e"]);
    assert_eq!(app.message(), "`nowhere' doesn't have the form REMOTE/BRANCH");

    // o: another branch (or commit)
    app.script_answers(&["master", "origin/copy"]);
    press(&mut app, &["P", "o"]);
    assert_eq!(app.prompts()[app.prompts().len() - 2..], ["Push", "Push master to"]);
    assert_eq!(rev_bare(&bare, "refs/heads/copy"), master, "{}", app.message());

    // r: refspecs separated by commas
    app.script_answers(&["origin", "topic:refs/heads/r1, master:refs/heads/r2"]);
    press(&mut app, &["P", "r"]);
    assert_eq!(app.prompts()[app.prompts().len() - 2..], ["Push to remote", "Push refspec,s"]);
    assert_eq!(rev_bare(&bare, "refs/heads/r1"), c, "{}", app.message());
    assert_eq!(rev_bare(&bare, "refs/heads/r2"), master);

    // m: the matching branches, the only remote not asked for
    git(&g, &["checkout", "-q", "master"]);
    let m2 = commit(&g, "m.txt");
    let n = app.prompts().len();
    press(&mut app, &["P", "m"]);
    assert_eq!(app.prompts().len(), n);
    assert_eq!(rev_bare(&bare, "refs/heads/master"), m2, "{}", app.message());
}

#[test]
fn push_tags_and_notes() {
    let (_td, g, _btd, bare) = with_bare();
    git(&g, &["tag", "v1"]);
    git(&g, &["tag", "v2"]);
    let mut app = App::new(g.clone());
    app.script_answers(&["v1"]);
    press(&mut app, &["P", "T"]);
    assert_eq!(app.prompts().last().unwrap(), "Push tag");
    assert!(!rev_bare(&bare, "refs/tags/v1").is_empty(), "{}", app.message());
    assert!(rev_bare(&bare, "refs/tags/v2").is_empty());
    press(&mut app, &["P", "t"]);
    assert!(!rev_bare(&bare, "refs/tags/v2").is_empty(), "{}", app.message());

    // n (level 6): a notes ref
    git(&g, &["notes", "add", "-m", "note"]);
    let (o, _) = Options::parse("transient-level magit-push 6");
    app.set_options(o);
    app.script_answers(&["commits"]);
    press(&mut app, &["P", "n"]);
    assert_eq!(app.prompts().last().unwrap(), "Push notes");
    assert_eq!(rev_bare(&bare, "refs/notes/commits"), rev(&g, "refs/notes/commits"), "{}", app.message());
}

#[test]
fn push_options_multi_value() {
    let (_td, g, _btd, _bare) = with_bare();
    let mut app = App::new(g.clone());
    let (o, _) = Options::parse("transient-level magit-push 5\nmagit-push-options a b");
    app.set_options(o);
    app.script_answers(&["skip-ci, ci.skip"]);
    press(&mut app, &["P", "-", "o"]);
    assert_eq!(app.active_switches(), ["--push-option=ci.skip", "--push-option=skip-ci"]);
    assert!(popup(&app).contains("--push-option=skip-ci --push-option=ci.skip")
        || popup(&app).contains("--push-option=ci.skip --push-option=skip-ci"), "{}", popup(&app));
    // read again (not unset), empty unsets
    app.script_answers(&[""]);
    press(&mut app, &["-", "o"]);
    assert!(app.active_switches().is_empty());
}

#[test]
fn rebase_onto() {
    let (_td, g, _btd, _bare) = with_bare();
    git(&g, &["checkout", "-q", "-b", "base"]);
    let b = commit(&g, "b.txt");
    git(&g, &["checkout", "-q", "-b", "topic", "master"]);
    commit(&g, "t.txt");
    let mut app = App::new(g.clone());
    app.press("r").unwrap();
    let p = popup(&app);
    assert!(p.contains("p pushRemote, setting that"), "{p}");
    assert!(p.contains("u @{upstream}, setting that"), "{p}");
    assert!(!p.contains("Force rebase"), "level 7");
    app.press("C-g").unwrap();

    // e: magit-read-other-branch-or-commit
    app.script_answers(&["base"]);
    press(&mut app, &["r", "e"]);
    assert_eq!(app.prompts().last().unwrap(), "Rebase onto");
    assert_eq!(rev(&g, "HEAD~"), b, "{}", app.message());

    // u: the upstream read and set
    git(&g, &["checkout", "-q", "base"]);
    let b2 = commit(&g, "b2.txt");
    git(&g, &["checkout", "-q", "topic"]);
    app.script_answers(&["base"]);
    press(&mut app, &["r", "u"]);
    assert_eq!(app.prompts().last().unwrap(), "Set upstream of topic and rebase onto that");
    assert_eq!(config(&g, "branch.topic.merge"), "refs/heads/base");
    assert_eq!(rev(&g, "HEAD~"), b2, "{}", app.message());
    app.press("r").unwrap();
    assert!(popup(&app).contains("u base"));
    app.press("C-g").unwrap();

    // p: the push-remote read and set
    git(&g, &["push", "-q", "origin", "base:topic"]);
    git(&g, &["fetch", "-q", "origin"]);
    git(&g, &["checkout", "-q", "base"]);
    let b3 = commit(&g, "b3.txt");
    git(&g, &["push", "-q", "origin", "base:topic"]);
    git(&g, &["fetch", "-q", "origin"]);
    git(&g, &["checkout", "-q", "topic"]);
    app.script_answers(&["origin"]);
    press(&mut app, &["r", "p"]);
    assert_eq!(app.prompts().last().unwrap(), "Set branch.topic.pushRemote and rebase onto that");
    assert_eq!(rev(&g, "HEAD~"), b3, "{}", app.message());
    app.press("r").unwrap();
    assert!(popup(&app).contains("p origin/topic"));
}

#[test]
fn rebase_merges_and_signing_arguments() {
    let (_td, g, _) = repo_with_commit();
    let mut app = App::new(g.clone());
    app.script_answers(&["r"]);
    press(&mut app, &["r", "-", "r"]);
    assert_eq!(app.active_switches(), ["--autostash", "--rebase-merges=rebase-cousins"]);
    app.press("C-g").unwrap();

    // -S and +s are at the levels 5 and 6
    git(&g, &["config", "gpg.format", "ssh"]);
    app.press("c").unwrap();
    assert!(!popup(&app).contains("Sign using"));
    app.press("C-g").unwrap();
    let (o, _) = Options::parse("transient-level magit-commit 6");
    app.set_options(o);
    app.press("c").unwrap();
    assert!(popup(&app).contains("Sign using ssh"), "{}", popup(&app));
    // not openpgp: the value is empty (:allow-empty)
    press(&mut app, &["-", "S", "+", "s"]);
    assert_eq!(app.active_switches(), ["--gpg-sign=", "--signoff", "--verbose"]);
    app.press("C-g").unwrap();

    // magit-tag:--local-user doesn't allow an empty value
    app.press("t").unwrap();
    press(&mut app, &["-", "u"]);
    assert!(app.active_switches().is_empty());
}
