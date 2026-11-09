//! ui/blame.rs — file and blob buffers (a visited file, magit-blob-mode)
//! and magit-blame: the chunks of `git blame --incremental`, shown in the
//! styles of magit-blame-styles.

use super::logbuf::format_time;
use super::view::{Builder, LineKind, StatusBuffer};
use crate::git::Git;
use anyhow::{bail, Result};
use std::collections::HashMap;

/// magit-blame-type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlameType {
    /// magit-blame-addition: the commits that added the lines
    Addition,
    /// magit-blame-removal: the commits that removed them
    Removal,
    /// magit-blame-reverse: the last commits that still have them
    Final,
}

/// The blame of a file or blob buffer (magit-blame-mode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blame {
    pub typ: BlameType,
    /// index into STYLES (magit-blame--style)
    pub style: usize,
    /// the buffer was created by a recursive blame (magit-blame-recursive-p)
    pub recursive: bool,
    /// the arguments of the blame (magit-blame-arguments)
    pub args: Vec<String>,
}

/// The identifiers of magit-blame-styles, cycled by `c`.
pub const STYLES: [&str; 3] = ["headings", "highlight", "lines"];
/// heading-format of the headings style
const HEADING_FORMAT: &str = "%-20a %C %s";
/// magit-blame-time-format
const TIME_FORMAT: &str = "%F %H:%M";
/// magit-blame-separator: the thin line before each chunk (lines style)
pub const SEPARATOR: &str = "────────────────────────────────────────";

/// magit-blame-chunk
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chunk {
    pub orig_rev: String,
    pub orig_file: String,
    pub orig_line: usize,
    pub final_line: usize,
    pub num_lines: usize,
    pub prev_rev: Option<String>,
    pub prev_file: Option<String>,
    /// summary, author, author-time, committer, committer-time...
    pub info: HashMap<String, String>,
}

impl Chunk {
    fn uncommitted(&self) -> bool {
        self.orig_rev.len() >= 40 && self.orig_rev.chars().all(|c| c == '0')
    }

    /// magit-blame--format-string-1 for FORMAT (%H %h %s %a %c %A %C).
    pub fn format(&self, format: &str) -> String {
        if self.uncommitted() {
            return "Not Yet Committed".into();
        }
        let get = |k: &str| self.info.get(k).cloned().unwrap_or_default();
        let time = |k: &str| format_time(TIME_FORMAT, get(k).parse().unwrap_or(0));
        let mut out = String::new();
        let mut it = format.chars().peekable();
        while let Some(c) = it.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            // %[-]WIDTH
            let mut spec = String::new();
            while let Some(&d) = it.peek() {
                if d == '-' || d.is_ascii_digit() {
                    spec.push(d);
                    it.next();
                } else {
                    break;
                }
            }
            let v = match it.next() {
                Some('H') => self.orig_rev.clone(),
                Some('h') => self.orig_rev.chars().take(7).collect(),
                Some('s') => get("summary"),
                Some('a') => get("author"),
                Some('c') => get("committer"),
                Some('A') => time("author-time"),
                Some('C') => time("committer-time"),
                Some('%') => "%".into(),
                Some(o) => format!("%{spec}{o}"),
                None => String::new(),
            };
            let width: usize = spec.trim_start_matches('-').parse().unwrap_or(0);
            let pad = " ".repeat(width.saturating_sub(v.chars().count()));
            if spec.starts_with('-') {
                out.push_str(&v);
                out.push_str(&pad);
            } else {
                out.push_str(&pad);
                out.push_str(&v);
            }
        }
        out
    }

    /// The heading of the chunk in the headings style.
    pub fn heading(&self) -> String {
        self.format(HEADING_FORMAT)
    }
}

