//! ui/logbuf.rs — the log and reflog buffers (magit-log-mode and
//! magit-reflog-mode): one commit section per line of `git log` /
//! `git reflog`, washed like magit-log-wash-rev, with the author and age
//! of each commit in the right margin.

use super::diffbuf::{insert_diff, RefLabeler};
use super::view::{head_header, upstream_header, Builder, LineKind, StatusBuffer};
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
/// The style magit-cycle-margin-style uses after age-abbreviated.
pub const DEFAULT_TIME_FORMAT: &str = "%Y-%m-%d %H:%M ";

/// magit--age-spec: (char, unit, units, seconds)
const AGE_SPEC: [(char, &str, &str, f64); 7] = [
    ('Y', "year", "years", 31_556_952.0),
    ('M', "month", "months", 2_629_746.0),
    ('w', "week", "weeks", 604_800.0),
    ('d', "day", "days", 86_400.0),
    ('h', "hour", "hours", 3_600.0),
    ('m', "minute", "minutes", 60.0),
    ('s', "second", "seconds", 1.0),
];

/// magit--age: the age of something AGE seconds old as (count, unit).
pub fn age(age: f64) -> (i64, &'static str) {
    let (cnt, i) = age_index(age);
    let (_, unit, units, _) = AGE_SPEC[i];
    (cnt, if cnt == 1 { unit } else { units })
}

fn age_index(age: f64) -> (i64, usize) {
    let age = age.abs();
    let i = AGE_SPEC.iter().position(|s| age / s.3 >= 1.0).unwrap_or(AGE_SPEC.len() - 1);
    ((age / AGE_SPEC[i].3).round_ties_even() as i64, i)
}

pub(crate) fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn truncate(s: &str, width: usize) -> String {
    s.chars().take(width).collect()
}

/// format-time-string: TIME (seconds since the epoch) in the local time
/// zone, formatted by strftime.
pub fn format_time(format: &str, time: i64) -> String {
    #[cfg(unix)]
    {
        let Ok(fmt) = std::ffi::CString::new(format) else { return String::new() };
        let t = time as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return String::new();
        }
        let mut buf = vec![0u8; 256 + 4 * format.len()];
        let n = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), &tm) };
        buf.truncate(n);
        String::from_utf8_lossy(&buf).into_owned()
    }
    #[cfg(not(unix))]
    {
        let _ = format;
        let (days, secs) = (time.div_euclid(86_400), time.rem_euclid(86_400));
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!("{y:04}-{m:02}-{d:02} {:02}:{:02} ", secs / 3600, secs / 60 % 60)
    }
}

/// The STYLE of a margin: the committer date's age, abbreviated or not,
/// or the date formatted with a format-time-string format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarginStyle {
    Age,
    AgeAbbreviated,
    Format(String),
}

/// magit--right-margin-config: (ENABLE STYLE WIDTH DETAILS DETAILS-WIDTH),
/// plus magit-log-margin-show-shortstat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Margin {
    pub enable: bool,
    pub style: MarginStyle,
    pub details: bool,
    pub details_width: usize,
    pub shortstat: bool,
}

impl Margin {
    /// magit-log-margin and magit-cherry-margin
    pub fn log() -> Self {
        Margin { enable: true, style: MarginStyle::Age, details: true, details_width: AUTHOR_WIDTH, shortstat: false }
    }

    /// magit-reflog-margin and magit-stashes-margin
    pub fn reflog() -> Self {
        Margin { details: false, ..Margin::log() }
    }

    /// magit-status-margin
    pub fn status() -> Self {
        Margin { enable: false, details: false, ..Margin::log() }
    }

    /// magit-log-margin-width
    pub fn width(&self) -> usize {
        if self.shortstat {
            return 16;
        }
        let details = if self.details { 1 + self.details_width } else { 0 };
        details
            + match &self.style {
                MarginStyle::Format(f) => format_time(f, now() as i64).chars().count(),
                MarginStyle::AgeAbbreviated => 2 + 1 + 1,
                MarginStyle::Age => {
                    let unit = AGE_SPEC.iter().map(|s| s.1.len().max(s.2.len())).max().unwrap_or(0);
                    2 + 1 + 1 + unit
                }
            }
    }

    /// The width of the margin shown: 0 when it is hidden.
    pub fn shown_width(&self) -> usize {
        if self.enable {
            self.width()
        } else {
            0
        }
    }

    /// magit-cycle-margin-style: age, age-abbreviated, the time format.
    pub fn cycle_style(&mut self) {
        self.style = match self.style {
            MarginStyle::Age => MarginStyle::AgeAbbreviated,
            MarginStyle::AgeAbbreviated => MarginStyle::Format(DEFAULT_TIME_FORMAT.into()),
            MarginStyle::Format(_) => MarginStyle::Age,
        };
    }

