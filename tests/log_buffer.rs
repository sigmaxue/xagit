//! Log and reflog buffers (magit-log-mode, magit-reflog-mode): commit
//! lines with graph, refs and margin, the commit limit, movement and the
//! arguments of `l` / `L`.

mod common;

use common::*;
use ngit_lib::prelude::*;
use ngit_lib::ui::app::{App, BufferMode};
use ngit_lib::ui::logbuf::{
    age, author_margin, format_shortstat, format_time, reflog_subject, Margin, MarginStyle, DEFAULT_TIME_FORMAT,
    LOG_MARGIN_WIDTH,
};

/// initial -> three (master) and two (side), merged into master, tagged v1.
fn merge_repo() -> (TempDir, Git) {
    let (td, g, _) = repo_with_commit();
    g.git_interactive(&["checkout", "-q", "-b", "side"]).unwrap();
    write(&g, "g.txt", "g\n");
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", "two"]).unwrap();
    g.git_interactive(&["checkout", "-q", "master"]).unwrap();
    write(&g, "file.txt", "changed\n");
    g.git_interactive(&["commit", "-q", "-am", "three"]).unwrap();
    g.git_interactive(&["merge", "-q", "--no-edit", "side"]).unwrap();
    g.git_interactive(&["tag", "v1"]).unwrap();
    (td, g)
}

fn short(g: &Git, rev: &str) -> String {
    g.git_string(&["rev-parse", "--short", rev]).unwrap()
}

fn texts(app: &App) -> Vec<String> {
    app.visible_lines().iter().map(|l| l.text.clone()).collect()
}

fn goto(app: &mut App, prefix: &str) {
    let n = app.visible_lines().len();
    for _ in 0..n {
        app.press("C-p").unwrap();
    }
    for _ in 0..n {
        if app.point_text().starts_with(prefix) {
            return;
        }
        app.press("C-n").unwrap();
    }
    panic!("no visible line starting with {prefix:?} in {:#?}", texts(app));
}

fn log_current(app: &mut App) {
    app.press("l").unwrap();
    app.press("l").unwrap();
    assert!(matches!(app.mode(), BufferMode::Log { .. }), "{:?}", app.mode());
}

#[test]
fn log_buffer_layout() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g.clone());
    log_current(&mut app);
    assert_eq!(app.header_line(), "Commits in master");
    let (m, t, s, i) = (short(&g, "HEAD"), short(&g, "HEAD^1"), short(&g, "side"), short(&g, "HEAD~2"));
    let pad = " ".repeat(m.len() + 1);
    assert_eq!(
        texts(&app),
        [
            format!("{m} *   master v1 Merge branch 'side'"),
            format!("{pad}|\\  "),
            format!("{s} | * side two"),
            format!("{t} * | three"),
            format!("{pad}|/  "),
            format!("{i} * initial"),
        ]
    );
    // graph lines belong to the commit above them; RET there shows it
    let lines = app.visible_lines();
    assert!(lines[1].ancestors.is_empty() && !lines[1].heading);
    let author = g.git_string(&["log", "-1", "--format=%aN"]).unwrap();
    assert!(lines[0].margin.starts_with(&author), "{:?}", lines[0].margin);
    assert!(lines[1].margin.is_empty());
    goto(&mut app, &format!("{pad}|\\"));
    app.press("RET").unwrap();
    assert_eq!(app.mode(), &BufferMode::Revision { rev: m.clone(), files: vec![] });
    // q returns to the log, at the same commit
    app.press("q").unwrap();
    assert!(matches!(app.mode(), BufferMode::Log { .. }));
    app.press("SPC").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { .. }), "SPC shows the commit");
    app.press("q").unwrap();
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
}

#[test]
fn margin_and_age() {
    assert_eq!(age(0.4), (0, "seconds"));
    assert_eq!(age(1.0), (1, "second"));
    assert_eq!(age(90.0), (2, "minutes"));
    assert_eq!(age(3.0 * 86_400.0), (3, "days"));
    assert_eq!(age(-3.0 * 86_400.0), (3, "days"));
    assert_eq!(age(400.0 * 86_400.0), (1, "year"));
    let m = author_margin("Jane", "1000", 1000.0 + 7200.0);
    assert_eq!(m, format!("{:<18}  2 hours     ", "Jane")[..LOG_MARGIN_WIDTH].to_string());
    assert_eq!(m.chars().count(), LOG_MARGIN_WIDTH);
    let long = author_margin("A very long author name", "0", 0.0);
    assert!(long.starts_with("A very long autho… "), "{long}");
}

