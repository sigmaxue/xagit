//! ui/diffbuf.rs — the diff, revision and stash buffers (magit-diff-mode,
//! magit-revision-mode and magit-stash-mode), made of the same kind of
//! sections as the status buffer: a diffstat, file sections (with the
//! `chmod` / `rename` pseudo-hunks) and hunk sections; revision buffers
//! also have the headers, the commit message and the notes.

use super::view::{hunk_ident, push_branch, Builder, LineKind, StatusBuffer};
use crate::git::diff::{self, FileDiff};
use crate::git::Git;

/// What a diff buffer shows (magit-buffer-diff-range, -typearg, -type and
/// -files of magit-diff-setup-buffer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSpec {
    pub range: Option<String>,
    pub typearg: Option<String>,
    /// magit-buffer-diff-type: "unstaged", "staged", "committed" or "undefined"
    pub typ: String,
    pub files: Vec<String>,
}

/// Width used to wrap the related-refs headers (the window width).
fn window_width() -> usize {
    crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80).max(20)
}

// ---------------------------------------------------------------- diff washing

/// magit-diff-wash-diffstat: the `--numstat --stat` part before the diffs.
struct DiffStat {
    heading: String,
    /// (file, text of its line)
    files: Vec<(String, String)>,
}

/// File name of a numstat line, with renames resolved to the new name.
fn numstat_file(name: &str) -> String {
    if let (Some(a), Some(b)) = (name.find('{'), name.find('}')) {
        if let Some(arrow) = name[a..b].find(" => ") {
            let new = &name[a + arrow + 4..b];
            let joined = format!("{}{}{}", &name[..a], new, &name[b + 1..]);
            return joined.replace("//", "/");
        }
    }
    match name.find(" => ") {
        Some(i) => name[i + 4..].to_string(),
        None => diff::unquote_path(name),
    }
}

fn parse_diffstat(pre: &[&str]) -> Option<DiffStat> {
    let summary = pre.iter().position(|l| {
        let t = l.trim_start();
        let n = t.chars().take_while(char::is_ascii_digit).count();
        n > 0 && (t[n..].starts_with(" file changed") || t[n..].starts_with(" files changed"))
    })?;
    let heading = pre[summary].strip_prefix(' ').unwrap_or(pre[summary]).to_string();
    let mut names = Vec::new();
    let mut files = Vec::new();
    for l in &pre[..summary] {
        let parts: Vec<&str> = l.splitn(3, '\t').collect();
        let is_num = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '-');
        if parts.len() == 3 && is_num(parts[0]) && is_num(parts[1]) {
            names.push(numstat_file(parts[2]));
            continue;
        }
        // magit-diff-statline-re: " FILE | CNT +++--"
        let body = l.strip_prefix(' ').unwrap_or(l);
        let Some(bar) = body.rfind(" | ") else { continue };
        let file_part = body[..bar].trim_end();
        let sep = &body[file_part.len()..bar + 3];
        let rest = &body[bar + 3..];
        let rest = rest.trim_start();
        let lead = &body[bar + 3..bar + 3 + (body.len() - bar - 3 - rest.len())];
        let text = if rest.starts_with("Bin") {
            format!("{file_part}{sep}{lead}{rest} ")
        } else {
            let (cnt, graph) = rest.split_once(' ').unwrap_or((rest, ""));
            format!("{file_part}{sep}{lead}{cnt} {graph}")
        };
        let file = if names.is_empty() { file_part.to_string() } else { names.remove(0) };
        files.push((file, text));
    }
    Some(DiffStat { heading, files })
}

/// Insert the sections of a diff (magit-diff-wash-diffs) at DEPTH for diff
/// group GROUP and return its files. REVISION: combined diffs are shown as
/// "resolved" (magit-revision-mode).
fn insert_diff(b: &mut Builder, group: &str, text: &str, depth: usize, revision: bool, stat: bool) -> Vec<FileDiff> {
    let lines: Vec<&str> = text.lines().collect();
    let first = lines
        .iter()
        .position(|l| l.starts_with("diff --git ") || l.starts_with("diff --cc ") || l.starts_with("diff --combined "))
        .unwrap_or(lines.len());
    if stat {
        if let Some(ds) = parse_diffstat(&lines[..first]) {
            let id = format!("{group}/diffstat");
            b.heading_hide(ds.heading, LineKind::Info, &id, depth, true, false);
            for (file, text) in ds.files {
                let fid = format!("{group}/diffstat/{file}");
                b.line(text, LineKind::File { section: group.into(), path: file }, &fid, true, depth + 1);
            }
            b.empty(depth + 1);
        }
    }
    let mut files = diff::parse(&lines[first..].join("\n"));
    if revision {
        for f in files.iter_mut().filter(|f| f.combined) {
            f.status = "resolved".into();
        }
    }
    for f in &files {
        insert_file_section(b, group, f, depth);
    }
    if !files.is_empty() {
        b.empty(depth);
    }
    files
}

