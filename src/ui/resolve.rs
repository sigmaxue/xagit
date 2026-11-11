//! ui/resolve.rs — the built-in conflict resolution view (an xagit
//! addition, covering the "choose A / B / both" part of Emacs Ediff's
//! three-way merge session): parse the conflict regions of a worktree file,
//! remember a per-conflict decision, and apply them all at once on save.

/// The choice made for one conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Choice {
    /// keep A / ours (the upper version)
    A,
    /// keep B / theirs (the lower version)
    B,
    /// keep both versions, one after the other (in their marker order)
    Both,
    /// keep the base version (diff3 / zdiff3 only)
    Base,
    /// not decided yet: the conflict markers are left in place
    #[default]
    Undecided,
}

impl Choice {
    pub fn label(self) -> &'static str {
        match self {
            Choice::A => "A (ours)",
            Choice::B => "B (theirs)",
            Choice::Both => "both",
            Choice::Base => "base",
            Choice::Undecided => "undecided",
        }
    }
}

/// One conflict region of a file, with the three versions and the byte
/// range of the whole region (markers included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub start: usize,
    pub end: usize,
    pub upper: Vec<u8>,
    pub base: Option<Vec<u8>>,
    pub lower: Vec<u8>,
    pub choice: Choice,
    /// the labels after the <<<<<<< and >>>>>>> markers ("HEAD",
    /// "abc1234 (subject)", a branch name …)
    pub upper_label: String,
    pub lower_label: String,
}

/// One side-by-side row of a conflict: the A line, the B line, and whether
/// the line has no counterpart on the other side (a changed line).
pub type AlignedRow = (Option<String>, Option<String>, bool);

/// The LCS-aligned rows of one conflict's A and B versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aligned {
    pub rows: Vec<AlignedRow>,
}

impl Aligned {
    /// Pair the A and B lines by position for a side-by-side comparison:
    /// row i shows A[i] next to B[i]; the shorter side is padded with a
    /// spacer.  A row is "changed" when the two lines differ (or one side
    /// is missing), which the view renders as a diff highlight.  Positional
    /// pairing (rather than LCS) keeps the two versions visually adjacent,
    /// which is what "compare the two columns and choose" needs.
    pub fn of(r: &Region) -> Aligned {
        let a: Vec<&[u8]> = split_lines(&r.upper);
        let b: Vec<&[u8]> = split_lines(&r.lower);
        let n = a.len().max(b.len());
        let mut rows: Vec<AlignedRow> = Vec::with_capacity(n);
        for i in 0..n {
            let (x, y) = (a.get(i).copied(), b.get(i).copied());
            let changed = match (x, y) {
                (Some(x), Some(y)) => x != y,
                _ => true,
            };
            rows.push((x.map(lossy), y.map(lossy), changed));
        }
        Aligned { rows }
    }
}

/// The lines of TEXT without their trailing newline.
fn split_lines(t: &[u8]) -> Vec<&[u8]> {
    let mut out: Vec<&[u8]> = t.split(|&b| b == b'\n').collect();
    if out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    for l in &mut out {
        if l.last() == Some(&b'\r') {
            *l = &l[..l.len() - 1];
        }
    }
    out
}

fn lossy(t: &[u8]) -> String {
    String::from_utf8_lossy(t).into_owned()
}

/// One display line of the full-file resolution view.  The parts of the
/// file that are NOT part of any conflict are shown in both columns alike
/// as context; conflict regions carry their A/B versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewLine {
    /// a line common to both versions (context, shown in both columns)
    Context(String),
    /// the header of conflict INDEX
    Header(usize),
    /// the column-title row of a conflict
    Columns,
    /// an A/B row of conflict INDEX (a, b, changed)
    Pair(usize, Option<String>, Option<String>, bool),
    /// a blank separator after a conflict
    Blank,
}

/// A file with its conflict regions and the surrounding text.
#[derive(Debug, Clone)]
pub struct ResolveFile {
    pub path: String,
    pub original: Vec<u8>,
    pub regions: Vec<Region>,
}