#[test]
fn commit_limit_and_more_button() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g);
    log_current(&mut app);
    // -n256 halved down to -n2
    for _ in 0..7 {
        app.press("-").unwrap();
    }
    let t = texts(&app);
    assert_eq!(t.iter().filter(|l| l.contains('*')).count(), 2, "{t:#?}");
    assert_eq!(t.last().unwrap(), "Type + to show more history");
    // the button doubles the limit
    goto(&mut app, "Type + to show more history");
    app.press("RET").unwrap();
    let t = texts(&app);
    assert_eq!(t.iter().filter(|l| l.contains('*')).count(), 4, "{t:#?}");
    // exactly as many commits as the limit: there might be more
    assert_eq!(t.last().unwrap(), "Type + to show more history");
    app.press("+").unwrap();
    let t = texts(&app);
    assert_eq!(t.iter().filter(|l| l.contains('*')).count(), 4, "{t:#?}");
    assert!(!t.iter().any(|l| l.starts_with("Type +")));
    // = removes the limit, and sets it to 256 again
    app.press("=").unwrap();
    app.press("L").unwrap();
    assert!(!app.active_switches().iter().any(|a| a.starts_with("-n")), "{:?}", app.active_switches());
    app.press("q").unwrap();
    app.press("=").unwrap();
    app.press("L").unwrap();
    assert!(app.active_switches().contains(&"-n256".to_string()));
}

#[test]
fn move_to_revision_and_parent() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g.clone());
    log_current(&mut app);
    app.script_answers(&["side"]);
    app.press("j").unwrap();
    assert!(app.point_text().starts_with(&short(&g, "side")), "{}", app.point_text());
    app.script_answers(&["nope"]);
    app.press("j").unwrap();
    assert_eq!(app.message(), "nope isn't visible in the current log buffer");
    // C-c C-n on the merge moves to its first parent
    goto(&mut app, &short(&g, "HEAD"));
    app.press("C-c").unwrap();
    app.press("C-n").unwrap();
    assert!(app.point_text().ends_with("three"), "{}", app.point_text());
    app.press("C-c").unwrap();
    app.press("C-n").unwrap();
    assert!(app.point_text().ends_with("initial"), "{}", app.point_text());
    app.press("C-c").unwrap();
    app.press("C-n").unwrap();
    assert_eq!(app.message(), format!("Parent {}^1 does not exist", short(&g, "HEAD~2")));
}

#[test]
fn log_refresh_changes_only_the_buffer_shown() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g);
    log_current(&mut app);
    app.press("L").unwrap();
    assert_eq!(app.active_switches(), ["--decorate", "--graph", "-n256"]);
    app.press("-").unwrap();
    app.press("g").unwrap();
    app.press("g").unwrap();
    let t = texts(&app);
    assert_eq!(t.len(), 4, "{t:#?}");
    assert!(!t.iter().any(|l| l.contains('*')), "{t:#?}");
    // `l` in a log buffer starts with that buffer's arguments
    app.press("l").unwrap();
    assert_eq!(app.active_switches(), ["--decorate", "-n256"]);
    app.press("q").unwrap();
    app.press("q").unwrap();
    // new log buffers still use the default arguments
    log_current(&mut app);
    assert!(texts(&app)[0].contains('*'));
}

#[test]
fn log_with_diffstat_and_patches() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g.clone());
    // l -g (no graph) -p (patches) l
    app.press("l").unwrap();
    app.press("-").unwrap();
    app.press("g").unwrap();
    app.press("-").unwrap();
    app.press("p").unwrap();
    app.press("l").unwrap();
    let t = texts(&app);
    let three = t.iter().position(|l| l.ends_with(" three")).unwrap();
    assert_eq!(&t[three + 1..three + 4], ["modified   file.txt", "@@ -1 +1 @@", "-hello"], "{t:#?}");
    // TAB folds the commit
    goto(&mut app, &short(&g, "HEAD^1"));
    app.press("TAB").unwrap();
    assert!(!texts(&app).iter().any(|l| l == "-hello"));
    app.press("TAB").unwrap();
    // RET on a removed line visits the parent's version of the file
    goto(&mut app, "-hello");
    app.press("RET").unwrap();
    assert_eq!(app.header_line(), format!("file.txt.~{}~", short(&g, "HEAD~2")));
    assert_eq!(texts(&app), ["hello"]);
    app.press("q").unwrap();
    // u on a committed hunk reverses it in the index
    goto(&mut app, "+changed");
    app.script_answers(&["y"]);
    app.press("u").unwrap();
    let cached = g.git_string(&["diff", "--cached"]).unwrap();
    assert!(cached.contains("-changed\n+hello"), "{cached}");
}

