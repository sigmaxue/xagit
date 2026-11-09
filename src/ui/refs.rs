//! ui/refs.rs — the references buffer (magit-refs-mode, `y`): the local
//! branches, the branches of each remote and the tags, each compared with
//! the focused ref (magit-buffer-refs-upstream) and with the commits it
//! has that the focused ref doesn't (magit-refs--insert-cherry-commits)
//! as its body.

use super::logbuf::{cherry_commits, now, Margin};
use super::view::{Builder, LineKind, StatusBuffer, Visibility};
use crate::git::Git;

/// magit-refs-show-commit-count
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommitCount {
    /// nil: never
    #[default]
    Nothing,
    /// branch (t): for branches only
    Branches,
    /// all: for branches and tags
    All,
}

/// magit-refs-primary-column-width
const PRIMARY_MIN: usize = 16;
const PRIMARY_MAX: usize = 32;
/// magit-refs-focus-column-width
const FOCUS_WIDTH: usize = 5;

/// A ref's kind, as the type of its section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

impl RefKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RefKind::Local => "local",
            RefKind::Remote => "remote",
            RefKind::Tag => "tag",
        }
    }
}

/// The identity of the section of the ref NAME.
pub fn ref_ident(kind: RefKind, name: &str) -> String {
    match kind {
        RefKind::Tag => format!("tag:{name}"),
        _ => format!("branch:{name}"),
    }
}

/// The ref whose cherry commits the body of the section IDENT lists.
fn ident_ref(ident: &str) -> Option<&str> {
    ident.strip_prefix("branch:").or_else(|| ident.strip_prefix("tag:"))
}

struct Refs<'a> {
    g: &'a Git,
    /// magit-buffer-refs-upstream
    focus: &'a str,
    /// the commit counts shown (none if FOCUS isn't a revision)
    counts: CommitCount,
    width: usize,
    vis: &'a Visibility,
    margin: &'a Margin,
    now: f64,
    b: Builder,
    /// ref sections whose body isn't inserted (because they are hidden)
    deferred: Vec<String>,
}

/// magit-refs-refresh-buffer: the sections of magit-refs-sections-hook.
pub fn build_refs(
    g: &Git,
    focus: &str,
    args: &[String],
    counts: CommitCount,
    vis: &Visibility,
    margin: &Margin,
) -> StatusBuffer {
    let counts = if g.rev_verify(focus) { counts } else { CommitCount::Nothing };
    let mut r = Refs { g, focus, counts, width: PRIMARY_MIN, vis, margin, now: now(), b: Builder::new(), deferred: Vec::new() };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    r.branch_description();
    r.local_branches(&args);
    r.remote_branches(&args);
    r.tags(&args);
    let deferred = std::mem::take(&mut r.deferred);
    let mut buf = StatusBuffer::new(r.b.lines, Vec::new(), format!("{focus} {}", args.join(" ")));
    buf.margin_width = margin.shown_width();
    buf.deferred = deferred;
    buf
}

/// Split a `for-each-ref` line on NULs, empty fields being None.
fn fields(line: &str) -> Vec<Option<&str>> {
    line.split('\0').map(|f| (!f.is_empty()).then_some(f)).collect()
}

/// The counts of `[ahead N, behind M]` (%(upstream:track)).
fn track_count(track: &str, what: &str) -> Option<String> {
    let rest = &track[track.find(what)? + what.len()..];
    let n: String = rest.trim_start().chars().take_while(char::is_ascii_digit).collect();
    (!n.is_empty()).then_some(n)
}

/// The line of a local branch: its name and heading parts.
struct Local {
    branch: Option<String>,
    current: bool,
    pretty: String,
    ahead: String,
    behind: String,
    upstream: String,
    msg: String,
}