    /// magit-log-format-author-margin for AUTHOR and DATE (seconds since
    /// the epoch, maybe followed by a time zone), cut to the margin width.
    pub fn author(&self, author: &str, date: &str, now: f64) -> String {
        let width = self.width();
        let mut s = String::new();
        if self.details {
            let w = self.details_width;
            if author.chars().count() > w {
                s.push_str(&truncate(author, w.saturating_sub(1)));
                s.push('…');
            } else {
                s.push_str(&format!("{author:<w$}"));
            }
            s.push(' ');
        }
        let date: f64 = date.split_whitespace().next().and_then(|d| d.parse().ok()).unwrap_or(0.0);
        let rest = width - if self.details { 1 + self.details_width } else { 0 };
        match &self.style {
            MarginStyle::Format(f) => s.push_str(&format_time(f, date as i64)),
            style => {
                let (cnt, i) = age_index(now - date);
                if *style == MarginStyle::AgeAbbreviated {
                    s.push_str(&format!("{cnt:>2}{:<rest$}", AGE_SPEC[i].0));
                } else {
                    let unit = if cnt == 1 { AGE_SPEC[i].1 } else { AGE_SPEC[i].2 };
                    s.push_str(&format!("{cnt:>2} {unit:<rest$}"));
                }
            }
        }
        truncate(&s, width)
    }

    /// magit-log-format-margin for REV: its shortstat if that style is
    /// shown (SHORTSTATS, see `shortstats`), else the author and date.
    pub fn format(&self, rev: &str, author: &str, date: &str, now: f64, shortstats: &Shortstats) -> String {
        if self.shortstat {
            return shortstats.get(rev).cloned().unwrap_or_default();
        }
        self.author(author, date, now)
    }
}

/// The shortstat margins of some revisions.
pub type Shortstats = std::collections::HashMap<String, String>;

/// magit-log-format-shortstat-margin for each of REVS (only when MARGIN
/// shows them: this is slow): "%5s %5s%4s" of "N+", "N-" and the number
/// of files of `git show --format= --shortstat REV`.
pub fn shortstats(g: &Git, margin: &Margin, revs: &[&str]) -> Shortstats {
    let mut m = Shortstats::new();
    if !margin.enable || !margin.shortstat {
        return m;
    }
    for rev in revs {
        let line = g.git_string(&["show", "--format=", "--shortstat", rev]).unwrap_or_default();
        m.insert(rev.to_string(), format_shortstat(line.trim()));
    }
    m
}

/// "%5s %5s%4s" of a `--shortstat` line ("" if it is not one).
pub fn format_shortstat(line: &str) -> String {
    let mut files = None;
    let (mut add, mut del) = (String::new(), String::new());
    for part in line.split(", ") {
        let Some((n, what)) = part.split_once(' ') else { return String::new() };
        if n.parse::<u64>().is_err() {
            return String::new();
        }
        if what.starts_with("file") {
            files = Some(n);
        } else if what.starts_with("insertion") {
            add = format!("{n}+");
        } else if what.starts_with("deletion") {
            del = format!("{n}-");
        } else {
            return String::new();
        }
    }
    match files {
        Some(f) => format!("{add:>5} {del:>5}{f:>4}"),
        None => String::new(),
    }
}

