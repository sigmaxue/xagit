//! ui/logbuf.rs — the log and reflog buffers (magit-log-mode and
//! magit-reflog-mode): one commit section per line of `git log` /
//! `git reflog`, washed like magit-log-wash-rev, with the author and age
//! of each commit in the right margin.

use super::diffbuf::{insert_diff, RefLabeler};
use super::view::{Builder, LineKind, StatusBuffer};
use crate::git::diff::FileDiff;
use crate::git::Git;

/// AUTHOR-WIDTH of magit-log-margin.
const AUTHOR_WIDTH: usize = 18;
/// magit-log-margin-width for magit-log-margin '(t age ... t 18).
pub const LOG_MARGIN_WIDTH: usize = AUTHOR_WIDTH + 1 + 2 + 1 + 1 + 7;
/// magit-log-margin-width for magit-reflog-margin (no author).
pub const REFLOG_MARGIN_WIDTH: usize = 2 + 1 + 1 + 7;
/// magit-reflog-limit
pub const REFLOG_LIMIT: usize = 256;
/// The default commit limit set by `=` and `+` / `-` without a limit.
const DEFAULT_LIMIT: usize = 256;

/// magit--age-spec: (unit, units, seconds)
const AGE_SPEC: [(&str, &str, f64); 7] = [
    ("year", "years", 31_556_952.0),
    ("month", "months", 2_629_746.0),
    ("week", "weeks", 604_800.0),
    ("day", "days", 86_400.0),
    ("hour", "hours", 3_600.0),
    ("minute", "minutes", 60.0),
    ("second", "seconds", 1.0),
];

/// magit--age: the age of something AGE seconds old as (count, unit).
pub fn age(age: f64) -> (i64, &'static str) {
    let age = age.abs();
    for (i, (unit, units, weight)) in AGE_SPEC.iter().enumerate() {
        if i == AGE_SPEC.len() - 1 || age / weight >= 1.0 {
            let cnt = (age / weight).round_ties_even() as i64;
            return (cnt, if cnt == 1 { unit } else { units });
        }
    }
    unreachable!()
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn truncate(s: &str, width: usize) -> String {
    s.chars().take(width).collect()
}

/// The age part of the margin: "%2d %-Ns", N being what is left of WIDTH.
fn age_margin(date: &str, now: f64, width: usize) -> String {
    let date: f64 = date.split_whitespace().next().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    let (cnt, unit) = age(now - date);
    format!("{cnt:>2} {unit:<w$}", w = width)
}

/// magit-log-format-author-margin: the author (padded or truncated to
/// AUTHOR-WIDTH with an ellipsis) and the age, cut to the margin width.
pub fn author_margin(author: &str, date: &str, now: f64) -> String {
    let author = if author.chars().count() > AUTHOR_WIDTH {
        format!("{}…", truncate(author, AUTHOR_WIDTH - 1))
    } else {
        format!("{author:<AUTHOR_WIDTH$}")
    };
    let rest = LOG_MARGIN_WIDTH - (AUTHOR_WIDTH + 1);
    truncate(&format!("{author} {}", age_margin(date, now, rest)), LOG_MARGIN_WIDTH)
}

/// magit-log-get-commit-limit
pub fn commit_limit(args: &[String]) -> Option<usize> {
    args.iter().find_map(|a| a.strip_prefix("-n").filter(|n| n.chars().all(|c| c.is_ascii_digit()))?.parse().ok())
}

/// How `=`, `+` and `-` change the limit (magit-log-set-commit-limit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Toggle,
    Double,
    Half,
}

/// magit-log-set-commit-limit: without a limit set it to 256, else apply
/// FN; a limit of zero (or none) removes "-nN".
pub fn set_commit_limit(args: &[String], f: Limit) -> Vec<String> {
    let num = match commit_limit(args) {
        Some(n) => match f {
            Limit::Toggle => None,
            Limit::Double => Some(n * 2),
            Limit::Half => Some(n / 2),
        },
        None => Some(DEFAULT_LIMIT),
    };
    let is_limit = |a: &String| a.strip_prefix("-n").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()));
    let mut out: Vec<String> = args.iter().filter(|a| !is_limit(a)).cloned().collect();
    if let Some(n) = num.filter(|n| *n > 0) {
        out.insert(0, format!("-n{n}"));
    }
    out
}

/// magit-log-header-line-sentence
pub fn log_title(revs: &[String], args: &[String], files: &[String]) -> String {
    let mut t = format!("Commits in {}", revs.join(" "));
    if args.iter().any(|a| a == "--reverse") {
        t.push_str(" in reverse");
    }
    if !files.is_empty() {
        t.push_str(&format!(" touching {}", files.join(" ")));
    }
    if let Some(l) = args.iter().find(|a| a.starts_with("-L")) {
        t.push_str(&format!(" {l}"));
    }
    t
}