/// magit-blame--commit-alist
fn commit_alist(g: &Git, rev: &str) -> HashMap<String, String> {
    let out = g
        .git_string(&["log", "-1", "--no-walk", "--format=%s\x0b%an\x0b%ad\x0b%cn\x0b%cd", "--date=format:%s\x0b%z", rev, "--"])
        .unwrap_or_default();
    let keys = ["summary", "author", "author-time", "author-tz", "committer", "committer-time", "committer-tz"];
    keys.iter().map(|k| k.to_string()).zip(out.trim_end_matches('\n').split('\x0b').map(String::from)).collect()
}

/// magit-blame-run-process + magit-blame--parse-chunk: the chunks of
/// FILE at REV (none: the worktree) in final-line order, adjacent chunks
/// of the same commit merged (magit-blame--make-overlays).
pub fn run_blame(g: &Git, rev: Option<&str>, file: &str, args: &[String], typ: BlameType) -> Result<Vec<Chunk>> {
    let mut a: Vec<&str> = vec!["blame", "--incremental"];
    if typ != BlameType::Addition {
        a.push("--reverse");
    }
    a.extend(args.iter().map(String::as_str));
    a.extend(rev);
    a.extend(["--", file]);
    let out = g.git_output(&a)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{}", err.lines().next().unwrap_or("git blame failed"));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut cache: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut lines = text.lines();
    while let Some(head) = lines.next() {
        let f: Vec<&str> = head.split(' ').collect();
        if f.len() < 4 || f[0].len() < 40 {
            bail!("Blaming failed due to unexpected output: {head}");
        }
        let num = |s: &str| s.parse::<usize>().unwrap_or(0);
        let mut c = Chunk {
            orig_rev: f[0].into(),
            orig_line: num(f[1]),
            final_line: num(f[2]),
            num_lines: num(f[3]),
            ..Chunk::default()
        };
        let mut info = HashMap::new();
        for l in lines.by_ref() {
            if let Some(name) = l.strip_prefix("filename ") {
                c.orig_file = name.into();
                break;
            }
            if let Some(p) = l.strip_prefix("previous ") {
                if let Some((r, f)) = p.split_once(' ') {
                    c.prev_rev = Some(r.into());
                    c.prev_file = Some(f.into());
                }
            } else if let Some((k, v)) = l.split_once(' ') {
                info.insert(k.to_string(), v.to_string());
            }
        }
        if typ == BlameType::Removal {
            if let (Some(pr), Some(pf)) = (c.prev_rev.take(), c.prev_file.take()) {
                c.prev_rev = Some(std::mem::replace(&mut c.orig_rev, pr));
                c.prev_file = Some(std::mem::replace(&mut c.orig_file, pf));
                info.clear();
            }
        }
        let info = if info.is_empty() {
            cache.entry(c.orig_rev.clone()).or_insert_with(|| commit_alist(g, &c.orig_rev)).clone()
        } else {
            cache.insert(c.orig_rev.clone(), info.clone());
            info
        };
        c.info = info;
        chunks.push(c);
    }
    chunks.sort_by_key(|c| c.final_line);
    let mut merged: Vec<Chunk> = Vec::new();
    for c in chunks {
        match merged.last_mut() {
            Some(m) if m.orig_rev == c.orig_rev && m.final_line + m.num_lines == c.final_line => {
                m.num_lines += c.num_lines;
            }
            _ => merged.push(c),
        }
    }
    Ok(merged)
}

/// The contents of FILE at REV: "{worktree}", "{index}" or a commit.
pub fn file_contents(g: &Git, rev: &str, file: &str) -> Result<String> {
    match rev {
        "{worktree}" => match std::fs::read(g.toplevel.join(file)) {
            Ok(b) => Ok(String::from_utf8_lossy(&b).into_owned()),
            Err(_) => bail!("Cannot visit {file}: file does not exist"),
        },
        "{index}" => g.git_string(&["cat-file", "-p", &format!(":{file}")]),
        _ => g.git_string(&["cat-file", "-p", &format!("{rev}:{file}")]),
    }
}