const BEGIN: &[u8] = b"<<<<<<<";
const BASE: &[u8] = b"|||||||";
const SEP: &[u8] = b"=======";
const END: &[u8] = b">>>>>>>";

/// The beginnings of all lines of T (including one past the end).
fn bols(t: &[u8]) -> Vec<usize> {
    let mut v: Vec<usize> = std::iter::once(0)
        .chain(t.iter().enumerate().filter(|(_, &b)| b == b'\n').map(|(i, _)| i + 1))
        .collect();
    if v.last() != Some(&t.len()) {
        v.push(t.len());
    }
    v
}

/// The line at index I (without its trailing newline), trimmed of '\r'.
fn line<'a>(t: &'a [u8], bols: &[usize], i: usize) -> &'a [u8] {
    let a = bols[i];
    let b = bols.get(i + 1).copied().unwrap_or(t.len());
    let mut s = &t[a..b];
    while s.last() == Some(&b'\n') || s.last() == Some(&b'\r') {
        s = &s[..s.len() - 1];
    }
    s
}

/// Whether the line at I is a marker with PREFIX (followed by a space or
/// end of line). Returns the byte offset just after that marker line.
fn is_marker(t: &[u8], bols: &[usize], i: usize, prefix: &[u8]) -> bool {
    let l = line(t, bols, i);
    l.starts_with(prefix) && l.get(prefix.len()).is_none_or(|&b| b == b' ' || b == b'\r')
}

impl ResolveFile {
    /// Parse the conflicts of the worktree file at PATH.  TEXT is its bytes.
    pub fn parse(path: &str, text: &[u8]) -> ResolveFile {
        let b = bols(text);
        let mut regions = Vec::new();
        let mut i = 0;
        while i + 1 < b.len() {
            if !is_marker(text, &b, i, BEGIN) {
                i += 1;
                continue;
            }
            // find the ======= separator, optional ||||||| base, and
            // >>>>>>> end; nested <<<<<<< aborts this region.
            let mut j = i + 1;
            let mut sep: Option<usize> = None;
            let mut base: Option<usize> = None;
            let mut end: Option<usize> = None;
            while j + 1 < b.len() {
                if is_marker(text, &b, j, BEGIN) {
                    break; // nested conflict: not a simple region
                }
                if is_marker(text, &b, j, BASE) && sep.is_none() {
                    base = Some(j);
                } else if line(text, &b, j) == SEP && sep.is_none() {
                    sep = Some(j);
                } else if is_marker(text, &b, j, END) {
                    end = Some(j);
                    break;
                }
                j += 1;
            }
            let (Some(sep_line), Some(end_line)) = (sep, end) else {
                // unterminated / malformed: leave the rest as ordinary text
                break;
            };
            let after = |idx: usize| b.get(idx + 1).copied().unwrap_or(text.len());
            let base_sep = base.unwrap_or(sep_line);
            let upper = text[after(i)..b[base_sep]].to_vec();
            let base_bytes = base.map(|bl| text[after(bl)..b[sep_line]].to_vec());
            let lower = text[after(sep_line)..b[end_line]].to_vec();
            let label = |idx: usize, prefix: &[u8]| {
                let l = line(text, &b, idx);
                String::from_utf8_lossy(&l[prefix.len()..]).trim().to_string()
            };
            regions.push(Region {
                start: b[i],
                end: after(end_line),
                upper,
                base: base_bytes,
                lower,
                choice: Choice::Undecided,
                upper_label: label(i, BEGIN),
                lower_label: label(end_line, END),
            });
            i = end_line + 1;
        }
        ResolveFile { path: path.to_string(), original: text.to_vec(), regions }
    }

    /// One aligned pair of lines of a conflict's A and B versions.  `None`
    /// means a blank spacer; `changed` marks a line with no counterpart on
    /// the other side (an added/removed line), `common` a matched line.
    pub fn aligned(&self) -> Vec<Aligned> {
        self.regions.iter().map(Aligned::of).collect()
    }