#[test]
fn log_from_commit_goes_to_that_commit() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g.clone());
    goto(&mut app, "Recent commits");
    app.press("TAB").unwrap();
    app.press("C-n").unwrap();
    app.press("C-n").unwrap();
    let at = app.point_text().to_string();
    log_current(&mut app);
    let h = at.split(' ').next().unwrap();
    assert!(app.point_text().starts_with(h), "{} / {at}", app.point_text());
}

#[test]
fn log_other_related_and_file() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g.clone());
    app.script_answers(&["side, v1"]);
    app.press("l").unwrap();
    app.press("o").unwrap();
    assert_eq!(app.header_line(), "Commits in side v1");
    app.press("l").unwrap();
    app.press("u").unwrap();
    assert_eq!(app.header_line(), "Commits in master");
    app.press("l").unwrap();
    app.press("b").unwrap();
    assert_eq!(app.header_line(), "Commits in --branches --remotes");
    app.script_answers(&["g.txt"]);
    app.press("l").unwrap();
    app.press("f").unwrap();
    assert_eq!(app.header_line(), "Commits in master touching g.txt");
    let t = texts(&app);
    assert_eq!(t.len(), 1, "{t:#?}");
    assert!(t[0].ends_with("* side two"), "{t:#?}");
}

#[test]
fn reflog_buffer() {
    let (_td, g) = merge_repo();
    g.git_interactive(&["commit", "-q", "--amend", "-m", "Merge branch 'side'", "-m", "amended"]).unwrap();
    let mut app = App::new(g.clone());
    app.press("l").unwrap();
    app.press("r").unwrap();
    assert_eq!(app.mode(), &BufferMode::Reflog { refname: "master".into() });
    assert_eq!(app.header_line(), "Reflog for master");
    let t = texts(&app);
    let h = short(&g, "HEAD");
    assert_eq!(t[0], format!("{h} 0  amend            Merge branch 'side'"));
    assert!(t[1].ends_with(" 1  merge            side: Merge made by the 'ort' strategy."), "{t:#?}");
    assert!(t[2].ends_with(" 2  commit           three"), "{t:#?}");
    assert!(t[3].ends_with(" 3  initial          initial"), "{t:#?}");
    assert!(app.visible_lines()[0].margin.ends_with("seconds") || app.visible_lines()[0].margin.contains("second"));
    // L is magit-margin-settings; magit-log-refresh (h L) refuses reflogs
    app.press("L").unwrap();
    assert_eq!(app.current_menu(), Some("margin-settings-menu"));
    app.press("C-g").unwrap();
    app.press("h").unwrap();
    app.press("L").unwrap();
    assert_eq!(app.message(), "Cannot change log arguments in reflog buffers");
    assert!(app.current_menu().is_none());
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Revision { .. }));
}

#[test]
fn log_options() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g.clone());
    app.press("l").unwrap();
    // -n is an option: typing it unsets it, typing it again reads a value
    assert!(app.active_switches().contains(&"-n256".to_string()));
    app.press("-").unwrap();
    app.press("n").unwrap();
    assert!(!app.active_switches().iter().any(|a| a.starts_with("-n")));
    app.script_answers(&["0"]);
    app.press("-").unwrap();
    app.press("n").unwrap();
    assert_eq!(app.message(), "Please enter a positive integer");
    app.script_answers(&["2"]);
    app.press("-").unwrap();
    app.press("n").unwrap();
    assert!(app.active_switches().contains(&"-n2".to_string()));
    // -F: --grep=
    app.script_answers(&["thr"]);
    app.press("-").unwrap();
    app.press("F").unwrap();
    // -o cycles through the orders and back to none
    app.press("-").unwrap();
    app.press("o").unwrap();
    assert!(app.active_switches().contains(&"--topo-order".to_string()));
    app.press("-").unwrap();
    app.press("o").unwrap();
    assert!(app.active_switches().contains(&"--author-date-order".to_string()));
    assert!(!app.active_switches().contains(&"--topo-order".to_string()));
    app.press("-").unwrap();
    app.press("o").unwrap();
    app.press("-").unwrap();
    app.press("o").unwrap();
    assert!(!app.active_switches().iter().any(|a| a.ends_with("-order")), "{:?}", app.active_switches());
    app.press("l").unwrap();
    let t = texts(&app);
    assert_eq!(t.len(), 1, "{t:#?}");
    assert!(t[0].ends_with("* three"), "{t:#?}");
    // L shows the buffer's values
    app.press("L").unwrap();
    assert!(app.active_switches().contains(&"--grep=thr".to_string()));
    assert!(app.active_switches().contains(&"-n2".to_string()));
}