/// The name of the buffer visiting FILE at REV (magit-find-file).
pub fn buffer_name(rev: &str, file: &str) -> String {
    match rev {
        "{worktree}" => file.to_string(),
        _ => format!("{file}.~{rev}~"),
    }
}

fn staged(g: &Git, file: &str) -> bool {
    !g.git_ok(&["diff", "--cached", "--quiet", "--", file])
}

/// `git log --format=%h --name-only --follow` pairs of (rev, file).
fn file_log(g: &Git, extra: &[&str], rev: &str, file: &str) -> Vec<(String, String)> {
    let mut a = vec!["log"];
    a.extend(extra);
    a.extend(["--format=%h", "--name-only", "--follow", rev, "--", file]);
    let out = g.git_string(&a).unwrap_or_default();
    let lines: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
    lines.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].to_string(), c[1].to_string())).collect()
}

/// magit-blob-ancestor: the blob before FILE at REV.
pub fn blob_ancestor(g: &Git, rev: &str, file: &str) -> Option<(String, String)> {
    match rev {
        "{worktree}" if staged(g, file) => Some(("{index}".into(), file.into())),
        "{worktree}" | "{index}" => g.git_string(&["rev-parse", "--short", "HEAD"]).ok().map(|h| (h, file.into())),
        _ => file_log(g, &["-2"], rev, file).into_iter().nth(1),
    }
}

/// magit-blob-successor: the blob after FILE at REV.
pub fn blob_successor(g: &Git, rev: &str, file: &str) -> Option<(String, String)> {
    match rev {
        "{worktree}" => None,
        "{index}" => Some(("{worktree}".into(), file.into())),
        _ => {
            let log = file_log(g, &[], "HEAD", file);
            let found = log.iter().position(|(r, _)| r == rev).and_then(|i| i.checked_sub(1)).map(|i| log[i].clone());
            Some(found.unwrap_or_else(|| {
                let r = if staged(g, file) { "{index}" } else { "{worktree}" };
                (r.into(), file.into())
            }))
        }
    }
}

/// The lines of FILE at REV, with the chunks of BLAME (if any) shown in
/// its style: a heading before each chunk (headings), nothing but the
/// chunks (highlight), or a thin line before each chunk (lines). A blame
/// that fails is shown in the title.
pub fn build_file(g: &Git, rev: &str, file: &str, blame: Option<&Blame>) -> StatusBuffer {
    let content = file_contents(g, rev, file).unwrap_or_default();
    let mut b = Builder::new();
    let mut error = None;
    let (style, chunks) = match blame {
        Some(bl) => {
            let r = (rev != "{worktree}" && rev != "{index}").then_some(rev);
            match run_blame(g, r, file, &bl.args, bl.typ) {
                Ok(ch) => (Some(STYLES[bl.style]), ch),
                Err(e) => {
                    error = Some(e.to_string());
                    (None, Vec::new())
                }
            }
        }
        None => (None, Vec::new()),
    };
    let chunk_of = |line: usize| chunks.iter().position(|c| line >= c.final_line && line < c.final_line + c.num_lines);
    for (i, text) in content.lines().enumerate() {
        let line = i + 1;
        let chunk = chunk_of(line);
        if let Some(ci) = chunk.filter(|&ci| chunks[ci].final_line == line) {
            match style {
                Some("headings") => b.line(chunks[ci].heading(), LineKind::BlameHeading { chunk: ci }, "", false, 0),
                Some("lines") => b.line(SEPARATOR.into(), LineKind::BlameHeading { chunk: ci }, "", false, 0),
                _ => {}
            }
        }
        b.line(text.to_string(), LineKind::FileLine { line, chunk }, "", false, 0);
    }
    let mut buf = StatusBuffer::new(b.lines, Vec::new(), buffer_name(rev, file));
    buf.blame_error = error;
    buf.blame = chunks;
    buf
}
