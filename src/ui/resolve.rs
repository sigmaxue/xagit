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
    /// the decision for the whole conflict (block-level a/b/d)
    pub choice: Choice,
    /// per-line decisions after refining the conflict (`*`). Empty until
    /// the first line-level choice; then one entry per aligned row.
    pub lines: Vec<Choice>,
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

/// The positional pairing of A and B as raw bytes (without the lossy
/// conversion of `Aligned::of`), used when writing the merge result so
/// non-UTF-8 content survives.
type RawRow<'a> = (Option<&'a [u8]>, Option<&'a [u8]>);

fn aligned_bytes<'a>(r: &'a Region) -> Vec<RawRow<'a>> {
    let a = split_lines(&r.upper);
    let b = split_lines(&r.lower);
    let n = a.len().max(b.len());
    (0..n).map(|i| (a.get(i).copied(), b.get(i).copied())).collect()
}

impl Region {
    /// Number of aligned rows (the lines shown side by side).
    pub fn line_count(&self) -> usize {
        aligned_bytes(self).len()
    }

    /// True once at least one line has been decided individually.
    pub fn refined(&self) -> bool {
        !self.lines.is_empty()
    }

    /// One choice per aligned row, padded with `Undecided`.
    pub fn line_choices(&self) -> Vec<Choice> {
        let mut v = self.lines.clone();
        v.resize(self.line_count(), Choice::Undecided);
        v
    }

    /// Decide one line. False when ROW is out of range. A line-level
    /// choice takes over from a whole-conflict decision.
    pub fn set_line(&mut self, row: usize, choice: Choice) -> bool {
        let n = self.line_count();
        if row >= n {
            return false;
        }
        self.lines.resize(n, Choice::Undecided);
        self.lines[row] = choice;
        self.choice = Choice::Undecided;
        true
    }

    /// Number of lines still undecided (0 when not refined).
    pub fn undecided_lines(&self) -> usize {
        if !self.refined() {
            return 0;
        }
        self.line_choices().iter().filter(|c| **c == Choice::Undecided).count()
    }

    /// True when every line has a decision.
    pub fn lines_decided(&self) -> bool {
        self.refined() && self.undecided_lines() == 0
    }