#[test]
fn log_trace_lines() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g.clone());
    app.press("l").unwrap();
    app.script_answers(&["file.txt", "1,1"]);
    app.press("-").unwrap();
    app.press("L").unwrap();
    assert!(app.active_switches().contains(&"-L1,1:file.txt".to_string()), "{:?}", app.active_switches());
    app.press("l").unwrap();
    assert_eq!(app.header_line(), "Commits in master -L1,1:file.txt");
    let t = texts(&app);
    // no graph with -L; each commit's diff is its body
    let three = t.iter().position(|l| l.ends_with(" three")).unwrap_or_else(|| panic!("{t:#?}"));
    assert!(!t[three].contains('*'), "{t:#?}");
    assert!(t[three + 1..].iter().any(|l| l == "+changed"), "{t:#?}");
    assert!(t.iter().any(|l| l.ends_with(" initial")), "{t:#?}");
}

#[test]
fn margin_styles() {
    let mut m = Margin::log();
    assert_eq!(m.width(), LOG_MARGIN_WIDTH);
    assert_eq!(m.author("Jane", "1000", 1000.0 + 7200.0), format!("{:<18}  2 hours   ", "Jane"));
    m.cycle_style();
    assert_eq!(m.style, MarginStyle::AgeAbbreviated);
    assert_eq!(m.width(), 18 + 1 + 4);
    assert_eq!(m.author("Jane", "1000", 1000.0 + 7200.0), format!("{:<18}  2h ", "Jane"));
    m.cycle_style();
    assert_eq!(m.style, MarginStyle::Format(DEFAULT_TIME_FORMAT.into()));
    assert_eq!(m.width(), 18 + 1 + 17);
    let date = format_time("%Y-%m-%d %H:%M ", 86_400 * 365);
    assert!(date.starts_with("1971-01-0"), "{date}");
    assert_eq!(m.author("Jane", &(86_400 * 365).to_string(), 0.0), format!("{:<18} {date}", "Jane"));
    m.cycle_style();
    assert_eq!(m.style, MarginStyle::Age);
    // without details
    m.details = false;
    assert_eq!(m.width(), 11);
    assert_eq!(m.author("Jane", "0", 3.0 * 86_400.0), " 3 days    ");
    // the shortstat style
    assert_eq!(format_shortstat("2 files changed, 10 insertions(+), 3 deletions(-)"), "  10+    3-   2");
    assert_eq!(format_shortstat("1 file changed, 1 deletion(-)"), "         1-   1");
    assert_eq!(format_shortstat(""), "");
    m.shortstat = true;
    assert_eq!(m.width(), 16);
}

#[test]
fn margin_settings() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g.clone());
    log_current(&mut app);
    let lines = app.visible_lines();
    assert_eq!(lines[0].margin.chars().count(), LOG_MARGIN_WIDTH);
    // L L hides the margin, the menu stays
    app.press("L").unwrap();
    app.press("L").unwrap();
    assert_eq!(app.current_menu(), Some("log-refresh-menu"));
    assert!(!app.margin_config().enable);
    // l cycles the style
    app.press("L").unwrap();
    app.press("l").unwrap();
    assert!(app.visible_lines()[0].margin.ends_with("s "), "{:?}", app.visible_lines()[0].margin);
    // d hides the author
    app.press("d").unwrap();
    assert_eq!(app.visible_lines()[0].margin.chars().count(), 4);
    // x shows the shortstat
    app.press("x").unwrap();
    let t = texts(&app);
    let three = t.iter().position(|l| l.ends_with(" three")).unwrap();
    assert_eq!(app.visible_lines()[three].margin, "   1+    1-   1");
    app.press("x").unwrap();
    app.press("C-g").unwrap();
    // the setting is the buffer's: another log keeps it
    app.press("l").unwrap();
    app.press("a").unwrap();
    assert_eq!(app.visible_lines()[0].margin.chars().count(), 4);

    // the status buffer has no margin until L L
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    goto(&mut app, "Recent commits");
    app.press("TAB").unwrap();
    let at = app.visible_lines().iter().position(|l| l.text.ends_with(" three")).unwrap();
    assert!(!app.margin_config().enable);
    app.press("L").unwrap();
    app.press("L").unwrap();
    app.press("C-g").unwrap();
    assert!(app.visible_lines()[at].margin.contains("second"), "{:?}", app.visible_lines()[at].margin);
    // without details: only the age
    assert_eq!(app.visible_lines()[at].margin.chars().count(), 11);

    // reflog buffers: L opens magit-margin-settings
    app.press("l").unwrap();
    app.press("r").unwrap();
    assert!(app.visible_lines()[0].margin.contains("second"));
    app.press("L").unwrap();
    app.press("d").unwrap();
    let author = g.git_string(&["log", "-1", "--format=%aN"]).unwrap();
    assert!(app.visible_lines()[0].margin.starts_with(&author), "{:?}", app.visible_lines()[0].margin);
    app.press("L").unwrap();
    assert!(app.visible_lines()[0].margin.is_empty() || !app.margin_config().enable);
    // x is not a margin-settings command
    app.press("x").unwrap();
    assert_eq!(app.current_menu(), Some("margin-settings-menu"));
}