    /// The full-file display: ordinary (non-conflict) lines are shown as
    /// context in both columns; each conflict is a header, a column-title
    /// row, its A/B pairs and a blank separator.
    pub fn view(&self) -> Vec<ViewLine> {
        let mut out: Vec<ViewLine> = Vec::new();
        let mut at = 0;
        for (idx, r) in self.regions.iter().enumerate() {
            for l in split_lines(&self.original[at..r.start]) {
                out.push(ViewLine::Context(lossy(l)));
            }
            out.push(ViewLine::Header(idx));
            out.push(ViewLine::Columns);
            let al = Aligned::of(r);
            if al.rows.is_empty() {
                out.push(ViewLine::Pair(idx, None, None, false));
            } else {
                for (a, b, changed) in al.rows {
                    out.push(ViewLine::Pair(idx, a, b, changed));
                }
            }
            out.push(ViewLine::Blank);
            at = r.end;
        }
        for l in split_lines(&self.original[at..]) {
            out.push(ViewLine::Context(lossy(l)));
        }
        out
    }

    /// The lines of the merge result (Ediff's buffer C) with, for each, the
    /// conflict it comes from (None for the text around the conflicts).  A
    /// conflict whose result is empty still gets an (empty) entry so that
    /// its position can be shown.
    pub fn result_lines(&self) -> Vec<(String, Option<usize>, bool)> {
        let mut out: Vec<(String, Option<usize>, bool)> = Vec::new();
        let mut at = 0;
        for (idx, r) in self.regions.iter().enumerate() {
            for l in split_lines(&self.original[at..r.start]) {
                out.push((lossy(l), None, false));
            }
            let (bytes, empty_mark): (Vec<u8>, bool) = match r.choice {
                Choice::A => (r.upper.clone(), r.upper.is_empty()),
                Choice::B => (r.lower.clone(), r.lower.is_empty()),
                Choice::Both => {
                    let mut v = r.upper.clone();
                    v.extend_from_slice(&r.lower);
                    let e = v.is_empty();
                    (v, e)
                }
                Choice::Base => {
                    let v = r.base.clone().unwrap_or_default();
                    let e = v.is_empty();
                    (v, e)
                }
                Choice::Undecided => (self.original[r.start..r.end].to_vec(), false),
            };
            if empty_mark {
                out.push((String::new(), Some(idx), true));
            } else {
                for l in split_lines(&bytes) {
                    out.push((lossy(l), Some(idx), false));
                }
            }
            at = r.end;
        }
        for l in split_lines(&self.original[at..]) {
            out.push((lossy(l), None, false));
        }
        out
    }

    pub fn decided(&self) -> usize {
        self.regions.iter().filter(|r| r.choice != Choice::Undecided).count()
    }

    pub fn all_decided(&self) -> bool {
        self.regions.iter().all(|r| r.choice != Choice::Undecided)
    }

