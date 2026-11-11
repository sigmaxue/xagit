//! Magit's git variable infixes (magit--git-variable and its subclasses):
//! the configuration suffixes of magit-branch, magit-pull,
//! magit-branch-configure, magit-remote and magit-remote-configure; and
//! the suffixes of magit-remote (magit-remote.el).

mod common;

use common::*;
use xagit_lib::prelude::*;
use xagit_lib::ui::app::App;
use xagit_lib::ui::options::Options;

/// Ignore the user's global and system git configuration (the values
/// shown after the choices depend on them).
fn isolate() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("xagit-remote-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gitconfig");
        std::fs::write(&file, "[user]\n\tname = t\n\temail = t@t\n").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &file);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    });
}

fn git(g: &Git, args: &[&str]) {
    g.git_interactive(args).unwrap();
}

fn config_all(g: &Git, key: &str) -> Vec<String> {
    g.git_string(&["config", "--get-all", key])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .collect()
}

fn config(g: &Git, key: &str) -> Option<String> {
    config_all(g, key).into_iter().next()
}

fn popup(app: &App) -> String {
    app.transient_popup().join("\n")
}

fn bare_repo(dir: &std::path::Path) {
    let ok = std::process::Command::new("git").args(["init", "-q", "--bare", &dir.to_string_lossy()]).status().unwrap();
    assert!(ok.success());
}

/// A repository with master pushed to the bare repository origin.
fn with_origin() -> (TempDir, Git, TempDir, std::path::PathBuf) {
    isolate();
    let (td, g, _) = repo_with_commit();
    let btd = TempDir::new().unwrap();
    let dir = btd.path().join("origin.git");
    bare_repo(&dir);
    git(&g, &["remote", "add", "origin", &dir.to_string_lossy()]);
    git(&g, &["push", "-q", "origin", "master"]);
    git(&g, &["fetch", "-q", "origin"]);
    (td, g, btd, dir)
}

#[test]
fn branch_variables_describe_and_cycle() {
    let (_td, g, _btd, _dir) = with_origin();
    let mut app = App::new(g.clone());
    app.press("b").unwrap();
    let p = popup(&app);
    assert!(p.contains("d branch.master.description unset"), "{p}");
    assert!(p.contains("u branch.master.merge unset"), "{p}");
    assert!(p.contains("branch.master.remote unset"), "{p}");
    assert!(p.contains("r branch.master.rebase [true|merges|interactive|false|(default:false)]"), "{p}");
    assert!(p.contains("p branch.master.pushRemote [origin]"), "{p}");
    assert!(p.contains("R pull.rebase [true|merges|interactive|false|(default:false)]"), "{p}");
    assert!(p.contains("P remote.pushDefault [origin]"), "{p}");

    // the transient stays, the descriptions refreshed: the fallback of
    // branch.<branch>.rebase is pull.rebase
    app.press("R").unwrap();
    assert_eq!(app.current_menu(), Some("branch-menu"));
    assert_eq!(config(&g, "pull.rebase").as_deref(), Some("true"));
    let p = popup(&app);
    assert!(p.contains("R pull.rebase [(true)|merges|interactive|false|default:false]"), "{p}");
    assert!(p.contains("r branch.master.rebase [true|merges|interactive|false|(pull.rebase:true)]"), "{p}");
    app.press("P").unwrap();
    let p = popup(&app);
    assert!(p.contains("P remote.pushDefault [(origin)]"), "{p}");
    assert!(p.contains("p branch.master.pushRemote [origin|(remote.pushDefault:origin)]"), "{p}");
    // after the last choice the variable is unset
    app.press("P").unwrap();
    assert_eq!(config(&g, "remote.pushDefault"), None);

    // with a prefix argument a choice is read, the fallback unsetting
    app.script_answers(&["interactive"]);
    press(&mut app, &["C-u", "r"]);
    assert_eq!(app.prompts().last().unwrap(), "Set `branch.master.rebase' to");
    assert_eq!(config(&g, "branch.master.rebase").as_deref(), Some("interactive"));
    assert!(popup(&app).contains("r branch.master.rebase [true|merges|(interactive)|false|pull.rebase:true]"));
    app.script_answers(&["pull.rebase:true"]);
    press(&mut app, &["C-u", "r"]);
    assert_eq!(config(&g, "branch.master.rebase"), None);
    assert_eq!(app.current_menu(), Some("branch-menu"));

    // magit--git-branch:upstream: read when unset, else unset
    app.script_answers(&["origin/master"]);
    app.press("u").unwrap();
    assert_eq!(app.current_menu(), Some("branch-menu"));
    assert_eq!(config(&g, "branch.master.remote").as_deref(), Some("origin"));
    let p = popup(&app);
    assert!(p.contains("u branch.master.merge refs/heads/master"), "{p}");
    assert!(p.contains("branch.master.remote origin"), "{p}");
    app.press("u").unwrap();
    assert_eq!(config(&g, "branch.master.merge"), None);
    app.press("C-g").unwrap();

    // without a branch the branch variables aren't shown
    git(&g, &["checkout", "-q", "--detach"]);
    app.press("b").unwrap();
    let keys = app.menu_keys();
    for k in ["d", "u", "r", "p"] {
        assert!(!keys.contains(&k.to_string()), "{k}");
    }
    assert!(keys.contains(&"R".to_string()));
}

