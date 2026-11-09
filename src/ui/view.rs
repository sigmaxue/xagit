//! ui/view.rs — buffer rendering mirroring magit-status buffer layout.

use crate::git::status::Status;
use crate::git::Git;

/// A line in the status buffer, with its section identity.
#[derive(Debug, Clone, PartialEq)]
pub enum LineKind {
    Head,
    SectionHeader { section: String, n: usize }, // e.g. section "Unstaged changes"
    File { section: String, path: String },
    Stash { idx: usize, name: String, msg: String },
    Info,
}

#[derive(Debug, Clone)]
pub struct BufferLine {
    pub text: String,
    pub kind: LineKind,
}

/// Build the magit-status buffer lines (same ordering as magit-status-insert-status-headers
/// + sections: Unmerged > Untracked > Unstaged > Staged > Stashes).
pub fn render_status(g: &Git, st: &Status) -> Vec<BufferLine> {
    let mut lines = Vec::new();

    lines.push(BufferLine {
        text: format!("Head: {}", g.head_desc()),
        kind: LineKind::Head,
    });
    if let Some(up) = &st.upstream {
        let ab = match (st.ahead, st.behind) {
            (0, 0) => String::new(),
            (a, 0) => format!(" (ahead {a})"),
            (0, b) => format!(" (behind {b})"),
            (a, b) => format!(" (ahead {a}, behind {b})"),
        };
        lines.push(BufferLine {
            text: format!("Push: {up}{ab}"),
            kind: LineKind::Head,
        });
    }
    if st.merge_in_progress {
        lines.push(BufferLine {
            text: "Merge: in progress".into(),
            kind: LineKind::Info,
        });
    }
    if st.rebase_in_progress {
        lines.push(BufferLine {
            text: "Rebase: in progress".into(),
            kind: LineKind::Info,
        });
    }

    push_files(&mut lines, "Untracked files", &st.untracked, "untracked");
    push_files(&mut lines, "Unstaged changes", &st.unstaged, "unstaged");
    push_files(&mut lines, "Staged changes", &st.staged, "staged");

    if !st.stashes.is_empty() {
        lines.push(BufferLine {
            text: format!("Stashes: {}", st.stashes.len()),
            kind: LineKind::SectionHeader { section: "stashes".into(), n: st.stashes.len() },
        });
        for (i, (name, msg)) in st.stashes.iter().enumerate() {
            lines.push(BufferLine {
                text: format!("  {name}: {msg}"),
                kind: LineKind::Stash { idx: i, name: name.clone(), msg: msg.clone() },
            });
        }
    }

    if lines.iter().all(|l| !matches!(l.kind, LineKind::SectionHeader { .. } | LineKind::File { .. } | LineKind::Stash { .. }))
        && st.is_clean()
        {
        lines.push(BufferLine {
            text: "Nothing to commit (working tree clean)".into(),
            kind: LineKind::Info,
        });
        // Magit also shows "No commits yet" for empty repos
        if g.no_commit() {
            lines.push(BufferLine {
                text: "No commits yet".into(),
                kind: LineKind::Info,
            });
        }
    }

    lines
}

fn push_files(lines: &mut Vec<BufferLine>, title: &str, files: &[crate::git::status::FileEntry], section: &str) {
    if files.is_empty() {
        return;
    }
    lines.push(BufferLine {
        text: format!("{title}: {}", files.len()),
        kind: LineKind::SectionHeader { section: section.into(), n: files.len() },
    });
    for f in files {
        let text = if f.is_unmerged() {
            format!("  unmerged   {}{}", f.path, f.conflict_desc())
        } else if let Some(orig) = &f.orig_path {
            format!("  {orig} -> {}", f.path)
        } else {
            format!("  {}", f.path)
        };
        lines.push(BufferLine {
            text,
            kind: LineKind::File { section: section.into(), path: f.path.clone() },
        });
    }
}

/// Current mode line shown in the footer.
pub fn modeline(app_name: &str, repo: &str, msg: &str) -> String {
    format!("{app_name} {repo}   {msg}")
}