    /// The file bytes with every decided conflict replaced by its choice;
    /// undecided conflicts keep their markers.
    pub fn resolved(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.original.len());
        let mut at = 0;
        for r in &self.regions {
            out.extend_from_slice(&self.original[at..r.start]);
            match r.choice {
                Choice::A => out.extend_from_slice(&r.upper),
                Choice::B => out.extend_from_slice(&r.lower),
                Choice::Both => {
                    out.extend_from_slice(&r.upper);
                    out.extend_from_slice(&r.lower);
                }
                Choice::Base => {
                    if let Some(b) = &r.base {
                        out.extend_from_slice(b);
                    } else {
                        out.extend_from_slice(&self.original[r.start..r.end]);
                    }
                }
                Choice::Undecided => out.extend_from_slice(&self.original[r.start..r.end]),
            }
            at = r.end;
        }
        out.extend_from_slice(&self.original[at..]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "top\n<<<<<<< HEAD\nours1\nours2\n=======\ntheirs\n>>>>>>> other\nbottom\n";
    const THREE: &str = "top\n<<<<<<< HEAD\nours\n||||||| base\norig\n=======\ntheirs\n>>>>>>> other\nbottom\n";

    #[test]
    fn parses_two_way() {
        let f = ResolveFile::parse("f.txt", TWO.as_bytes());
        assert_eq!(f.regions.len(), 1);
        let r = &f.regions[0];
        assert_eq!(r.upper, b"ours1\nours2\n");
        assert_eq!(r.lower, b"theirs\n");
        assert_eq!(r.base, None);
        assert_eq!(r.choice, Choice::Undecided);
    }

    #[test]
    fn parses_three_way_and_multiple() {
        let text = format!("{TWO}middle\n{THREE}");
        let f = ResolveFile::parse("f.txt", text.as_bytes());
        assert_eq!(f.regions.len(), 2);
        assert_eq!(f.regions[1].base.as_deref(), Some(&b"orig\n"[..]));
        assert_eq!(f.regions[1].upper, b"ours\n");
    }

    #[test]
    fn aligns_lines_and_marks_changes() {
        let f = ResolveFile::parse("f.txt", TWO.as_bytes());
        let rows = &f.aligned()[0].rows;
        // positional pairing: A[i] beside B[i], the shorter side padded
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], (Some("ours1".into()), Some("theirs".into()), true));
        assert_eq!(rows[1], (Some("ours2".into()), None, true));

        // equal lines on both sides are marked unchanged
        let text = b"<<<<<<< HEAD\nA1\nsame\n=======\nB1\nsame\n>>>>>>> x\n";
        let f2 = ResolveFile::parse("f.txt", text);
        let rows = &f2.aligned()[0].rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], (Some("A1".into()), Some("B1".into()), true));
        assert_eq!(rows[1], (Some("same".into()), Some("same".into()), false));
    }

    #[test]
    fn view_shows_context_lines() {
        let f = ResolveFile::parse("f.txt", TWO.as_bytes());
        let v = f.view();
        // "top" before the conflict, "bottom" after: both as context
        assert_eq!(v.first(), Some(&ViewLine::Context("top".into())));
        assert_eq!(v.last(), Some(&ViewLine::Context("bottom".into())));
        assert!(v.iter().any(|l| *l == ViewLine::Header(0)));
        assert!(v.iter().any(|l| *l == ViewLine::Columns));
        let pairs: Vec<_> = v.iter().filter(|l| matches!(l, ViewLine::Pair(..))).collect();
        assert_eq!(pairs.len(), 2);
        // context lines surround the conflict in the two_conflicts case
        let text = format!("head\n{TWO}mid\n{THREE}tail\n");
        let f2 = ResolveFile::parse("f.txt", text.as_bytes());
        let v2 = f2.view();
        let contexts: Vec<&str> = v2.iter().filter_map(|l| match l { ViewLine::Context(s) => Some(s.as_str()), _ => None }).collect();
        assert_eq!(contexts, ["head", "top", "bottom", "mid", "top", "bottom", "tail"]);
        assert_eq!(v2.iter().filter(|l| matches!(l, ViewLine::Header(_))).count(), 2);
    }

    #[test]
    fn applies_all_choices() {
        let mut f = ResolveFile::parse("f.txt", TWO.as_bytes());
        f.regions[0].choice = Choice::A;
        assert_eq!(String::from_utf8(f.resolved()).unwrap(), "top\nours1\nours2\nbottom\n");
        f.regions[0].choice = Choice::B;
        assert_eq!(String::from_utf8(f.resolved()).unwrap(), "top\ntheirs\nbottom\n");
        f.regions[0].choice = Choice::Both;
        assert_eq!(String::from_utf8(f.resolved()).unwrap(), "top\nours1\nours2\ntheirs\nbottom\n");
        f.regions[0].choice = Choice::Undecided;
        assert_eq!(String::from_utf8(f.resolved()).unwrap(), TWO);
        assert!(!f.all_decided());
        let mut g = ResolveFile::parse("f.txt", THREE.as_bytes());
        g.regions[0].choice = Choice::Base;
        assert_eq!(String::from_utf8(g.resolved()).unwrap(), "top\norig\nbottom\n");
    }
}