#[test]
fn branch_configure_scope() {
    let (_td, g, _btd, _dir) = with_origin();
    git(&g, &["branch", "other"]);
    let mut app = App::new(g.clone());
    // from another transient: the current branch
    press(&mut app, &["f", "C"]);
    assert_eq!(app.current_menu(), Some("branch-config-menu"));
    assert!(popup(&app).contains("r branch.master.rebase"));
    app.press("C-g").unwrap();
    // from magit-branch, or with a prefix argument: read
    app.script_answers(&["other"]);
    press(&mut app, &["b", "C"]);
    assert_eq!(app.prompts().last().unwrap(), "Configure branch");
    let p = popup(&app);
    assert!(p.contains("r branch.other.rebase"), "{p}");
    assert!(p.contains("a m branch.autoSetupMerge [always|true|false|(default:true)]"), "{p}");
    assert!(p.contains("a r branch.autoSetupRebase [always|local|remote|(never)|default:never]")
        || p.contains("a r branch.autoSetupRebase [always|local|remote|never|(default:never)]"), "{p}");
    app.press("r").unwrap();
    assert_eq!(config(&g, "branch.other.rebase").as_deref(), Some("true"));
    assert_eq!(config(&g, "branch.master.rebase"), None);
    assert!(popup(&app).contains("r branch.other.rebase [(true)|"));
    app.press("C-g").unwrap();

    // the scope of a later invocation is the current branch again
    press(&mut app, &["F"]);
    assert!(popup(&app).contains("r branch.master.rebase"));
    app.press("C-g").unwrap();
    app.script_answers(&["other"]);
    press(&mut app, &["f", "C-u", "C"]);
    assert!(popup(&app).contains("r branch.other.rebase [(true)|"));
    app.press("C-g").unwrap();
}