/// magit-diff-insert-file-section: hidden only for deleted files (outside
/// the status buffer).
fn insert_file_section(b: &mut Builder, group: &str, f: &FileDiff, depth: usize) {
    let mut text = match &f.orig {
        Some(orig) if *orig != f.path => format!("{:<11}{orig} -> {}", f.status, f.path),
        _ => format!("{:<11}{}", f.status, f.path),
    };
    if f.binary {
        text.push_str(" (binary)");
    }
    let id = format!("{group}/{}", f.path);
    let has_body = !f.hunks.is_empty() || f.modes.is_some() || f.rename.is_some();
    let fk = LineKind::File { section: group.into(), path: f.path.clone() };
    b.heading_hide(text, fk, &id, depth, has_body, f.status == "deleted");
    for (kind, meta) in [("chmod", &f.modes), ("rename", &f.rename)] {
        let Some(meta) = meta else { continue };
        let mk = LineKind::Meta { section: group.into(), path: f.path.clone(), kind: kind.into() };
        let mid = format!("{id}/{kind}");
        let mut it = meta.lines();
        b.line(it.next().unwrap_or("").to_string(), mk.clone(), &mid, true, depth + 1);
        for l in it {
            b.heading_cont(l.to_string(), mk.clone(), &mid, depth + 1);
        }
    }
    for (hi, h) in f.hunks.iter().enumerate() {
        let hid = hunk_ident(group, &f.path, &h.header);
        let hk = LineKind::Hunk { section: group.into(), path: f.path.clone(), hunk: hi };
        b.heading_hide(h.header.clone(), hk, &hid, depth + 1, !h.lines.is_empty(), false);
        for (li, l) in h.lines.iter().enumerate() {
            let lk = LineKind::DiffLine { section: group.into(), path: f.path.clone(), hunk: hi, line: li };
            b.line(l.clone(), lk, &hid, false, depth + 1);
        }
    }
}

/// Arguments for the diff command: `--numstat` alongside `--stat`
/// (magit-insert-diff), and "--ita-visible-in-index" unless `-C` is used.
fn diff_command_args(args: &[String]) -> (Vec<String>, bool) {
    let stat = args.iter().any(|a| a == "--stat");
    let mut out = Vec::new();
    if stat {
        out.push("--numstat".to_string());
    }
    out.extend(args.iter().cloned());
    (out, stat)
}

fn files_suffix(files: &[String], one: &str, many: &str) -> String {
    match files {
        [] => String::new(),
        [f] => format!("{one}{f}"),
        fs => format!("{many}{}", fs.join(", ")),
    }
}

// ---------------------------------------------------------------- diff buffer

/// magit-diff-refresh-buffer: the header line and the diff.
pub fn build_diff(g: &Git, spec: &DiffSpec, args: &[String]) -> StatusBuffer {
    let title = diff_title(spec, args);
    let (extra, stat) = diff_command_args(args);
    let mut a: Vec<&str> = vec!["diff"];
    if let Some(r) = &spec.range {
        a.push(r);
    }
    a.push("-p");
    if let Some(t) = &spec.typearg {
        a.push(t);
    }
    a.extend(extra.iter().map(String::as_str));
    if !args.iter().any(|x| x.starts_with("-C")) {
        a.push("--ita-visible-in-index");
    }
    a.push("--");
    a.extend(spec.files.iter().map(String::as_str));
    let out = g.diff_output(&a).unwrap_or_default();
    let mut b = Builder::new();
    let files = insert_diff(&mut b, &spec.typ, &out, 0, false, stat);
    trim_trailing_empty(&mut b);
    StatusBuffer::new(b.lines, vec![(spec.typ.clone(), files)], title)
}

