//! ui/smerge.rs — the parts of smerge-mode (Emacs' vc/smerge-mode.el)
//! that magit-call-smerge uses: matching the conflict at a position
//! (smerge-match-conflict) and keeping one or all of its versions
//! (smerge-keep-n, smerge-keep-current, smerge-keep-all), on the bytes of
//! a file. Positions are byte offsets, like Emacs' points.

const BEGIN: &[u8] = b"<<<<<<< ";
const END: &[u8] = b">>>>>>> ";
const BASE: &[u8] = b"||||||| ";

/// What to keep of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// smerge-keep-current: the version at the position
    Current,
    /// smerge-keep-upper (ours)
    Upper,
    /// smerge-keep-base
    Base,
    /// smerge-keep-lower (theirs)
    Lower,
    /// smerge-keep-all: all versions concatenated
    All,
}

/// The match data of smerge-match-conflict: the whole conflict and the
/// (start, end) of its upper, base and lower versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conflict {
    pub start: usize,
    pub end: usize,
    pub upper: (usize, usize),
    pub base: Option<(usize, usize)>,
    pub lower: (usize, usize),
}

/// The beginnings of the lines of T.
fn bols(t: &[u8]) -> impl DoubleEndedIterator<Item = usize> + '_ {
    std::iter::once(0).chain(t.iter().enumerate().filter(|(_, &b)| b == b'\n').map(|(i, _)| i + 1))
}

/// `^PREFIX\(.*\)\n` at BOL: the end of the match and the group.
fn marker<'a>(t: &'a [u8], bol: usize, prefix: &[u8]) -> Option<(usize, &'a [u8])> {
    let rest = t.get(bol..)?;
    if !rest.starts_with(prefix) {
        return None;
    }
    let nl = rest.iter().position(|&b| b == b'\n')?;
    Some((bol + nl + 1, &rest[prefix.len()..nl]))
}

/// `^=======\n` at BOL: the end of the match.
fn lower_marker(t: &[u8], bol: usize) -> Option<usize> {
    t.get(bol..)?.starts_with(b"=======\n").then_some(bol + 8)
}

/// re-search-backward from POINT (bounded by BOUND): the last match
/// ending before POINT, as (beginning, end, group).
fn search_backward<'a>(
    t: &'a [u8],
    point: usize,
    bound: usize,
    m: impl Fn(&'a [u8], usize) -> Option<(usize, &'a [u8])>,
) -> Option<(usize, usize, &'a [u8])> {
    bols(t)
        .rev()
        .filter(|&b| b >= bound && b <= point)
        .find_map(|b| m(t, b).filter(|(e, _)| *e <= point).map(|(e, g)| (b, e, g)))
}

/// re-search-forward from POINT (bounded by BOUND): the first match.
fn search_forward<'a>(
    t: &'a [u8],
    point: usize,
    bound: usize,
    m: impl Fn(&'a [u8], usize) -> Option<(usize, &'a [u8])>,
) -> Option<(usize, usize, &'a [u8])> {
    bols(t).filter(|&b| b >= point).find_map(|b| m(t, b).filter(|(e, _)| *e <= bound).map(|(e, g)| (b, e, g)))
}

fn begin(t: &[u8], b: usize) -> Option<(usize, &[u8])> {
    marker(t, b, BEGIN)
}

/// The position of the next `<<<<<<< ` after POS (the end of the match,
/// like re-search-forward smerge-begin-re), else POS.
pub fn next_conflict(t: &[u8], pos: usize) -> usize {
    search_forward(t, pos, t.len(), begin).map_or(pos, |(_, e, _)| e)
}

/// The beginning of line LINE (1-based), like forward-line from the
/// beginning of the buffer: the end of the buffer past its last line.
pub fn line_position(t: &[u8], line: usize) -> usize {
    bols(t).nth(line.saturating_sub(1)).unwrap_or(t.len())
}

/// smerge-match-conflict: the conflict around POS of the file named
/// FILENAME (its base name, which marks a two-parts conflict).
pub fn match_conflict(t: &[u8], pos: usize, filename: &str) -> Result<Conflict, String> {
    let not_in = || "Point not in conflict region".to_string();
    // (forward-line 1) (re-search-backward smerge-begin-re)
    let p1 = t[pos.min(t.len())..].iter().position(|&b| b == b'\n').map_or(t.len(), |i| pos + i + 1);
    let (start, upper_start, name) = search_backward(t, p1, 0, begin).ok_or_else(not_in)?;
    let (lower_end, end, _) = search_forward(t, upper_start, t.len(), |t, b| marker(t, b, END)).ok_or_else(not_in)?;
    if end < pos {
        return Err(not_in());
    }
    let (mut upper_end, lower_start, _) =
        search_backward(t, end, start, |t, b| lower_marker(t, b).map(|e| (e, &t[0..0]))).ok_or_else(not_in)?;
    if search_forward(t, upper_start, end, begin).is_some() {
        return Err("There is a nested conflict".into());
    }
    let mut upper_start = upper_start;
    let mut base = None;
    if let Some((bs, be, _)) = search_backward(t, upper_end, start, |t, b| marker(t, b, BASE)) {
        // a 3-parts conflict
        base = Some((be, upper_end));
        upper_end = bs;
    } else if name == filename.as_bytes() {
        // a 2-parts conflict
    } else if name == b"ANCESTOR" || (!name.is_empty() && name.iter().all(|c| *c == b'.' || c.is_ascii_digit())) {
        // a same-diff conflict
        base = Some((upper_start, upper_end));
        upper_start = lower_start;
        upper_end = lower_end;
    }
    Ok(Conflict { start, end, upper: (upper_start, upper_end), base, lower: (lower_start, lower_end) })
}