#[test]
fn remote_variables() {
    let (_td, g, _btd, dir) = with_origin();
    let mut app = App::new(g.clone());
    app.press("M").unwrap();
    let p = popup(&app);
    assert!(p.contains(&format!("u remote.origin.url {}", dir.display())), "{p}");
    assert!(p.contains("U remote.origin.fetch +refs/heads/*:refs/remotes/origin/*"), "{p}");
    assert!(p.contains("s remote.origin.pushurl unset"), "{p}");
    assert!(p.contains("S remote.origin.push unset"), "{p}");
    assert!(p.contains("O remote.origin.tagOpt [--no-tags|--tags]"), "{p}");
    assert!(p.contains("h remote.origin.followRemoteHEAD [create|always|warn|(default:create)]"), "{p}");

    // urls: added and deleted with `git remote set-url`
    app.script_answers(&["/tmp/a,/tmp/b"]);
    app.press("s").unwrap();
    assert_eq!(app.current_menu(), Some("remote-menu"));
    assert_eq!(config_all(&g, "remote.origin.pushurl"), ["/tmp/a", "/tmp/b"]);
    let p = popup(&app);
    assert!(p.contains("s remote.origin.pushurl ") && p.contains("\n       /tmp/a\n       /tmp/b\n"), "{p}");
    // the current values are the initial input
    app.script_keys(&["RET"]);
    app.press("s").unwrap();
    assert_eq!(config_all(&g, "remote.origin.pushurl"), ["/tmp/a", "/tmp/b"]);
    app.script_answers(&["/tmp/b,/tmp/c"]);
    app.press("s").unwrap();
    assert_eq!(config_all(&g, "remote.origin.pushurl"), ["/tmp/b", "/tmp/c"]);
    app.script_answers(&[""]);
    app.press("s").unwrap();
    assert!(config_all(&g, "remote.origin.pushurl").is_empty());
    // url and pushurl share a history
    assert_eq!(app.infix_history("remote.*.url").first().map(String::as_str), Some("/tmp/b,/tmp/c"));

    // the refspecs
    app.script_keys(&[",", "+", "r", "e", "f", "s", "/", "t", "a", "g", "s", "/", "*", ":", "r", "e", "f", "s", "/", "t", "a", "g", "s", "/", "*", "RET"]);
    app.press("U").unwrap();
    assert_eq!(
        config_all(&g, "remote.origin.fetch"),
        ["+refs/heads/*:refs/remotes/origin/*", "+refs/tags/*:refs/tags/*"]
    );

    // a single value: read when unset, else unset
    app.script_answers(&["refs/heads/master"]);
    app.press("S").unwrap();
    assert_eq!(config(&g, "remote.origin.push").as_deref(), Some("refs/heads/master"));
    assert!(popup(&app).contains("S remote.origin.push refs/heads/master"));
    app.press("S").unwrap();
    assert_eq!(config(&g, "remote.origin.push"), None);

    for want in [Some("--no-tags"), Some("--tags"), None] {
        app.press("O").unwrap();
        assert_eq!(config(&g, "remote.origin.tagOpt").as_deref(), want);
    }
    assert_eq!(app.current_menu(), Some("remote-menu"));
    app.press("C-g").unwrap();

    // magit-remote-configure: of another remote with a prefix argument
    git(&g, &["remote", "add", "fork", "/tmp/fork"]);
    app.script_answers(&["fork"]);
    press(&mut app, &["C-u", "M", "C"]);
    assert_eq!(app.current_menu(), Some("remote-configure-menu"));
    assert_eq!(app.prompts().last().unwrap(), "Configure remote");
    assert!(popup(&app).contains("u remote.fork.url /tmp/fork"));
    app.press("h").unwrap();
    assert_eq!(config(&g, "remote.fork.followRemoteHEAD").as_deref(), Some("create"));
    app.press("C-g").unwrap();
}

#[test]
fn remote_add_offers_url_and_push_default() {
    isolate();
    let (_td, g, _) = repo_with_commit();
    git(&g, &["remote", "add", "origin", "git@example.com:owner/repo.git"]);
    let mut app = App::new(g.clone());
    // the url of origin with the owner replaced by the name of the remote
    app.script_answers(&["fork"]);
    app.script_keys(&["RET"]);
    app.script_keys(&["y"]);
    press(&mut app, &["M", "-", "f", "a"]);
    assert_eq!(app.current_menu(), None);
    let prompts = app.prompts();
    assert_eq!(prompts[prompts.len() - 3], "Remote name");
    assert_eq!(prompts[prompts.len() - 2], "Remote url");
    assert!(prompts.last().unwrap().starts_with("Set `remote.pushDefault' to \"fork\"?"), "{prompts:?}");
    assert_eq!(config(&g, "remote.fork.url").as_deref(), Some("git@example.com:fork/repo.git"));
    assert_eq!(config(&g, "remote.pushDefault").as_deref(), Some("fork"));

    // ask-if-unset: not asked when set
    app.script_answers(&["other", "/tmp/other"]);
    press(&mut app, &["M", "-", "f", "a"]);
    assert_eq!(app.prompts().last().unwrap(), "Remote url");
    assert_eq!(config(&g, "remote.other.url").as_deref(), Some("/tmp/other"));
    assert_eq!(config(&g, "remote.pushDefault").as_deref(), Some("fork"));

    // a string: set without asking if the name is that string and the
    // variable is unset
    let (o, _) = Options::parse("magit-remote-add-set-remote.pushDefault \"mine\"");
    app.set_options(o);
    git(&g, &["config", "--unset", "remote.pushDefault"]);
    app.script_answers(&["theirs", "/tmp/theirs"]);
    press(&mut app, &["M", "-", "f", "a"]);
    assert_eq!(config(&g, "remote.pushDefault"), None);
    app.script_answers(&["mine", "/tmp/mine"]);
    press(&mut app, &["M", "-", "f", "a"]);
    assert_eq!(app.prompts().last().unwrap(), "Remote url");
    assert_eq!(config(&g, "remote.pushDefault").as_deref(), Some("mine"));

    // renaming and removing update the push variables
    app.script_answers(&["mine", "ours"]);
    press(&mut app, &["M", "r"]);
    assert_eq!(config(&g, "remote.ours.url").as_deref(), Some("/tmp/mine"));
    assert_eq!(config(&g, "remote.pushDefault").as_deref(), Some("ours"));
    app.script_answers(&["ours"]);
    press(&mut app, &["M", "k"]);
    assert_eq!(config(&g, "remote.ours.url"), None);
    assert_eq!(config(&g, "remote.pushDefault"), None);
}