/// The header line of a diff buffer.
pub fn diff_title(spec: &DiffSpec, args: &[String]) -> String {
    if spec.typearg.as_deref() == Some("--no-index") {
        let a = spec.files.first().map_or("", String::as_str);
        let b = spec.files.get(1).map_or("", String::as_str);
        return format!("Differences between {a} and {b}");
    }
    let head = match &spec.range {
        Some(r) if r.contains("..") || r.contains("^-") => format!("Changes in {r}"),
        Some(r) => {
            let end = if spec.typearg.as_deref() == Some("--cached") { "index" } else { "working tree" };
            if args.iter().any(|a| a == "-R") {
                format!("Changes from {end} to {r}")
            } else {
                format!("Changes from {r} to {end}")
            }
        }
        None if spec.typearg.as_deref() == Some("--cached") => "Staged changes".into(),
        None => "Unstaged changes".into(),
    };
    format!("{head}{}", files_suffix(&spec.files, " in file ", " in files "))
}

fn trim_trailing_empty(b: &mut Builder) {
    while b.lines.last().is_some_and(|l| l.text.is_empty() && l.kind == LineKind::Info) {
        b.lines.pop();
    }
}

// ---------------------------------------------------------------- revision buffer

/// magit-format-ref-labels for `%D` of `--decorate=full`.
pub fn format_ref_labels(g: &Git, decoration: &str) -> String {
    let current = g.git_string(&["symbolic-ref", "--short", "-q", "HEAD"]).ok().filter(|s| !s.is_empty());
    let target = current
        .as_ref()
        .and_then(|c| g.git_string(&["rev-parse", "--abbrev-ref", &format!("{c}@{{upstream}}")]).ok())
        .filter(|s| !s.is_empty());
    let (mut state, mut head, mut upstream) = (None, None, None);
    let (mut tags, mut branches, mut remotes, mut other, mut combined) = (vec![], vec![], vec![], vec![], vec![]);
    for r in decoration.split(", ").filter(|r| !r.is_empty()) {
        let r = r.strip_prefix("tag: ").unwrap_or(r);
        let r = r.strip_prefix("HEAD -> ").unwrap_or(r);
        if r == "HEAD" {
            head = Some("@".to_string());
        } else if let Some(t) = r.strip_prefix("refs/tags/") {
            tags.push(t.to_string());
        } else if let Some(b) = r.strip_prefix("refs/heads/") {
            branches.push(b.to_string());
        } else if let Some(b) = r.strip_prefix("refs/remotes/") {
            remotes.push(b.to_string());
        } else if let Some(b) = r.strip_prefix("refs/bisect/") {
            state = Some(b.split('-').next().unwrap_or(b).to_string());
        } else {
            other.push(r.to_string());
        }
    }
    remotes.retain(|name| !matches!(name.split_once('/'), Some((_, "HEAD"))));
    for name in branches {
        let mut prefix = String::new();
        if let Some(push) = push_branch(g, &name).filter(|p| remotes.contains(p)) {
            remotes.retain(|r| *r != push);
            prefix = format!("{}/", push.split('/').next().unwrap_or(""));
        }
        if current.as_deref() == Some(name.as_str()) {
            head = Some(format!("{prefix}{name}"));
        } else if target.as_deref() == Some(name.as_str()) {
            upstream = Some(format!("{prefix}{name}"));
        } else {
            combined.push(format!("{prefix}{name}"));
        }
    }
    if let (None, Some(t)) = (&upstream, &target) {
        if remotes.contains(t) {
            remotes.retain(|r| r != t);
            upstream = Some(t.clone());
        } else if combined.contains(t) {
            combined.retain(|r| r != t);
            upstream = Some(t.clone());
        }
    }
    let mut all: Vec<String> = Vec::new();
    all.extend(state);
    all.extend(head);
    all.extend(upstream);
    all.extend(tags);
    all.extend(combined);
    all.extend(remotes);
    all.extend(other);
    all.join(" ")
}

/// magit-get-current-tag REV with distance.
fn current_tag_of(g: &Git, rev: &str) -> Option<(String, usize)> {
    let s = g.git_string(&["describe", "--long", "--tags", rev]).ok()?;
    let (rest, _) = s.trim().rsplit_once("-g")?;
    let (tag, n) = rest.rsplit_once('-')?;
    Some((tag.to_string(), n.parse().unwrap_or(0)))
}

/// magit-get-next-tag REV with distance.
fn next_tag_of(g: &Git, rev: &str) -> Option<(String, usize)> {
    let s = g.git_string(&["describe", "--contains", rev]).ok()?;
    let tag: String = s.trim().chars().take_while(|c| *c != '~' && *c != '^').collect();
    if tag.is_empty() || current_tag_of(g, rev).is_some_and(|(t, _)| t == tag) {
        return None;
    }
    let n = g
        .git_string(&["rev-list", "--count", &format!("{rev}..{tag}")])
        .ok()
        .and_then(|c| c.trim().parse().ok())
        .unwrap_or(0);
    Some((tag, n))
}