    /// How much is still unresolved: lines when refined, else the whole
    /// conflict.
    pub fn unresolved_units(&self) -> usize {
        if self.refined() {
            self.undecided_lines()
        } else if self.choice == Choice::Undecided {
            1
        } else {
            0
        }
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
                lines: Vec::new(),
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

    /// The bytes one conflict contributes to the merge result. `None`
    /// means it is still undecided and keeps its original markers. A
    /// refined conflict whose every line is decided is written line by
    /// line; a partly decided one keeps the markers, so no side is lost.
    fn region_bytes(&self, r: &Region) -> Option<Vec<u8>> {
        if r.lines_decided() {
            let rows = aligned_bytes(r);
            let choices = r.line_choices();
            let crlf = r.upper.windows(2).any(|w| w == b"\r\n") || r.lower.windows(2).any(|w| w == b"\r\n");
            let nl: &[u8] = if crlf { b"\r\n" } else { b"\n" };
            let mut out = Vec::new();
            for (i, (a, b)) in rows.iter().enumerate() {
                let c = choices.get(i).copied().unwrap_or(Choice::Undecided);
                if matches!(c, Choice::A | Choice::Both) {
                    if let Some(s) = a {
                        out.extend_from_slice(s);
                        out.extend_from_slice(nl);
                    }
                }
                if matches!(c, Choice::B | Choice::Both) {
                    if let Some(s) = b {
                        out.extend_from_slice(s);
                        out.extend_from_slice(nl);
                    }
                }
            }
            return Some(out);
        }
        match r.choice {
            Choice::A => Some(r.upper.clone()),
            Choice::B => Some(r.lower.clone()),
            Choice::Both => {
                let mut v = r.upper.clone();
                v.extend_from_slice(&r.lower);
                Some(v)
            }
            Choice::Base => r.base.clone(),
            Choice::Undecided => None,
        }
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
            let bytes = self.region_bytes(r).unwrap_or_else(|| self.original[r.start..r.end].to_vec());
            let empty_mark = bytes.is_empty();
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
        self.regions.iter().filter(|r| r.unresolved_units() == 0).count()
    }

    /// Swap the two sides of every conflict.  During a rebase the "onto"
    /// (target) branch is the file's upper (HEAD) side; xagit shows the
    /// target on the right, so the columns — and therefore the byte ranges
    /// that A/B choices write — are swapped here once, when the view opens.
    pub fn swap_sides(&mut self) {
        for r in &mut self.regions {
            std::mem::swap(&mut r.upper, &mut r.lower);
            std::mem::swap(&mut r.upper_label, &mut r.lower_label);
            r.choice = Choice::Undecided;
            // the sides (and their alignment) changed: old line choices
            // would now refer to the opposite column
            r.lines.clear();
        }
    }

    pub fn all_decided(&self) -> bool {
        self.regions.iter().all(|r| r.unresolved_units() == 0)
    }

    /// The file bytes with every decided conflict replaced by its choice;
    /// undecided conflicts keep their markers.
    pub fn resolved(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.original.len());
        let mut at = 0;
        for r in &self.regions {
            out.extend_from_slice(&self.original[at..r.start]);
            out.extend_from_slice(&self.region_bytes(r).unwrap_or_else(|| self.original[r.start..r.end].to_vec()));
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
    fn swap_sides_puts_the_target_on_the_right() {
        // rebase conflict: upper = onto (master), lower = branch (topic)
        let text = b"top\n<<<<<<< HEAD\nMASTER\n=======\nTOPIC\n>>>>>>> topic\n";
        let mut f = ResolveFile::parse("f.txt", text);
        f.swap_sides();
        let r = &f.regions[0];
        assert_eq!(r.upper, b"TOPIC\n");
        assert_eq!(r.lower, b"MASTER\n");
        assert_eq!(r.upper_label, "topic");
        assert_eq!(r.lower_label, "HEAD");
        // A (left) is now topic, B (right) is the target master
        f.regions[0].choice = Choice::A;
        assert_eq!(String::from_utf8(f.resolved()).unwrap(), "top\nTOPIC\n");
        f.regions[0].choice = Choice::B;
        assert_eq!(String::from_utf8(f.resolved()).unwrap(), "top\nMASTER\n");
    }

    #[test]
    fn view_shows_context_lines() {
        let f = ResolveFile::parse("f.txt", TWO.as_bytes());
        let v = f.view();
        // "top" before the conflict, "bottom" after: both as context
        assert_eq!(v.first(), Some(&ViewLine::Context("top".into())));
        assert_eq!(v.last(), Some(&ViewLine::Context("bottom".into())));
        assert!(v.contains(&ViewLine::Header(0)));
        assert!(v.contains(&ViewLine::Columns));
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

    #[test]
    fn line_choices_write_one_line_at_a_time() {
        let mut f = ResolveFile::parse("f.txt", TWO.as_bytes());
        assert_eq!(f.regions[0].line_count(), 2);
        assert!(!f.all_decided());
        // one decided line: keep the markers so neither side is lost
        assert!(f.regions[0].set_line(0, Choice::A));
        assert_eq!(String::from_utf8(f.resolved()).unwrap(), TWO);
        assert!(!f.all_decided());
        // every line decided: write them in order
        assert!(f.regions[0].set_line(1, Choice::Both));
        assert_eq!(String::from_utf8(f.resolved()).unwrap(), "top\nours1\nours2\nbottom\n");
        assert!(f.all_decided());
        // B for one line, A for the other
        f.regions[0].set_line(0, Choice::B);
        f.regions[0].set_line(1, Choice::A);
        assert_eq!(String::from_utf8(f.resolved()).unwrap(), "top\ntheirs\nours2\nbottom\n");
        assert!(!f.regions[0].set_line(9, Choice::A));
    }

    #[test]
    fn block_choice_and_line_choices_reset_each_other() {
        let mut f = ResolveFile::parse("f.txt", TWO.as_bytes());
        f.regions[0].choice = Choice::A;
        f.regions[0].set_line(0, Choice::B);
        assert_eq!(f.regions[0].choice, Choice::Undecided);
        f.regions[0].lines.clear();
        f.regions[0].choice = Choice::B;
        assert_eq!(String::from_utf8(f.resolved()).unwrap(), "top\ntheirs\nbottom\n");
    }
}