#[test]
fn remote_prune_refspecs() {
    let (_td, g, _btd, dir) = with_origin();
    let bare = |args: &[&str]| {
        let ok = std::process::Command::new("git").arg("--git-dir").arg(&dir).args(args).status().unwrap();
        assert!(ok.success());
    };
    bare(&["branch", "gone", "master"]);
    git(&g, &["config", "--add", "remote.origin.fetch", "+refs/heads/gone:refs/remotes/origin/gone"]);
    git(&g, &["fetch", "-q", "origin"]);
    let mut app = App::new(g.clone());
    app.script_answers(&["origin"]);
    press(&mut app, &["M", "P"]);
    assert_eq!(app.message(), "No stale refspecs for remote \"origin\"");

    bare(&["branch", "-D", "gone"]);
    app.script_answers(&["origin", "y"]);
    press(&mut app, &["M", "P"]);
    assert_eq!(app.prompts()[app.prompts().len() - 2], "Prune refspecs of remote");
    assert!(app.prompts().last().unwrap().starts_with("Prune stale refspec +refs/heads/gone:refs/remotes/origin/gone and branch refs/remotes/origin/gone"));
    assert_eq!(config_all(&g, "remote.origin.fetch"), ["+refs/heads/*:refs/remotes/origin/*"]);
    assert!(g.git_string(&["rev-parse", "--verify", "-q", "refs/remotes/origin/gone"]).unwrap_or_default().is_empty());

    // all stale: replaced with the default refspec
    git(&g, &["config", "remote.origin.fetch", "+refs/heads/none:refs/remotes/origin/none"]);
    app.script_answers(&["origin", "s"]);
    press(&mut app, &["M", "P"]);
    assert_eq!(
        app.prompts().last().unwrap(),
        "All of origin's refspecs are stale.  replace with [d]efault refspec, [r]emove remote, or [a]abort "
    );
    assert_eq!(config_all(&g, "remote.origin.fetch"), ["+refs/heads/*:refs/remotes/origin/*"]);
}

#[test]
fn remote_unshallow() {
    let (_td, g, _btd, dir) = with_origin();
    write(&g, "second.txt", "2");
    git(&g, &["add", "second.txt"]);
    git(&g, &["commit", "-qm", "second"]);
    git(&g, &["push", "-q", "origin", "master"]);
    let ctd = TempDir::new().unwrap();
    let clone = ctd.path().join("clone");
    let ok = std::process::Command::new("git")
        .args(["clone", "-q", "--depth", "1", &format!("file://{}", dir.display()), &clone.to_string_lossy()])
        .status()
        .unwrap();
    assert!(ok.success());
    let c = Git::discover(&clone).unwrap();
    assert!(c.gitdir.join("shallow").exists());
    let mut app = App::new(c.clone());
    // at level 7
    let (o, _) = Options::parse("transient-level magit-remote 7");
    app.set_options(o);
    app.script_answers(&["yes"]);
    press(&mut app, &["M", "z"]);
    assert_eq!(
        app.prompts().last().unwrap(),
        "Also replace refspec +refs/heads/master:refs/remotes/origin/master with +refs/heads/*:refs/remotes/origin/*? (yes or no) "
    );
    assert_eq!(config_all(&c, "remote.origin.fetch"), ["+refs/heads/*:refs/remotes/origin/*"]);
    assert!(!c.gitdir.join("shallow").exists(), "{}", app.message());
}