/// magit-list-related-branches RELATION REV (local branches).
fn related_branches(g: &Git, relation: &str, rev: &str) -> Vec<String> {
    g.git_string(&["branch", relation, rev])
        .unwrap_or_default()
        .lines()
        .map(|l| l.get(2..).unwrap_or("").to_string())
        .filter(|b| !b.is_empty() && !b.starts_with("(HEAD") && !b.starts_with("HEAD -> "))
        .collect()
}

/// magit--insert-related-refs: "TITLE:" padded to 10 columns, then the
/// branches, wrapped at the window width with an indentation of 12.
fn insert_related_refs(b: &mut Builder, title: &str, refs: &[String]) {
    if refs.is_empty() {
        return;
    }
    let width = window_width();
    let mut rows = vec![format!("{title}:{}", " ".repeat(10usize.saturating_sub(title.len())))];
    for r in refs {
        let col = rows.last().map_or(0, |l| l.chars().count());
        if col + 1 + r.chars().count() <= width {
            if let Some(last) = rows.last_mut() {
                last.push(' ');
                last.push_str(r);
            }
        } else {
            rows.push(format!("{}{r}", " ".repeat(12)));
        }
    }
    let id = format!("headers/related-refs:{title}");
    let mut it = rows.into_iter();
    b.line(it.next().unwrap_or_default(), LineKind::Info, &id, true, 1);
    for r in it {
        b.heading_cont(r, LineKind::Info, &id, 1);
    }
}

/// `git show --no-patch --format=FORMAT REV` without trimming newlines.
fn rev_format(g: &Git, format: &str, rev: &str) -> String {
    g.git_string(&["show", "--no-patch", "--no-color", &format!("--format={format}"), rev, "--"]).unwrap_or_default()
}

