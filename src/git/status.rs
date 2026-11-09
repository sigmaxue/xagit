//! git/status.rs — status model mirroring magit-status buffer sections.

use super::exec::Git;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Untracked,
    Unstaged,
    Staged,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    /// short status code from `git status --porcelain -z`
    pub code: char,
    /// 'M' modified, 'A' added, 'D' deleted, 'R' renamed, 'C' copied, '?' untracked, '!' ignored
    pub x: char,
    pub y: char,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub branch: Option<String>,
    pub detached_at: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub untracked: Vec<FileEntry>,
    pub unstaged: Vec<FileEntry>,
    pub staged: Vec<FileEntry>,
    pub stashes: Vec<(String, String)>,
    pub merge_in_progress: bool,
    pub rebase_in_progress: bool,
}

impl Status {
    /// Parse `git status --porcelain=v1 -z --branch` like Magit does for its status buffer.
    pub fn load(g: &Git) -> Status {
        let mut st = Status::default();
        // branch header
        let header = g
            .git_string(&["status", "--porcelain=v1", "-z", "--branch"])
            .unwrap_or_default();
        let mut parts = header.split('\0');
        if let Some(first) = parts.next() {
            for line in first.lines() {
                if let Some(rest) = line.strip_prefix("## ") {
                    let mut it = rest.split("...");
                    let local = it.next().unwrap_or("").trim().to_string();
                    if local == "No commits yet" || local.starts_with("No branch") {
                        // detached or unborn
                        if local.starts_with("No branch") {
                            st.detached_at = Some(g.rev_short("HEAD"));
                        }
                    } else {
                        st.branch = Some(local.clone());
                        if let Some(remote) = it.next() {
                            let remote = remote
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            if !remote.is_empty() {
                                st.upstream = Some(remote);
                            }
                        }
                        // ahead/behind
                        for tag in rest.split_whitespace() {
                            if let Some(n) = tag.strip_prefix("[ahead ") {
                                st.ahead = n.trim_end_matches(']').parse().unwrap_or(0);
                            }
                            if let Some(n) = tag.strip_prefix("[behind ") {
                                st.behind = n.trim_end_matches(']').parse().unwrap_or(0);
                            }
                            if tag.starts_with("ahead") {
                                if let Some(n) = tag.split_whitespace().nth(1) {
                                    st.ahead = n.parse().unwrap_or(0);
                                }
                            }
                            if tag.starts_with("behind") {
                                if let Some(n) = tag.split_whitespace().nth(1) {
                                    st.behind = n.parse().unwrap_or(0);
                                }
                            }
                        }
                    }
                }
            }
        }

        let porcelain = g
            .git_string(&["status", "--porcelain=v1", "-z"])
            .unwrap_or_default();
        let mut chunks = porcelain.split('\0').peekable();
        while let Some(chunk) = chunks.next() {
            if chunk.len() < 3 {
                continue;
            }
            let bytes = chunk.as_bytes();
            let x = bytes[0] as char;
            let y = bytes[1] as char;
            let path = chunk[3..].to_string();
            let mut path = path;
            // renames/copies come as "new\0old" when using -z? actually -z: "XY new\0old\0"
            if x == 'R' || x == 'C' {
                if let Some(old) = chunks.peek() {
                    // consume the old-path part
                    let oldp = old.to_string();
                    chunks.next();
                    path = format!("{oldp} -> {path}");
                }
            }
            let entry = FileEntry {
                path: path.clone(),
                code: y,
                x,
                y,
            };
            match (x, y) {
                ('?', _) => st.untracked.push(entry),
                (_, 'M') | (_, 'A') | (_, 'D') | (_, 'R') | (_, 'C') | (_, 'T') => {
                    // y != ' ' means staged? NO: x = index, y = worktree
                    // staged when x != ' ' && x != '?'
                    if x != ' ' && x != '?' && x != '!' {
                        st.staged.push(FileEntry { path: path.clone(), code: x, x, y });
                    }
                    if y != ' ' {
                        st.unstaged.push(FileEntry { path: path.clone(), code: y, x, y });
                    }
                }
                _ => {
                    if x != ' ' && x != '?' && x != '!' {
                        st.staged.push(FileEntry { path: path.clone(), code: x, x, y });
                    }
                    if y != ' ' && y != '?' && y != '!' {
                        st.unstaged.push(FileEntry { path: path.clone(), code: y, x, y });
                    }
                }
            }
        }

        st.stashes = g.stashes();
        st.merge_in_progress = g.merge_in_progress();
        st.rebase_in_progress = g.rebase_in_progress();
        if st.branch.is_none() && st.detached_at.is_none() {
            // unborn HEAD (no commits yet): branch from symbolic-ref
            if let Some(b) = g.current_branch() {
                st.branch = Some(b);
            }
        }
        st
    }

    /// Format the Magit-style status header lines:
    ///   "Head: branch main"
    ///   "Push: origin/main"
    pub fn head_line(&self) -> String {
        match (&self.branch, &self.detached_at) {
            (Some(b), _) => format!("Head: branch {b}"),
            (None, Some(d)) => format!("Head: detached at {d}"),
            (None, None) => "Head: unborn".into(),
        }
    }

    pub fn upstream_line(&self) -> String {
        let ahead = self.ahead;
        let behind = self.behind;
        match &self.upstream {
            Some(u) => format!(
                "Push: {u}{}",
                match (ahead, behind) {
                    (0, 0) => String::new(),
                    (a, 0) => format!(" (ahead {a})"),
                    (0, b) => format!(" (behind {b})"),
                    (a, b) => format!(" (ahead {a}, behind {b})"),
                }
            ),
            None => String::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        self.untracked.is_empty() && self.unstaged.is_empty() && self.staged.is_empty()
    }
}
