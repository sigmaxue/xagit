//! git/diff.rs — parsing `git diff --no-prefix` output into file and hunk
//! sections (magit-diff-wash-diffs) and building the patches Magit feeds to
//! `git apply` when staging, unstaging, discarding or reversing a file, hunk
//! or region (magit-apply.el).

/// One `@@ ... @@` hunk: the header line and the body lines (without "\n").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<String>,
}

/// One file section of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// current path (destination of a rename)
    pub path: String,
    /// source of a rename or copy
    pub orig: Option<String>,
    /// Magit's status word: "modified", "new file", "deleted", "renamed",
    /// "copied", "unmerged"
    pub status: String,
    /// everything from `diff --git` up to the first hunk, newline-terminated
    pub header: String,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
    /// `diff --cc` (conflict resolution): hunks cannot be applied one by one
    pub combined: bool,
    /// "old mode M\nnew mode N" (shown as a `chmod` pseudo-hunk)
    pub modes: Option<String>,
    /// "rename from A\nrename to B" (shown as a `rename` pseudo-hunk)
    pub rename: Option<String>,
}

/// Parsed `@@ -a,b +c,d @@` ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkRange {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
}

impl Hunk {
    /// Header and body as patch text, newline-terminated.
    pub fn text(&self) -> String {
        let mut s = String::with_capacity(self.header.len() + 1);
        s.push_str(&self.header);
        s.push('\n');
        for l in &self.lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    }

    pub fn range(&self) -> Option<HunkRange> {
        parse_hunk_header(&self.header).map(|(r, _)| r)
    }
}

impl FileDiff {
    /// Whole-file patch: header plus every hunk (magit-apply-diff).
    pub fn text(&self) -> String {
        let mut s = self.header.clone();
        for h in &self.hunks {
            s.push_str(&h.text());
        }
        s
    }

    /// magit-diff-file-header with NO-RENAME, as used for hunk patches: the
    /// header without the mode and rename lines, and `--- ORIG` replaced by
    /// `--- PATH`, so that a hunk applies to the file only.
    pub fn hunk_header(&self) -> String {
        let mut out = String::new();
        let dev_null = self.header.lines().any(|l| l == "--- /dev/null");
        for l in self.header.lines() {
            if ["old mode ", "new mode ", "rename from ", "rename to "].iter().any(|p| l.starts_with(p)) {
                continue;
            }
            if l.starts_with("--- ") && !dev_null && self.orig.is_some() {
                out.push_str("--- ");
                out.push_str(&quote_path(&self.path));
            } else {
                out.push_str(l);
            }
            out.push('\n');
        }
        out
    }

    /// The `diff --git` line plus the lines of a pseudo-hunk (mode change
    /// or rename), applying only that part of the change.
    fn meta_patch(&self, meta: &str, keep_similarity: bool) -> String {
        let mut out = String::new();
        for l in self.header.lines() {
            if l.starts_with("diff --git ") || (keep_similarity && l.starts_with("similarity index ")) {
                out.push_str(l);
                out.push('\n');
            }
        }
        out.push_str(meta);
        out.push('\n');
        out
    }
}

/// Quote a path like git does when it contains special characters.
fn quote_path(p: &str) -> String {
    if !p.bytes().any(|b| b < 0x20 || b == b'"' || b == b'\\' || b >= 0x80) {
        return p.to_string();
    }
    let mut s = String::from("\"");
    for b in p.bytes() {
        match b {
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            b if !(0x20..0x80).contains(&b) => s.push_str(&format!("\\{b:03o}")),
            b => s.push(b as char),
        }
    }
    s.push('"');
    s
}

