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
            regions.push(Region {
                start: b[i],
                end: after(end_line),
                upper,
                base: base_bytes,
                lower,
                choice: Choice::Undecided,
            });
            i = end_line + 1;
        }
        ResolveFile { path: path.to_string(), original: text.to_vec(), regions }
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