/// magit-revision-refresh-buffer: tag (not shown), headers, message, notes, diff.
pub fn build_revision(g: &Git, rev: &str, args: &[String], files: &[String]) -> StatusBuffer {
    let typ = g.git_string(&["cat-file", "-t", rev]).unwrap_or_else(|_| "commit".into());
    let title = format!("{} {rev}{}", typ.trim(), files_suffix(files, " limited to file ", " limited to files "));
    let commit = format!("{rev}^{{commit}}");
    let mut b = Builder::new();

    // magit-insert-revision-headers
    let full = g.git_string(&["rev-parse", &commit]).unwrap_or_else(|_| rev.to_string());
    let deco = g
        .git_string(&["show", "--no-patch", "--decorate=full", "--format=%D", &commit, "--"])
        .unwrap_or_default();
    let labels = format_ref_labels(g, deco.trim());
    let heading = if labels.is_empty() { full.clone() } else { format!("{labels} {full}") };
    b.heading_hide(heading, LineKind::Info, "headers", 0, true, false);
    let fmt = "Author:     %aN <%aE>%nAuthorDate: %ad%nCommit:     %cN <%cE>%nCommitDate: %cd";
    for l in rev_format(g, fmt, &commit).lines() {
        b.line(l.to_string(), LineKind::Info, "headers", false, 0);
    }
    let parents = g.git_string(&["rev-list", "-1", "--parents", &commit]).unwrap_or_default();
    for p in parents.split_whitespace().skip(1) {
        let line = rev_format(g, "%h %s", p);
        let (h, s) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let id = format!("headers/commit:{p}");
        b.line(format!("Parent:     {h} {s}"), LineKind::Commit { section: "parent".into(), hash: p.to_string() }, &id, true, 1);
    }
    insert_related_refs(&mut b, "Merged", &related_branches(g, "--merged", &commit));
    insert_related_refs(&mut b, "Contained", &related_branches(g, "--contains", &commit));
    if let Some((tag, n)) = current_tag_of(g, &commit) {
        let id = format!("headers/tag:{tag}");
        b.line(format!("Follows:    {tag} ({n})"), LineKind::Commit { section: "tag".into(), hash: tag.clone() }, &id, true, 1);
    }
    if let Some((tag, n)) = next_tag_of(g, &commit) {
        let id = format!("headers/tag:{tag}");
        b.line(format!("Precedes:   {tag} ({n})"), LineKind::Commit { section: "tag".into(), hash: tag.clone() }, &id, true, 1);
    }
    b.empty(1);

    // magit-insert-revision-message
    let msg = g
        .git_output(&["show", "--no-patch", "--no-color", "--format=%B", &commit, "--"])
        .map(|o| String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n"))
        .unwrap_or_default();
    // git prints "\n" for an empty message, which Magit inserts as is
    if msg.is_empty() {
        b.heading_hide("(no message)".into(), LineKind::Info, "commit-message", 0, false, false);
    } else {
        let mut lines: Vec<&str> = msg.split('\n').collect();
        if lines.last() == Some(&"") {
            lines.pop();
        }
        let has_body = lines.len() > 1;
        b.heading_hide(lines[0].to_string(), LineKind::Info, "commit-message", 0, has_body, false);
        for l in &lines[1..] {
            b.line(l.to_string(), LineKind::Info, "commit-message", false, 0);
        }
    }

    // magit-insert-revision-notes (the default notes ref)
    if let Ok(notes) = g.git_string(&["notes", "show", &commit]) {
        let mut it = notes.lines();
        let first = format!("{} (commits)", it.next().unwrap_or(""));
        let id = "notes:refs/notes/commits";
        b.heading_hide(first, LineKind::Info, id, 0, true, false);
        for l in it {
            b.line(l.to_string(), LineKind::Info, id, false, 0);
        }
        b.empty(1);
    }

    // magit-insert-revision-diff
    let (extra, stat) = diff_command_args(args);
    let mut a: Vec<&str> = vec!["show", "-p", "--format="];
    a.extend(extra.iter().map(String::as_str));
    a.push(&commit);
    a.push("--");
    a.extend(files.iter().map(String::as_str));
    let out = g.diff_output(&a).unwrap_or_default();
    let diffs = insert_diff(&mut b, "committed", &out, 0, true, stat);
    trim_trailing_empty(&mut b);
    StatusBuffer::new(b.lines, vec![("committed".into(), diffs)], title)
}

// ---------------------------------------------------------------- stash buffer

/// magit-stash-refresh-buffer: notes, then the worktree, index and
/// untracked parts of the stash as commit sections with their diffs.
pub fn build_stash(g: &Git, stash: &str, args: &[String]) -> StatusBuffer {
    let subject = rev_format(g, "%s", stash);
    let mut cap = stash.to_string();
    if let Some(f) = cap.get(..1) {
        cap = format!("{}{}", f.to_uppercase(), &cap[1..]);
    }
    let title = format!("{cap} {}", subject.trim());
    let mut b = Builder::new();
    let mut diffs = Vec::new();

    if let Ok(notes) = g.git_string(&["notes", "show", stash]) {
        if !notes.trim().is_empty() {
            b.heading_hide("Notes".into(), LineKind::Info, "note", 0, true, false);
            for l in notes.lines() {
                b.line(l.to_string(), LineKind::Info, "note", false, 0);
            }
            b.empty(1);
        }
    }
    let (extra, _) = diff_command_args(&args.iter().filter(|a| *a != "--stat").cloned().collect::<Vec<_>>());
    let parts = [
        ("stash-worktree", stash.to_string(), format!("{stash}^2..{stash}"), "Unstaged"),
        ("stash-index", format!("{stash}^2"), format!("{stash}^..{stash}^2"), "Staged"),
        ("stash-untracked", format!("{stash}^3"), format!("{stash}^..{stash}^3"), "Untracked files"),
    ];
    for (group, rev, range, message) in parts {
        let mut paths = Vec::new();
        if group == "stash-untracked" {
            if !g.git_ok(&["rev-parse", "--verify", "--quiet", &rev]) {
                continue;
            }
            paths = g.git_lines(&["ls-tree", "-z", "--name-only", "-r", "--full-tree", &rev]);
        }
        let mut a: Vec<&str> = vec!["diff", &range, "-p"];
        a.extend(extra.iter().map(String::as_str));
        a.push("--");
        a.extend(paths.iter().map(String::as_str));
        let out = g.diff_output(&a).unwrap_or_default();
        // magit--git-wash cancels the section when git printed nothing
        if out.trim().is_empty() {
            continue;
        }
        let id = format!("commit:{rev}");
        let has_body = out.lines().any(|l| l.starts_with("diff --"));
        b.heading_hide(message.into(), LineKind::Commit { section: "stash".into(), hash: rev.clone() }, &id, 0, has_body, false);
        let files = insert_diff(&mut b, group, &out, 1, false, false);
        diffs.push((group.to_string(), files));
    }
    trim_trailing_empty(&mut b);
    StatusBuffer::new(b.lines, diffs, title)
}