/// Undo git's C-style quoting of a path (`"a\tb"`, `"\303\251"`).
pub fn unquote_path(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
        return s.to_string();
    };
    let mut bytes = Vec::with_capacity(inner.len());
    let mut it = inner.bytes().peekable();
    while let Some(b) = it.next() {
        if b != b'\\' {
            bytes.push(b);
            continue;
        }
        match it.next() {
            Some(b'n') => bytes.push(b'\n'),
            Some(b't') => bytes.push(b'\t'),
            Some(b'r') => bytes.push(b'\r'),
            Some(b'a') => bytes.push(7),
            Some(b'b') => bytes.push(8),
            Some(b'f') => bytes.push(12),
            Some(b'v') => bytes.push(11),
            Some(d @ b'0'..=b'7') => {
                let mut v = u32::from(d - b'0');
                for _ in 0..2 {
                    match it.peek() {
                        Some(&n @ b'0'..=b'7') => {
                            v = v * 8 + u32::from(n - b'0');
                            it.next();
                        }
                        _ => break,
                    }
                }
                bytes.push(v as u8);
            }
            Some(other) => bytes.push(other),
            None => bytes.push(b'\\'),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Path from a `--- x` / `+++ x` line (None for /dev/null).
fn header_path(rest: &str) -> Option<String> {
    let rest = rest.strip_suffix('\t').unwrap_or(rest);
    if rest == "/dev/null" {
        return None;
    }
    Some(unquote_path(rest))
}

/// Parse `@@ -a,b +c,d @@ rest`; also accepts combined `@@@ ... @@@` (ranges
/// of the first parent are returned).
pub fn parse_hunk_header(h: &str) -> Option<(HunkRange, String)> {
    let at = h.chars().take_while(|c| *c == '@').count();
    if at < 2 {
        return None;
    }
    let marker = "@".repeat(at);
    let body = h.get(at..)?.trim_start();
    let end = body.find(&marker)?;
    let ranges = body[..end].trim();
    let rest = body[end + at..].to_string();
    let mut old = None;
    let mut new = None;
    for part in ranges.split_whitespace() {
        let (sign, spec) = part.split_at(1);
        let mut it = spec.splitn(2, ',');
        let start: usize = it.next()?.parse().ok()?;
        let len: usize = match it.next() {
            Some(l) => l.parse().ok()?,
            None => 1,
        };
        match sign {
            "-" if old.is_none() => old = Some((start, len)),
            "+" => new = Some((start, len)),
            _ => {}
        }
    }
    let (old_start, old_len) = old?;
    let (new_start, new_len) = new?;
    Some((HunkRange { old_start, old_len, new_start, new_len }, rest))
}

fn format_range(start: usize, len: usize) -> String {
    if len == 1 {
        start.to_string()
    } else {
        format!("{start},{len}")
    }
}

/// Rebuild a two-way hunk header with new ranges, keeping the trailing text.
pub fn format_hunk_header(r: HunkRange, rest: &str) -> String {
    format!(
        "@@ -{} +{} @@{}",
        format_range(r.old_start, r.old_len),
        format_range(r.new_start, r.new_len),
        rest
    )
}

/// Parse the output of `git diff --no-prefix` (or `git show`, `git stash show -p`).
/// Text before the first `diff ` line (commit headers) is ignored.
pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut in_hunk = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") || line.starts_with("diff --cc ") || line.starts_with("diff --combined ") {
            let combined = !line.starts_with("diff --git ");
            let guess = if combined {
                unquote_path(line.splitn(3, ' ').nth(2).unwrap_or(""))
            } else {
                guess_git_path(&line["diff --git ".len()..])
            };
            files.push(FileDiff {
                path: guess,
                orig: None,
                status: if combined { "unmerged".into() } else { "modified".into() },
                header: format!("{line}\n"),
                hunks: Vec::new(),
                binary: false,
                combined,
                modes: None,
                rename: None,
            });
            in_hunk = false;
            continue;
        }
        let Some(f) = files.last_mut() else { continue };
        if line.starts_with("@@") {
            f.hunks.push(Hunk { header: line.to_string(), lines: Vec::new() });
            in_hunk = true;
            continue;
        }
        if in_hunk {
            if let Some(h) = f.hunks.last_mut() {
                if line.starts_with([' ', '+', '-', '\\']) || line.is_empty() {
                    // an empty line can only be a context line whose space was eaten
                    h.lines.push(if line.is_empty() { " ".to_string() } else { line.to_string() });
                    continue;
                }
            }
            in_hunk = false;
        }
        f.header.push_str(line);
        f.header.push('\n');
        if let Some(p) = line.strip_prefix("--- ") {
            if f.orig.is_none() && !f.combined {
                if let Some(p) = header_path(p) {
                    f.path = p;
                }
            }
        } else if let Some(p) = line.strip_prefix("+++ ") {
            if let Some(p) = header_path(p) {
                f.path = p;
            }
        } else if let Some(p) = line.strip_prefix("rename from ") {
            f.orig = Some(unquote_path(p));
            f.status = "renamed".into();
            f.rename = Some(line.to_string());
        } else if let Some(p) = line.strip_prefix("rename to ") {
            f.path = unquote_path(p);
            if let Some(r) = f.rename.as_mut() {
                r.push('\n');
                r.push_str(line);
            }
        } else if line.starts_with("old mode ") {
            f.modes = Some(line.to_string());
        } else if line.starts_with("new mode ") {
            if let Some(m) = f.modes.as_mut() {
                m.push('\n');
                m.push_str(line);
            }
        } else if let Some(p) = line.strip_prefix("copy from ") {
            f.orig = Some(unquote_path(p));
            f.status = "copied".into();
        } else if let Some(p) = line.strip_prefix("copy to ") {
            f.path = unquote_path(p);
        } else if line.starts_with("new file mode") {
            f.status = "new file".into();
        } else if line.starts_with("deleted file mode") {
            f.status = "deleted".into();
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            f.binary = true;
        }
    }
    files
}

/// `diff --git A B` with --no-prefix: when both halves are equal, that path.
fn guess_git_path(rest: &str) -> String {
    if rest.starts_with('"') {
        // quoted: "a" "b" or "a" b
        if let Some(end) = rest[1..].find("\" ") {
            return unquote_path(&rest[..end + 2]);
        }
    }
    let n = rest.len();
    if n % 2 == 1 {
        let (a, b) = (&rest[..n / 2], &rest[n / 2 + 1..]);
        if a == b {
            return a.to_string();
        }
    }
    rest.split(' ').next().unwrap_or(rest).to_string()
}