#[test]
fn reflog_subjects() {
    assert_eq!(reflog_subject("commit"), format!("{:<16} ", "commit"));
    assert_eq!(reflog_subject("commit (amend)"), format!("{:<16} ", "amend"));
    assert_eq!(reflog_subject("checkout"), format!("{:<16} ", "checkout"));
    assert_eq!(reflog_subject("pull --rebase (finish)"), format!("{:<16} ", "pull --rebase finish"));
    assert_eq!(reflog_subject("rebase (start)"), format!("{:<16} ", "rebase start"));
}

#[test]
fn stashes_buffer() {
    let (_td, g) = merge_repo();
    write(&g, "file.txt", "one\n");
    g.git_interactive(&["stash", "-q"]).unwrap();
    write(&g, "file.txt", "two\n");
    g.git_interactive(&["stash", "-q", "-m", "custom"]).unwrap();
    let mut app = App::new(g.clone());
    app.press("z").unwrap();
    app.press("l").unwrap();
    assert_eq!(app.mode(), &BufferMode::Stashes { refname: "refs/stash".into() });
    let t = texts(&app);
    assert_eq!(t.len(), 3, "{t:#?}");
    assert_eq!(t[0], "Stashes");
    assert_eq!(t[1], "stash@{0} On master: custom");
    assert!(t[2].starts_with("stash@{1} WIP on master: "), "{t:#?}");
    assert!(app.visible_lines()[1].margin.contains("second"), "{:?}", app.visible_lines()[1].margin);
    // magit-stashes-mode derives from magit-reflog-mode
    app.press("L").unwrap();
    assert_eq!(app.current_menu(), Some("margin-settings-menu"));
    app.press("C-g").unwrap();
    // RET shows the stash, q comes back
    goto(&mut app, "stash@{0}");
    app.press("RET").unwrap();
    assert_eq!(app.mode(), &BufferMode::Stash { stash: "stash@{0}".into() });
    app.press("q").unwrap();
    assert!(matches!(app.mode(), BufferMode::Stashes { .. }));
    // k drops the stash at point
    goto(&mut app, "stash@{1}");
    app.script_answers(&["y"]);
    app.press("k").unwrap();
    assert_eq!(texts(&app), ["Stashes", "stash@{0} On master: custom"]);
    // RET on the stashes section of the status buffer lists them too
    app.press("q").unwrap();
    assert_eq!(app.mode(), &BufferMode::Status);
    goto(&mut app, "Stashes (1)");
    app.press("RET").unwrap();
    assert!(matches!(app.mode(), BufferMode::Stashes { .. }));
}