const GRAPH_CHARS: &str = "-_/|\\*o<>. ";

/// A line of magit-log-heading-re: graph, hash, refs, author, date, msg.
struct LogLine<'a> {
    graph: &'a str,
    hash: &'a str,
    refs: &'a str,
    author: &'a str,
    date: &'a str,
    msg: &'a str,
}

fn parse_log_line(line: &str) -> Option<LogLine<'_>> {
    let n = line.len() - line.trim_start_matches(|c| GRAPH_CHARS.contains(c)).len();
    let (graph, rest) = line.split_at(n);
    let f: Vec<&str> = rest.splitn(7, '\x0c').collect();
    if f.len() < 7 || !f[0].chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(LogLine { graph, hash: f[0], refs: f[1], author: f[3], date: f[4], msg: f[6] })
}

/// The arguments git log is run with (magit-log-refresh-buffer and
/// magit--insert-log), the format first.
fn log_command_args(args: &[String], files: &[String]) -> Vec<String> {
    let mut args = args.to_vec();
    if files.len() != 1 {
        args.retain(|a| a != "--follow");
    }
    // magit-log-remove-graph-args
    if args.iter().any(|a| ["--follow", "-G", "-S", "-L"].iter().any(|p| a.starts_with(p))) {
        args.retain(|a| a != "--graph");
    }
    // the graph is not colored (magit-log--maybe-drop-color-graph)
    args.retain(|a| a != "--color");
    let has = |a: &[String], x: &str| a.iter().any(|y| y == x);
    let format = format!(
        "--format={}%h%x0c{}%x0c%x0c%aN%x0c%at%x0c%x0c%s",
        if has(&args, "--left-right") && !has(&args, "--graph") { "%m " } else { "" },
        if has(&args, "--decorate") { "%D" } else { "" },
    );
    if has(&args, "--decorate") {
        args.retain(|a| a != "--decorate");
        args.insert(0, "--decorate=full".into());
    }
    if has(&args, "--reverse") {
        args.retain(|a| a != "--graph");
    }
    let mut out = vec![format];
    out.extend(args);
    out
}

/// magit-log-refresh-buffer: the commits of REVS touching FILES.
pub fn build_log(g: &Git, revs: &[String], args: &[String], files: &[String]) -> StatusBuffer {
    let title = log_title(revs, args, files);
    let mut a: Vec<String> = vec!["log".into()];
    let cmd = log_command_args(args, files);
    let align = !cmd.iter().any(|x| x == "--stat");
    a.extend(cmd);
    a.extend(["--use-mailmap".into(), "--no-prefix".into()]);
    a.extend(revs.iter().cloned());
    a.push("--".into());
    a.extend(files.iter().cloned());
    let a: Vec<&str> = a.iter().map(String::as_str).collect();
    let mut b = Builder::new();
    let mut diffs = Vec::new();
    let out = match g.git_output(&a) {
        Ok(o) if !o.status.success() && o.stdout.is_empty() => {
            // keep-error: show what git said
            for l in String::from_utf8_lossy(&o.stderr).lines() {
                b.line(l.to_string(), LineKind::Info, "error", false, 0);
            }
            String::new()
        }
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    };
    let count = wash_log(&mut b, g, &out, align, &mut diffs);
    if count > 0 && Some(count) == commit_limit(args) {
        b.line("Type + to show more history".into(), LineKind::Info, "longer", true, 0);
    }
    let mut buf = StatusBuffer::new(b.lines, diffs, title);
    buf.margin_width = LOG_MARGIN_WIDTH;
    buf
}

/// magit-log-wash-log for the `log` style; returns the number of commits.
fn wash_log(b: &mut Builder, g: &Git, out: &str, align: bool, diffs: &mut Vec<(String, Vec<FileDiff>)>) -> usize {
    let lines: Vec<&str> = out.lines().collect();
    let mut labels = RefLabeler::new(g);
    let now = now();
    let mut count = 0;
    let mut i = usize::from(lines.first().is_some_and(|l| l.starts_with("...")));
    while i < lines.len() {
        let Some(c) = parse_log_line(lines[i]) else {
            i += 1;
            continue;
        };
        count += 1;
        i += 1;
        let start = i;
        while i < lines.len() && parse_log_line(lines[i]).is_none() {
            i += 1;
        }
        let cont = &lines[start..i];

        let mut text = String::new();
        if align {
            text.push_str(c.hash);
            text.push(' ');
        }
        text.push_str(c.graph);
        if !align {
            text.push_str(c.hash);
            text.push(' ');
        }
        if !c.refs.is_empty() {
            let refs = labels.format(g, c.refs);
            if !refs.is_empty() {
                text.push_str(&refs);
                text.push(' ');
            }
        }
        text.push_str(c.msg);
        let margin = author_margin(c.author, c.date, now);
        let kind = LineKind::Commit { section: "log".into(), hash: c.hash.to_string() };
        let id = format!("commit:{}", c.hash);

        let is_diff = cont.first().is_some_and(|l| {
            l.starts_with("---")
                || (l.is_empty() && cont.get(1).is_some_and(|n| n.starts_with(' ') || n.starts_with("diff")))
        });
        if is_diff {
            // the diffstat and diffs become the body of the commit section
            b.heading_hide(text, kind, &id, 0, true, false);
            b.margin(margin);
            let body = cont[1..].join("\n");
            let files = insert_diff(b, c.hash, &body, 1, false, true);
            diffs.push((c.hash.to_string(), files));
            continue;
        }
        b.line(text, kind.clone(), &id, true, 0);
        b.margin(margin);
        if cont.is_empty() {
            continue;
        }
        // graph-only lines are part of the commit's heading
        let pad = if align { " ".repeat(c.hash.len() + 1) } else { String::new() };
        let mut last = String::new();
        for l in cont {
            last = format!("{pad}{l}");
            b.heading_cont(last.clone(), kind.clone(), &id, 0);
        }
        let graph: String = last.chars().take_while(|c| GRAPH_CHARS.contains(*c)).collect();
        if !graph.contains(['/', '\\', '.']) {
            b.heading_cont(graph, kind.clone(), &id, 0);
        }
    }
    count
}