impl Refs<'_> {
    /// magit-refs--format-focus-column: "@" (focus HEAD) or "*" at the
    /// focused ref, else maybe how many commits REF is ahead ("<N") or
    /// behind ("N>") of it.
    fn focus_column(&self, r: Option<&str>, current: bool, tag: bool) -> String {
        let width = if self.counts == CommitCount::Nothing { 1 } else { FOCUS_WIDTH };
        let text = if r == Some(self.focus) || (current && self.focus == "HEAD") {
            let mark = if self.focus == "HEAD" { "@" } else { "*" };
            format!("{mark}{}", " ".repeat(width - 1))
        } else if r.is_some() && (if tag { self.counts == CommitCount::All } else { self.counts != CommitCount::Nothing }) {
            let range = format!("{}...{}", self.focus, r.unwrap_or_default());
            let out = self.g.git_string(&["rev-list", "--count", "--left-right", &range]).unwrap_or_default();
            let mut it = out.split_whitespace().map(|n| n.parse::<usize>().unwrap_or(0));
            let (behind, ahead) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
            if ahead > 0 {
                format!("<{ahead}")
            } else if behind > 0 {
                format!("{behind}>")
            } else {
                "=".into()
            }
        } else {
            String::new()
        };
        format!("{text:>width$} ")
    }

    /// The padding after a primary column of TEXT.
    fn pad(&self, len: usize) -> String {
        " ".repeat(self.width.saturating_sub(len).max(1))
    }

    /// magit-refs--maybe-format-margin
    fn ref_margin(&mut self, rev: &str) {
        if !self.margin.enable {
            return;
        }
        let line = self.g.git_string(&["log", "-1", "--no-walk", "--format=%cN%x00%ct", rev, "--"]).unwrap_or_default();
        let (name, date) = line.split_once('\0').unwrap_or((&line, ""));
        let m = self.margin.author(name, date, self.now);
        self.b.margin(m);
    }

    /// A ref section (hidden initially) whose body lists the cherry
    /// commits of REF, inserted only once the section is shown
    /// (magit-insert-section-body).
    /// MARGIN: with the margin of REF (not for tags:
    /// magit-refs-margin-for-tags is nil).
    fn ref_section(&mut self, ident: &str, heading: String, kind: LineKind, r: &str, margin: bool) {
        self.b.heading_hide(heading, kind, ident, 1, true, true);
        if margin {
            self.ref_margin(r);
        }
        if self.vis.get(ident) != Some(&false) {
            self.deferred.push(ident.to_string());
            return;
        }
        let cherries = cherry_commits(self.g, self.focus, r, self.margin.enable);
        for c in cherries {
            let kind = LineKind::Commit { section: "cherry".into(), hash: c.hash.clone() };
            self.b.line(format!("{} {} {}", c.mark, c.hash, c.msg), kind, &format!("{ident}/commit:{}", c.hash), true, 2);
            if let Some((author, date)) = &c.ident {
                let m = self.margin.author(author, date, self.now);
                self.b.margin(m);
            }
        }
    }

    /// magit-insert-branch-description
    fn branch_description(&mut self) {
        let Some(branch) = self.g.current_branch() else { return };
        let Ok(desc) = self.g.git_string(&["config", &format!("branch.{branch}.description")]) else { return };
        if desc.is_empty() {
            return;
        }
        let mut lines = desc.lines();
        let first = lines.next().unwrap_or_default();
        let rest: Vec<&str> = lines.collect();
        let ident = "branchdesc";
        self.b.heading_hide(format!("{branch}: {first}"), LineKind::Head, ident, 0, !rest.is_empty(), false);
        if !rest.is_empty() {
            for l in rest {
                self.b.line(l.to_string(), LineKind::Info, ident, false, 0);
            }
            self.b.line(String::new(), LineKind::Info, ident, false, 0);
        }
    }

    /// magit-refs--format-local-branch
    fn format_local(&self, line: &str) -> Local {
        let f = fields(line);
        let get = |i: usize| f.get(i).copied().flatten();
        let current = get(0) == Some("*");
        let branch = get(1).map(str::to_string);
        let track = get(5).unwrap_or_default();
        Local {
            pretty: branch.clone().unwrap_or_else(|| "(detached)".into()),
            current,
            ahead: track_count(track, "ahead").map(|n| format!("{n}>")).unwrap_or_default(),
            behind: track_count(track, "behind").map(|n| format!("<{n}")).unwrap_or_default(),
            upstream: get(3).map(|u| format!("{u} ")).unwrap_or_default(),
            msg: get(9).unwrap_or_default().to_string(),
            branch,
        }
    }

    /// magit-insert-local-branches
    fn local_branches(&mut self, args: &[&str]) {
        let mut a = vec![
            "for-each-ref",
            "--format=%(HEAD)%00%(refname:short)%00%(refname)%00%(upstream:short)%00%(upstream)%00%(upstream:track)%00%00%00%00%(subject)",
            "refs/heads",
        ];
        a.extend(args);
        let mut lines: Vec<Local> = self.g.git_lines_pub(&a).iter().map(|l| self.format_local(l)).collect();
        if self.g.current_branch().is_none() {
            let subject = self.g.git_string(&["log", "-1", "--no-walk", "--format=%s", "HEAD", "--"]).unwrap_or_default();
            lines.insert(0, self.format_local(&format!("*\0\0\0\0\0\0\0\0\0{subject}")));
        }
        let longest = lines.iter().map(|l| 1 + l.pretty.len() + l.ahead.len() + l.behind.len()).max().unwrap_or(0);
        self.width = PRIMARY_MAX.min(PRIMARY_MIN.max(longest));
        let n = lines.len();
        let title = if n > 0 { format!("Branches ({n})") } else { "Branches".into() };
        self.b.heading_hide(title, LineKind::SectionHeader { section: "local".into(), n }, "local", 0, true, false);
        for l in lines {
            let focus = self.focus_column(l.branch.as_deref(), l.current, false);
            let pad = self.pad(l.pretty.len() + l.ahead.len() + l.behind.len());
            let heading = format!("{focus}{}{}{pad}{}{}{}", l.pretty, l.ahead, l.behind, l.upstream, l.msg);
            match &l.branch {
                Some(b) => {
                    let kind = LineKind::Ref { kind: RefKind::Local, name: b.clone() };
                    self.ref_section(&ref_ident(RefKind::Local, b), heading, kind, b, true);
                }
                // the detached HEAD: a commit section
                None => {
                    let hash = self.g.git_string(&["rev-parse", "HEAD"]).unwrap_or_default();
                    let kind = LineKind::Commit { section: "refs".into(), hash: hash.clone() };
                    self.b.line(heading, kind, &format!("commit:{hash}"), true, 1);
                    self.ref_margin("HEAD");
                }
            }
        }
        self.b.empty(1);
    }

    /// magit-insert-remote-branches
    fn remote_branches(&mut self, args: &[&str]) {
        for remote in self.g.remotes() {
            let get = |k: &str| self.g.git_string(&["config", &format!("remote.{remote}.{k}")]).ok().filter(|v| !v.is_empty());
            let urls: Vec<String> = [get("url"), get("pushurl")].into_iter().flatten().collect();
            let prefix = format!("refs/remotes/{remote}");
            let mut a = vec!["for-each-ref", "--format=%(symref:short)%00%(refname:short)%00%(refname)%00%(subject)", &prefix];
            a.extend(args);
            let head_ref = format!("refs/remotes/{remote}/HEAD");
            let branches: Vec<(String, String)> = self
                .g
                .git_lines_pub(&a)
                .iter()
                .filter_map(|l| {
                    let f = fields(l);
                    let get = |i: usize| f.get(i).copied().flatten();
                    // the remote HEAD (a symref), and misconfigured ones
                    if get(0).is_some() || get(2) == Some(head_ref.as_str()) {
                        return None;
                    }
                    Some((get(1)?.to_string(), get(3).unwrap_or_default().to_string()))
                })
                .collect();
            let ident = format!("remote:{remote}");
            let n = branches.len();
            let count = if n > 0 { format!(" ({n})") } else { ":".into() };
            let heading = format!("Remote {remote} ({}){count}", urls.join(", "));
            self.b.heading_hide(heading, LineKind::Remote { name: remote.clone() }, &ident, 0, true, false);
            for (branch, msg) in branches {
                let abbrev = &branch[remote.len() + 1..];
                let focus = self.focus_column(Some(&branch), false, false);
                let heading = format!("{focus}{abbrev}{}{msg}", self.pad(abbrev.len()));
                let kind = LineKind::Ref { kind: RefKind::Remote, name: branch.clone() };
                self.ref_section(&ref_ident(RefKind::Remote, &branch), heading, kind, &branch, true);
            }
            self.b.empty(1);
        }
    }

    /// magit-insert-tags
    fn tags(&mut self, args: &[&str]) {
        let mut a = vec!["tag", "--list", "-n"];
        a.extend(args);
        let tags: Vec<(String, String)> = self
            .g
            .git_lines_pub(&a)
            .iter()
            .filter_map(|l| {
                let mut it = l.splitn(2, [' ', '\t']);
                let tag = it.next().filter(|t| !t.is_empty())?;
                Some((tag.to_string(), it.next().unwrap_or_default().trim_start().to_string()))
            })
            .collect();
        if tags.is_empty() {
            return;
        }
        let n = tags.len();
        self.b.heading_hide(format!("Tags ({n})"), LineKind::SectionHeader { section: "tags".into(), n }, "tags", 0, true, false);
        for (tag, msg) in tags {
            let focus = self.focus_column(Some(&tag), false, true);
            let heading = format!("{focus}{tag}{}{msg}", self.pad(tag.len()));
            let kind = LineKind::Ref { kind: RefKind::Tag, name: tag.clone() };
            self.ref_section(&ref_ident(RefKind::Tag, &tag), heading, kind, &tag, false);
        }
        self.b.empty(1);
    }
}

/// The ref of the deferred section IDENT, for "No cherries for REF".
pub fn deferred_ref(ident: &str) -> &str {
    ident_ref(ident).unwrap_or(ident)
}