/// magit-apply--adjust-hunk-new-starts: shift the new start of each hunk by
/// the offset of the first one, as if only these hunks were applied.
pub fn adjust_new_starts(hunks: &[String]) -> Vec<String> {
    let Some(first) = hunks.first() else { return Vec::new() };
    let Some((r0, _)) = parse_hunk_header(first.lines().next().unwrap_or("")) else {
        return hunks.to_vec();
    };
    let offset = r0.new_start as isize - r0.old_start as isize;
    if offset == 0 {
        return hunks.to_vec();
    }
    hunks
        .iter()
        .map(|h| {
            let (head, body) = h.split_once('\n').unwrap_or((h, ""));
            match parse_hunk_header(head) {
                Some((mut r, rest)) => {
                    r.new_start = (r.new_start as isize - offset).max(0) as usize;
                    format!("{}\n{body}", format_hunk_header(r, &rest))
                }
                None => h.clone(),
            }
        })
        .collect()
}

/// magit-diff-hunk-region-patch + diff-fixup-modifs: keep the selected body
/// lines (indices FROM..=TO) of HUNK; outside the region, context lines are
/// kept, lines removed by the operation become context and the others are
/// dropped. OP is "+" when applying in reverse, "-" otherwise.
pub fn region_hunk(hunk: &Hunk, from: usize, to: usize, reverse: bool) -> String {
    let op = if reverse { '+' } else { '-' };
    let mut body: Vec<String> = Vec::new();
    let mut last_kept = false;
    for (i, l) in hunk.lines.iter().enumerate() {
        let c = l.chars().next().unwrap_or(' ');
        let selected = i >= from && i <= to;
        if c == '\\' {
            // "\ No newline at end of file" belongs to the line before it
            if last_kept {
                body.push(l.clone());
            }
            continue;
        }
        if c == ' ' || selected {
            body.push(l.clone());
            last_kept = true;
        } else if c == op {
            body.push(format!(" {}", &l[1..]));
            last_kept = true;
        } else {
            last_kept = false;
        }
    }
    let (old_len, new_len) = count_lines(&body);
    let (r, rest) = parse_hunk_header(&hunk.header).unwrap_or((
        HunkRange { old_start: 0, old_len: 0, new_start: 0, new_len: 0 },
        String::new(),
    ));
    let r = HunkRange { old_len, new_len, ..r };
    let mut s = format_hunk_header(r, &rest);
    s.push('\n');
    for l in body {
        s.push_str(&l);
        s.push('\n');
    }
    s
}

fn count_lines(body: &[String]) -> (usize, usize) {
    let mut old = 0;
    let mut new = 0;
    for l in body {
        match l.chars().next() {
            Some(' ') => {
                old += 1;
                new += 1;
            }
            Some('-') => old += 1,
            Some('+') => new += 1,
            _ => {}
        }
    }
    (old, new)
}

/// magit-apply-hunk: file header + the hunk with its new start adjusted.
pub fn hunk_patch(file: &FileDiff, hunk: usize) -> Option<String> {
    let h = file.hunks.get(hunk)?;
    let adjusted = adjust_new_starts(&[h.text()]);
    Some(format!("{}{}", file.hunk_header(), adjusted.concat()))
}

/// magit-apply-hunks: file header + several contiguous hunks.
pub fn hunks_patch(file: &FileDiff, hunks: &[usize]) -> Option<String> {
    let texts: Vec<String> = hunks.iter().map(|&i| file.hunks.get(i).map(Hunk::text)).collect::<Option<_>>()?;
    Some(format!("{}{}", file.hunk_header(), adjust_new_starts(&texts).concat()))
}

/// magit-apply-region: file header + the partial hunk.
pub fn region_patch(file: &FileDiff, hunk: usize, from: usize, to: usize, reverse: bool) -> Option<String> {
    let h = file.hunks.get(hunk)?;
    let partial = region_hunk(h, from.min(to), from.max(to), reverse);
    Some(format!("{}{}", file.hunk_header(), adjust_new_starts(&[partial]).concat()))
}

/// What part of a file section a command acts on (magit-diff-scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    File,
    Hunk(usize),
    /// several hunks of the same file (a region spanning hunk headings)
    Hunks(Vec<usize>),
    /// body lines FROM..=TO of one hunk
    Region { hunk: usize, from: usize, to: usize },
    /// the `chmod` pseudo-hunk (mode change)
    Chmod,
    /// the `rename` pseudo-hunk
    Rename,
}

/// The patch Magit would apply for SCOPE of FILE.
pub fn scope_patch(file: &FileDiff, scope: &Scope, reverse: bool) -> Option<String> {
    match scope {
        Scope::File => Some(file.text()),
        Scope::Hunk(i) => hunk_patch(file, *i),
        Scope::Hunks(v) => hunks_patch(file, v),
        Scope::Region { hunk, from, to } => region_patch(file, *hunk, *from, *to, reverse),
        Scope::Chmod => file.modes.as_deref().map(|m| file.meta_patch(m, false)),
        Scope::Rename => file.rename.as_deref().map(|r| file.meta_patch(r, true)),
    }
}