// ---------------------------------------------------------------- reflog

/// magit-reflog-format-subject: the reflog command, its options and type,
/// padded to 16 columns.
pub fn reflog_subject(subject: &str) -> String {
    let mut words = subject.splitn(2, ' ');
    let command = words.next().unwrap_or("");
    let mut rest = words.next().unwrap_or("").trim_start();
    let mut options = Vec::new();
    while rest.starts_with('-') {
        let (o, r) = rest.split_once(' ').unwrap_or((rest, ""));
        options.push(o);
        rest = r.trim_start();
    }
    let typ = rest.strip_prefix('(').and_then(|r| r.split_once(')')).map(|(t, _)| t);
    let text = if command == "commit" {
        typ.unwrap_or(command).to_string()
    } else {
        let mut parts = vec![command.to_string()];
        if !options.is_empty() {
            parts.push(options.join(" "));
        }
        parts.extend(typ.map(str::to_string));
        parts.join(" ")
    };
    format!("{text:<16} ")
}

/// The "refsub" of a reflog subject (magit-log-reflog-re): "merge ",
/// "autosave ", "restart ", "rewritten " or "COMMAND: ".
fn split_refsub(gs: &str) -> (Option<&str>, &str) {
    for p in ["merge ", "autosave ", "restart ", "rewritten "] {
        if let Some(rest) = gs.strip_prefix(p) {
            return (Some(&gs[..p.len()]), rest);
        }
    }
    match gs.find(':') {
        Some(i) if i > 0 && gs[i + 1..].starts_with(' ') => (Some(&gs[..i + 2]), &gs[i + 2..]),
        _ => (None, gs),
    }
}

/// magit-reflog-refresh-buffer
pub fn build_reflog(g: &Git, refname: &str, args: &[String]) -> StatusBuffer {
    let title = format!("Reflog for {refname}");
    let mut a: Vec<&str> = vec!["reflog", "show", "--format=%h%x00%aN%x00%gd%x00%gs", "--date=raw"];
    a.extend(args.iter().map(String::as_str));
    a.push(refname);
    a.push("--");
    let out = g.git_output(&a).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let now = now();
    let mut b = Builder::new();
    let mut count = 0;
    for line in out.lines() {
        count += 1;
        let f: Vec<&str> = line.splitn(4, '\0').collect();
        let (hash, gd, gs) = (f[0], f.get(2).copied().unwrap_or(""), f.get(3).copied().unwrap_or(""));
        // entries whose date is gone (pruned reflogs) are dropped
        let Some(date) = gd.split_once("@{").and_then(|(_, d)| d.strip_suffix('}')) else { continue };
        let (refsub, msg) = split_refsub(gs);
        let mut text = format!("{hash} {:<2} ", count - 1);
        if let Some(r) = refsub {
            let cut = if r.contains(':') { r.len() - 2 } else { r.len() - 1 };
            text.push_str(&reflog_subject(&r[..cut]));
        }
        text.push_str(msg);
        let kind = LineKind::Commit { section: "reflog".into(), hash: hash.to_string() };
        b.line(text, kind, &format!("commit:{hash}:{}", count - 1), true, 0);
        b.margin(truncate(&age_margin(date, now, REFLOG_MARGIN_WIDTH), REFLOG_MARGIN_WIDTH));
    }
    if count > 0 && Some(count) == commit_limit(args) {
        b.line("Type + to show more history".into(), LineKind::Info, "longer", true, 0);
    }
    let mut buf = StatusBuffer::new(b.lines, Vec::new(), title);
    buf.margin_width = REFLOG_MARGIN_WIDTH;
    buf
}