#[test]
fn cherry_buffer() {
    let (_td, g, _) = repo_with_commit();
    g.git_interactive(&["checkout", "-q", "-b", "topic"]).unwrap();
    for name in ["a", "b"] {
        write(&g, &format!("{name}.txt"), "x\n");
        g.git_interactive(&["add", "."]).unwrap();
        g.git_interactive(&["commit", "-q", "-m", name]).unwrap();
    }
    g.git_interactive(&["checkout", "-q", "master"]).unwrap();
    write(&g, "m.txt", "m\n");
    g.git_interactive(&["add", "."]).unwrap();
    g.git_interactive(&["commit", "-q", "-m", "m"]).unwrap();
    g.git_interactive(&["cherry-pick", "topic~1"]).unwrap();
    g.git_interactive(&["checkout", "-q", "topic"]).unwrap();
    g.git_interactive(&["branch", "-q", "-u", "master"]).unwrap();
    let (a, b) = (short(&g, "topic~1"), short(&g, "topic"));
    let mut app = App::new(g.clone());
    app.script_answers(&["topic", "master"]);
    app.press("Y").unwrap();
    assert_eq!(app.mode(), &BufferMode::Cherry { head: "topic".into(), upstream: "master".into() });
    assert_eq!(
        texts(&app),
        [
            "Head:     topic b".to_string(),
            "Upstream: master a".to_string(),
            String::new(),
            "Cherry commits".to_string(),
            format!("+ {b} b"),
            format!("- {a} a"),
        ]
    );
    let author = g.git_string(&["log", "-1", "--format=%aN"]).unwrap();
    assert!(app.visible_lines()[4].margin.starts_with(&author));
    app.press("L").unwrap();
    assert_eq!(app.current_menu(), Some("margin-settings-menu"));
    app.press("C-g").unwrap();
    app.press("h").unwrap();
    app.press("L").unwrap();
    assert_eq!(app.message(), "Cannot change log arguments in cherry buffers");
    goto(&mut app, "+ ");
    app.press("RET").unwrap();
    assert_eq!(app.mode(), &BufferMode::Revision { rev: b, files: vec![] });
    app.press("q").unwrap();
    assert!(matches!(app.mode(), BufferMode::Cherry { .. }));
}

#[test]
fn shortlog() {
    let (_td, g) = merge_repo();
    let author = g.git_string(&["log", "-1", "--format=%aN"]).unwrap();
    let mut app = App::new(g.clone());
    app.press("l").unwrap();
    app.press("s").unwrap();
    assert_eq!(app.active_switches(), ["--numbered", "--summary"]);
    app.script_answers(&["HEAD~2"]);
    app.press("s").unwrap();
    let (title, content) = app.text_view().unwrap();
    assert_eq!(title, "*magit-shortlog*");
    assert_eq!(content.trim(), format!("3\t{author}"));
    app.press("q").unwrap();
    // without --summary, for a range
    app.press("l").unwrap();
    app.press("s").unwrap();
    app.press("-").unwrap();
    app.press("s").unwrap();
    app.script_answers(&["HEAD~1..HEAD"]);
    app.press("r").unwrap();
    let (_, content) = app.text_view().unwrap();
    assert!(content.starts_with(&format!("{author} (2):")), "{content}");
    assert!(content.contains("Merge branch 'side'"), "{content}");
}

/// magit-log-refresh in the status buffer: only the "Arguments" group,
/// changing the buffer's log arguments (magit-status-mode's default
/// "-n256 --decorate"), which the log sections use.
#[test]
fn status_log_arguments() {
    let (_td, g) = merge_repo();
    let mut app = App::new(g.clone());
    goto(&mut app, "Recent commits");
    app.press("TAB").unwrap();
    let merge = format!("{} master v1 Merge branch 'side'", short(&g, "HEAD"));
    assert!(texts(&app).contains(&merge), "{:#?}", texts(&app));
    assert!(!texts(&app).iter().any(|t| t.trim() == "|\\"));
    app.press("L").unwrap();
    assert_eq!(app.active_switches(), ["--decorate", "-n256"]);
    for k in ["-", "r"] {
        app.press(k).unwrap();
    }
    assert_eq!(app.message(), "- r is undefined");
    for k in ["-", "g", "g"] {
        app.press(k).unwrap();
    }
    assert_eq!(app.current_menu(), None);
    let t = texts(&app);
    assert!(t.iter().any(|l| l.trim() == "|\\"), "{t:#?}");
    let h = short(&g, "HEAD");
    assert!(t.iter().any(|l| l.starts_with(&format!("{h} *"))), "{t:#?}");
    // the log buffer's arguments are not changed ...
    assert_eq!(app.log_args, ["--graph", "-n256", "--decorate"]);
    // ... and its L has all arguments
    log_current(&mut app);
    for k in ["L", "-", "r"] {
        app.press(k).unwrap();
    }
    assert!(app.active_switches().contains(&"--reverse".to_string()));
    app.press("C-g").unwrap();
    app.press("q").unwrap();
    // without a decoration, no refs
    for k in ["L", "-", "d", "-", "g", "g"] {
        app.press(k).unwrap();
    }
    let merge = format!("{} Merge branch 'side'", short(&g, "HEAD"));
    assert!(texts(&app).contains(&merge), "{:#?}", texts(&app));
}
