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
    /// source path of a rename/copy (`path` is always the current path)
    pub orig_path: Option<String>,
}

impl FileEntry {
    /// Unmerged (conflicted) entry: DD AU UD UA DU AA UU.
    pub fn is_unmerged(&self) -> bool {
        is_unmerged(self.x, self.y)
    }

    /// Suffix Magit shows after "unmerged   <file>".
    pub fn conflict_desc(&self) -> &'static str {
        match (self.x, self.y) {
            ('D', 'D') => " (both deleted)",
            ('D', 'U') => " (deleted by us)",
            ('U', 'D') => " (deleted by them)",
            ('A', 'A') => " (both added)",
            ('A', 'U') => " (added by us)",
            ('U', 'A') => " (added by them)",
            _ => "",
        }
    }
}

fn is_unmerged(x: char, y: char) -> bool {
    matches!(
        (x, y),
        ('D', 'D') | ('A', 'U') | ('U', 'D') | ('U', 'A') | ('D', 'U') | ('A', 'A') | ('U', 'U')
    )
}

/// Parsed `## ...` header of `git status --porcelain=v1 --branch`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchHeader {
    pub branch: Option<String>,
    pub detached: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub gone: bool,
}

/// Parse the text after "## ", e.g. `main...origin/main [ahead 1, behind 2]`,
/// `No commits yet on main`, `HEAD (no branch)`.
pub fn parse_branch_header(rest: &str) -> BranchHeader {
    let mut h = BranchHeader::default();
    let rest = rest.trim();
    if let Some(b) = rest
        .strip_prefix("No commits yet on ")
        .or_else(|| rest.strip_prefix("Initial commit on "))
    {
        h.branch = Some(b.trim().to_string());
        return h;
    }
    if rest.starts_with("HEAD (no branch)") || rest.starts_with("No branch") {
        h.detached = true;
        return h;
    }
    let (refs, track) = match rest.find(" [") {
        Some(i) => (&rest[..i], Some(&rest[i + 2..])),
        None => (rest, None),
    };
    let mut it = refs.splitn(2, "...");
    let local = it.next().unwrap_or("").trim();
    if !local.is_empty() {
        h.branch = Some(local.to_string());
    }
    if let Some(up) = it.next().map(str::trim).filter(|u| !u.is_empty()) {
        h.upstream = Some(up.to_string());
    }
    if let Some(t) = track {
        for part in t.trim_end_matches(']').split(',') {
            let part = part.trim();
            if let Some(n) = part.strip_prefix("ahead ") {
                h.ahead = n.trim().parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                h.behind = n.trim().parse().unwrap_or(0);
            } else if part == "gone" {
                h.gone = true;
            }
        }
    }
    h
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
        if let Some(rest) = parts.next().and_then(|first| first.strip_prefix("## ")) {
            let h = parse_branch_header(rest);
            if h.detached {
                st.detached_at = Some(g.rev_short("HEAD"));
            }
            st.branch = h.branch;
            st.upstream = h.upstream;
            st.ahead = h.ahead;
            st.behind = h.behind;
        }

        let porcelain = g
            .git_string(&["status", "--porcelain=v1", "-z"])
            .unwrap_or_default();
        let mut chunks = porcelain.split('\0');
        while let Some(chunk) = chunks.next() {
            if chunk.len() < 3 {
                continue;
            }
            let bytes = chunk.as_bytes();
            let x = bytes[0] as char;
            let y = bytes[1] as char;
            let path = chunk[3..].to_string();
            // -z format: "XY <path>\0<orig>\0" when X or Y is R/C
            let orig_path = if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
                chunks.next().map(str::to_string)
            } else {
                None
            };
            let entry = |code: char| FileEntry {
                path: path.clone(),
                code,
                x,
                y,
                orig_path: orig_path.clone(),
            };
            if x == '?' {
                st.untracked.push(entry('?'));
            } else if is_unmerged(x, y) {
                // Magit lists conflicts once, in the unstaged section ("unmerged   file")
                st.unstaged.push(entry('U'));
            } else {
                if x != ' ' && x != '!' {
                    st.staged.push(entry(x));
                }
                if y != ' ' && y != '!' {
                    st.unstaged.push(entry(y));
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