/// magit-log-format-author-margin with magit-log-margin.
pub fn author_margin(author: &str, date: &str, now: f64) -> String {
    Margin::log().author(author, date, now)
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
pub fn build_log(g: &Git, revs: &[String], args: &[String], files: &[String], margin: &Margin) -> StatusBuffer {
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
    let count = wash_log(&mut b, g, &out, align, &mut diffs, margin);
    if count > 0 && Some(count) == commit_limit(args) {
        b.line("Type + to show more history".into(), LineKind::Info, "longer", true, 0);
    }
    let mut buf = StatusBuffer::new(b.lines, diffs, title);
    buf.margin_width = margin.shown_width();
    buf
}

/// magit-log-wash-log for the `log` style; returns the number of commits.
fn wash_log(
    b: &mut Builder,
    g: &Git,
    out: &str,
    align: bool,
    diffs: &mut Vec<(String, Vec<FileDiff>)>,
    margin: &Margin,
) -> usize {
    let lines: Vec<&str> = out.lines().collect();
    let revs: Vec<&str> = lines.iter().filter_map(|l| parse_log_line(l)).map(|c| c.hash).collect();
    let stats = shortstats(g, margin, &revs);
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
        let margin = margin.format(c.hash, c.author, c.date, now, &stats);
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
pub fn build_reflog(g: &Git, refname: &str, args: &[String], margin: &Margin) -> StatusBuffer {
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
        b.margin(margin.author(f.get(1).copied().unwrap_or(""), date, now));
    }
    if count > 0 && Some(count) == commit_limit(args) {
        b.line("Type + to show more history".into(), LineKind::Info, "longer", true, 0);
    }
    let mut buf = StatusBuffer::new(b.lines, Vec::new(), title);
    buf.margin_width = margin.shown_width();
    buf
}

// ---------------------------------------------------------------- stashes

/// magit-stashes-refresh-buffer (magit-stashes-mode): a "Stashes" section
/// with one section per entry of REFNAME's reflog.
pub fn build_stashes(g: &Git, refname: &str, margin: &Margin) -> StatusBuffer {
    let out = g
        .git_output(&["reflog", "--format=%gd%x00%aN%x00%at%x00%gs", refname])
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let entries: Vec<Vec<&str>> = out
        .lines()
        .map(|l| l.splitn(4, '\0').collect::<Vec<_>>())
        .filter(|f| f.len() == 4 && !f[0].is_empty())
        .collect();
    let title = if refname == "refs/stash" { "Stashes".to_string() } else { format!("Stashes [{refname}]") };
    let mut b = Builder::new();
    let n = entries.len();
    b.heading_hide(title, LineKind::SectionHeader { section: "stashesbuf".into(), n }, "stashesbuf", 0, n > 0, false);
    let now = now();
    for (i, f) in entries.iter().enumerate() {
        let kind = LineKind::Stash { idx: i, name: f[0].to_string(), msg: f[3].to_string() };
        b.line(format!("{} {}", f[0], f[3]), kind, &format!("stash:{}", f[0]), true, 1);
        b.margin(margin.author(f[1], f[2], now));
    }
    let mut buf = StatusBuffer::new(b.lines, Vec::new(), String::new());
    buf.margin_width = margin.shown_width();
    buf
}

// ---------------------------------------------------------------- cherry

/// magit-cherry-refresh-buffer (magit-cherry-mode): the headers of HEAD
/// and UPSTREAM, then the commits of `git cherry -v UPSTREAM HEAD`, newest
/// first, "+" for those not in UPSTREAM and "-" for equivalent ones.
pub fn build_cherry(g: &Git, head: &str, upstream: &str, margin: &Margin) -> StatusBuffer {
    let mut b = Builder::new();
    // magit-insert-cherry-headers
    b.line(head_header(g, Some(head)), LineKind::Head, "header:head", true, 0);
    if let Some(text) = upstream_header(g, head, Some(upstream), Some("Upstream: ")) {
        b.line(text, LineKind::Head, "header:upstream", true, 0);
    }
    b.root_blank();
    // magit-insert-cherry-commits
    let out = g.git_string(&["cherry", "-v", "--abbrev", upstream, head]).unwrap_or_default();
    let cherries: Vec<(&str, &str, &str)> = out
        .lines()
        .rev()
        .filter_map(|l| {
            let (mark, rest) = l.split_once(' ')?;
            let (hash, msg) = rest.split_once(' ').unwrap_or((rest, ""));
            matches!(mark, "+" | "-").then_some((mark, hash, msg))
        })
        .collect();
    let mut idents: Vec<(String, String)> = Vec::new();
    // only when the margin is shown (magit-log-wash-rev, cherry style)
    if !cherries.is_empty() && margin.enable {
        let mut a = vec!["log", "--no-walk=unsorted", "--format=%aN%x00%ct"];
        a.extend(cherries.iter().map(|c| c.1));
        if let Ok(o) = g.git_output(&a) {
            idents = String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| {
                    let (a, d) = l.split_once('\0').unwrap_or((l, ""));
                    (a.to_string(), d.to_string())
                })
                .collect();
        }
    }
    let n = cherries.len();
    b.heading_hide("Cherry commits".into(), LineKind::SectionHeader { section: "cherries".into(), n }, "cherries", 0, n > 0, false);
    let now = now();
    for (i, (mark, hash, msg)) in cherries.iter().enumerate() {
        let kind = LineKind::Commit { section: "cherry".into(), hash: hash.to_string() };
        b.line(format!("{mark} {hash} {msg}"), kind, &format!("commit:{hash}"), true, 1);
        if let Some((author, date)) = idents.get(i) {
            b.margin(margin.author(author, date, now));
        }
    }
    let mut buf = StatusBuffer::new(b.lines, Vec::new(), String::new());
    buf.margin_width = margin.shown_width();
    buf
}