/// delete-region on T (the arguments in any order).
fn delete_region(t: &mut Vec<u8>, a: usize, b: usize) {
    let (a, b) = (a.min(b), a.max(b));
    t.drain(a.min(t.len())..b.min(t.len()));
}

/// smerge-keep-n: the text with conflict C replaced by version (S, E).
fn keep_n(t: &[u8], c: &Conflict, (s, e): (usize, usize)) -> Vec<u8> {
    let mut out = t.to_vec();
    delete_region(&mut out, e, c.end);
    delete_region(&mut out, c.start, s);
    out
}

/// smerge-get-current: the version of C containing POS (3 lower, 2
/// base, 1 upper), else 0.
fn get_current(c: &Conflict, pos: usize) -> usize {
    let groups = [Some((c.start, c.end)), Some(c.upper), c.base, Some(c.lower)];
    (0..4).rev().find(|&i| groups[i].is_some_and(|(s, e)| s <= pos && pos <= e)).unwrap_or(0)
}

/// smerge-keep-all
fn keep_all(t: &[u8], c: &Conflict) -> Vec<u8> {
    let mut out = t.to_vec();
    let mb2 = c.base.map_or(t.len(), |b| b.0);
    let me2 = c.base.map_or(0, |b| b.1);
    delete_region(&mut out, c.lower.1, c.end);
    delete_region(&mut out, me2.max(c.upper.1), c.lower.0);
    if c.base.is_some() && c.upper.1 != c.lower.1 {
        delete_region(&mut out, c.upper.1, mb2);
    }
    delete_region(&mut out, c.start, c.upper.0.min(mb2));
    out
}

/// smerge-keep-upper / -base / -lower / -current / -all on the conflict
/// at POS of T: the resolved text, or smerge's error.
pub fn keep(t: &[u8], pos: usize, how: Keep, filename: &str) -> Result<Vec<u8>, String> {
    let c = match_conflict(t, pos, filename)?;
    match how {
        Keep::Upper => Ok(keep_n(t, &c, c.upper)),
        Keep::Lower => Ok(keep_n(t, &c, c.lower)),
        Keep::Base => c.base.map(|b| keep_n(t, &c, b)).ok_or_else(|| "No `base'".to_string()),
        Keep::All => Ok(keep_all(t, &c)),
        Keep::Current => match get_current(&c, pos) {
            1 => Ok(keep_n(t, &c, c.upper)),
            2 => Ok(keep_n(t, &c, c.base.unwrap_or_default())),
            3 => Ok(keep_n(t, &c, c.lower)),
            _ => Err("Not inside a version".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nb\n";
    const THREE: &str = "a\n<<<<<<< HEAD\nours\n||||||| base\norig\n=======\ntheirs\n>>>>>>> other\nb\n";

    fn kept(t: &str, line: usize, how: Keep) -> Result<String, String> {
        let pos = line_position(t.as_bytes(), line);
        keep(t.as_bytes(), pos, how, "f.txt").map(|v| String::from_utf8(v).unwrap())
    }

    #[test]
    fn current() {
        assert_eq!(kept(TWO, 3, Keep::Current).unwrap(), "a\nours\nb\n");
        assert_eq!(kept(TWO, 5, Keep::Current).unwrap(), "a\ntheirs\nb\n");
        // the beginning of the separator ends the upper version, that of
        // the end marker the lower one
        assert_eq!(kept(TWO, 4, Keep::Current).unwrap(), "a\nours\nb\n");
        assert_eq!(kept(TWO, 6, Keep::Current).unwrap(), "a\ntheirs\nb\n");
        assert_eq!(kept(TWO, 2, Keep::Current).unwrap_err(), "Not inside a version");
        // the line after the conflict still matches it
        assert_eq!(kept(TWO, 7, Keep::Current).unwrap_err(), "Not inside a version");
        assert_eq!(kept(TWO, 1, Keep::Current).unwrap_err(), "Point not in conflict region");
        assert_eq!(kept(TWO, 8, Keep::Current).unwrap_err(), "Point not in conflict region");
        assert_eq!(kept(THREE, 5, Keep::Current).unwrap(), "a\norig\nb\n");
        assert_eq!(kept(THREE, 3, Keep::Current).unwrap(), "a\nours\nb\n");
        assert_eq!(kept(THREE, 7, Keep::Current).unwrap(), "a\ntheirs\nb\n");
    }

    #[test]
    fn sides() {
        assert_eq!(kept(TWO, 3, Keep::Lower).unwrap(), "a\ntheirs\nb\n");
        assert_eq!(kept(TWO, 5, Keep::Upper).unwrap(), "a\nours\nb\n");
        assert_eq!(kept(TWO, 5, Keep::Base).unwrap_err(), "No `base'");
        assert_eq!(kept(THREE, 3, Keep::Base).unwrap(), "a\norig\nb\n");
        assert_eq!(kept(TWO, 3, Keep::All).unwrap(), "a\nours\ntheirs\nb\n");
        assert_eq!(kept(THREE, 3, Keep::All).unwrap(), "a\nours\norig\ntheirs\nb\n");
    }

    #[test]
    fn nested_and_next() {
        let t = "<<<<<<< A\nx\n<<<<<<< B\ny\n=======\nz\n>>>>>>> B\n";
        assert_eq!(kept(t, 2, Keep::Current).unwrap_err(), "There is a nested conflict");
        let t = TWO.as_bytes();
        let p = next_conflict(t, 0);
        assert_eq!(p, line_position(t, 3));
        assert_eq!(next_conflict(t, p), p);
        // a same-diff conflict: the upper version is the base
        let t = "<<<<<<< 1.2\nold\n=======\nnew\n>>>>>>> 1.3\n";
        assert_eq!(kept(t, 2, Keep::Base).unwrap(), "old\n");
    }
}
