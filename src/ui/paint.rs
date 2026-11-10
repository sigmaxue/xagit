//! ui/paint.rs — styled rows and how buffer lines are highlighted.
//!
//! The app lays out rows of styled spans; `write_rows` sends them to the
//! terminal. Every row is painted over its full width (its FILL style
//! after the spans), so a frame never needs a full clear and doesn't
//! flicker, and output another program left on the screen is overwritten.

use super::theme::{Face, Style, Theme};
use super::view::{BufferLine, LineKind};
use super::width::{char_width, sanitize_for_terminal, str_width, truncate};
use crossterm::queue;
use crossterm::style::{Attribute, Print, SetAttribute, SetBackgroundColor, SetForegroundColor};
use std::io::Write;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

/// A screen row: spans, then FILL to the right edge.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    pub spans: Vec<Span>,
    pub fill: Style,
}

impl Row {
    pub fn new(fill: Style) -> Row {
        Row { spans: Vec::new(), fill }
    }

    /// A row of TEXT in STYLE.
    pub fn text(text: impl Into<String>, style: Style) -> Row {
        let mut r = Row::new(Style::default());
        r.push(text, style);
        r
    }

    pub fn push(&mut self, text: impl Into<String>, style: Style) -> &mut Row {
        let text = text.into();
        if !text.is_empty() {
            match self.spans.last_mut() {
                Some(last) if last.style == style => last.text.push_str(&text),
                _ => self.spans.push(Span { text, style }),
            }
        }
        self
    }

    pub fn extend(&mut self, spans: impl IntoIterator<Item = Span>) -> &mut Row {
        for s in spans {
            self.push(s.text, s.style);
        }
        self
    }

    /// The text of the row (what tests and the minibuffer see).
    pub fn plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    pub fn width(&self) -> usize {
        self.spans.iter().map(|s| str_width(&s.text)).sum()
    }

    /// The row cut to W columns.
    pub fn truncated(mut self, w: usize) -> Row {
        let mut used = 0;
        let mut keep = 0;
        for s in &mut self.spans {
            let sw = str_width(&s.text);
            if used + sw <= w {
                used += sw;
                keep += 1;
                continue;
            }
            let t = truncate(&s.text, w - used).to_string();
            s.text = t;
            keep += 1;
            break;
        }
        self.spans.truncate(keep);
        self.spans.retain(|s| !s.text.is_empty());
        self
    }

    /// LEFT, then RIGHT flush with the right edge of W columns; LEFT is
    /// cut first when both don't fit.
    pub fn split(left: Row, right: Row, w: usize) -> Row {
        let rw = right.width();
        if rw >= w {
            return right.truncated(w);
        }
        let fill = left.fill;
        let mut row = left.truncated(w - rw);
        let gap = w - rw - row.width();
        row.push(" ".repeat(gap), fill);
        row.extend(right.spans);
        row
    }
}

/// How a character of a buffer line is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Paint {
    Face(Face),
    /// the Nth column of a log graph
    Graph(usize),
}

const GRAPH_CHARS: &str = "-_/|\\*o<>. ";

/// magit-diff-file-heading's change types, longest first.
const FILE_STATES: &[(&str, Face)] = &[
    ("both modified", Face::FileUnmerged),
    ("both added", Face::FileUnmerged),
    ("both deleted", Face::FileUnmerged),
    ("added by us", Face::FileUnmerged),
    ("added by them", Face::FileUnmerged),
    ("deleted by us", Face::FileUnmerged),
    ("deleted by them", Face::FileUnmerged),
    ("typechange", Face::FileRenamed),
    ("new file", Face::FileNew),
    ("modified", Face::FileModified),
    ("unmerged", Face::FileUnmerged),
    ("deleted", Face::FileDeleted),
    ("renamed", Face::FileRenamed),
    ("copied", Face::FileRenamed),
    ("added", Face::FileNew),
];

fn is_hex(s: &[char]) -> bool {
    s.len() >= 4 && s.iter().all(char::is_ascii_hexdigit)
}

/// The end of the word starting at I (the first whitespace after it).
fn word_end(c: &[char], i: usize) -> usize {
    let mut j = i;
    while j < c.len() && !c[j].is_whitespace() {
        j += 1;
    }
    j
}

fn skip_space(c: &[char], i: usize) -> usize {
    let mut j = i;
    while j < c.len() && c[j] == ' ' {
        j += 1;
    }
    j
}

fn fill(p: &mut [Paint], a: usize, b: usize, face: Face) {
    let n = p.len();
    for x in &mut p[a.min(n)..b.min(n)] {
        *x = Paint::Face(face);
    }
}

/// The faces of a ref name in the header lines ("Head:", "Merge:" …).
fn header_ref_face(label: &str, word: &[char]) -> Face {
    if is_hex(word) && word.len() >= 7 {
        return Face::Hash;
    }
    match label {
        "Head:" => Face::BranchCurrent,
        "Tag:" => Face::Tag,
        _ if word.contains(&'/') => Face::BranchRemote,
        _ => Face::BranchLocal,
    }
}

/// A commit line: graph, hash (or keyword and hash), more graph when the
/// hash comes first (--graph with magit-log-margin's alignment).
fn paint_commit(c: &[char], p: &mut [Paint]) {
    let mut i = 0;
    // a graph starts with one of these ("o", "-" and "." also occur in
    // words, as in "onto")
    let starts_graph = |i: usize| c.get(i).is_some_and(|ch| "*|/\\_".contains(*ch));
    let graph = |i: &mut usize, p: &mut [Paint]| {
        let start = *i;
        while *i < c.len() && GRAPH_CHARS.contains(c[*i]) {
            if c[*i] != ' ' {
                p[*i] = Paint::Graph((*i - start) / 2);
            }
            *i += 1;
        }
    };
    // the cherry marks of magit-insert-cherries ("+ hash", "- hash")
    if c.len() > 2 && matches!(c[0], '+' | '-') && c[1] == ' ' {
        p[0] = Paint::Face(if c[0] == '+' { Face::FileNew } else { Face::FileDeleted });
        i = 2;
    } else if starts_graph(0) {
        graph(&mut i, p);
    }
    let end = word_end(c, i);
    if is_hex(&c[i..end]) {
        fill(p, i, end, Face::Hash);
        let mut j = skip_space(c, end);
        if starts_graph(j) {
            graph(&mut j, p);
        }
        return;
    }
    // sequence lines: "pick abc1234 subject", "onto abc1234 …"; the
    // headers of revision buffers: "Parent:     abc1234 subject"
    let next = skip_space(c, end);
    let next_end = word_end(c, next);
    let word = &c[i..end];
    let label = word.last() == Some(&':');
    let letters = if label { &word[..word.len() - 1] } else { word };
    if !letters.is_empty() && letters.iter().all(|ch| ch.is_ascii_alphabetic()) && is_hex(&c[next..next_end]) {
        fill(p, i, end, if label { Face::Label } else { Face::Keyword });
        fill(p, next, next_end, Face::Hash);
    } else if label && letters.iter().all(|ch| ch.is_ascii_alphabetic()) {
        // "Follows:    v1.0 (3)"
        fill(p, i, end, Face::Label);
        fill(p, next, next_end, Face::Tag);
    }
}

/// A branch or tag line of the refs buffer: the focus column, the name,
/// its ahead/behind counts and its upstream.
fn paint_ref(c: &[char], p: &mut [Paint], kind: super::refs::RefKind, name: &str) {
    use super::refs::RefKind;
    let name: Vec<char> = name.chars().collect();
    let Some(at) = (0..c.len()).find(|&i| c[i..].starts_with(&name)) else { return };
    let current = c[..at].iter().any(|ch| matches!(ch, '@' | '*'));
    for i in 0..at {
        if !c[i].is_whitespace() {
            p[i] = Paint::Face(if matches!(c[i], '@' | '*') { Face::Head } else { Face::Ahead });
        }
    }
    let face = match kind {
        RefKind::Local if current => Face::BranchCurrent,
        RefKind::Local => Face::BranchLocal,
        RefKind::Remote => Face::BranchRemote,
        RefKind::Tag => Face::Tag,
    };
    let end = at + name.len();
    fill(p, at, end, face);
    let counts_end = word_end(c, end);
    fill(p, end, counts_end, Face::Ahead);
    let up = skip_space(c, counts_end);
    let up_end = word_end(c, up);
    if kind == RefKind::Local && up_end > up && c[up..up_end].contains(&'/') {
        fill(p, up, up_end, Face::BranchRemote);
    }
}

/// The paints of the characters C of a line of KIND.
fn paint_kind(kind: &LineKind, c: &[char], p: &mut [Paint]) {
    let n = c.len();
    match kind {
        LineKind::SectionHeader { .. } | LineKind::Remote { .. } => {
            fill(p, 0, n, Face::SectionHeading);
            // "Title (N)"
            if c.last() == Some(&')') {
                if let Some(open) = (0..n).rev().find(|&i| c[i] == '(' && i > 0 && c[i - 1] == ' ') {
                    let inner = &c[open + 1..n - 1];
                    if !inner.is_empty() && inner.iter().all(|ch| ch.is_ascii_digit() || *ch == '+') {
                        fill(p, open - 1, n, Face::Count);
                    }
                }
            }
        }
        LineKind::Head => {
            let Some(colon) = c.iter().take(12).position(|&ch| ch == ':') else { return };
            fill(p, 0, colon + 1, Face::Label);
            let label: String = c[..=colon].iter().collect();
            let w = skip_space(c, colon + 1);
            let e = word_end(c, w);
            if e > w {
                fill(p, w, e, header_ref_face(&label, &c[w..e]));
            }
        }
        LineKind::File { .. } => {
            let text: String = c.iter().collect();
            let state = FILE_STATES.iter().find(|(s, _)| {
                text.starts_with(s) && text[s.len()..].starts_with([' ', ':'])
            });
            let path_at = match state {
                Some((s, face)) => {
                    let k = s.chars().count();
                    fill(p, 0, k + usize::from(c.get(k) == Some(&':')), *face);
                    skip_space(c, k + usize::from(c.get(k) == Some(&':')))
                }
                None => 0,
            };
            fill(p, path_at, n, Face::FileHeading);
        }
        LineKind::Hunk { .. } => {
            fill(p, 0, n, Face::HunkHeading);
            let text: String = c.iter().collect();
            if let Some(rest) = text.strip_prefix("@@") {
                if let Some(close) = rest.find("@@") {
                    let k = text[..2 + close + 2].chars().count();
                    fill(p, 0, k, Face::HunkRange);
                }
            }
        }
        LineKind::DiffLine { .. } => {
            let text: String = c.iter().take(8).collect();
            let face = if ["<<<<<<<", "=======", ">>>>>>>", "|||||||"].iter().any(|m| text.starts_with(m)) {
                Face::Conflict
            } else {
                match c.first() {
                    Some('+') => Face::Added,
                    Some('-') => Face::Removed,
                    // the second column of a combined diff
                    Some(' ') if c.get(1) == Some(&'+') => Face::Added,
                    Some(' ') if c.get(1) == Some(&'-') => Face::Removed,
                    _ => Face::Context,
                }
            };
            fill(p, 0, n, face);
        }
        LineKind::Meta { .. } => fill(p, 0, n, Face::Meta),
        LineKind::Stash { .. } => {
            let e = word_end(c, 0);
            fill(p, 0, e, Face::Stash);
        }
        LineKind::Commit { .. } => paint_commit(c, p),
        LineKind::BlameHeading { .. } => {
            fill(p, 0, n, Face::BlameHeading);
            let w = skip_space(c, 0);
            let e = word_end(c, w);
            if is_hex(&c[w..e]) {
                fill(p, w, e, Face::Hash);
            }
        }
        LineKind::Ref { kind, name } => paint_ref(c, p, *kind, name),
        LineKind::Info => {
            // "Author:     …" of revision buffers
            if let Some(colon) = c.iter().take(14).position(|&ch| ch == ':') {
                if colon > 0
                    && c[0].is_ascii_uppercase()
                    && c[..colon].iter().all(|ch| ch.is_ascii_alphabetic())
                    && c.get(colon + 1).is_none_or(|ch| *ch == ' ')
                {
                    fill(p, 0, colon + 1, Face::Label);
                }
            }
        }
        LineKind::FileLine { .. } => {}
    }
}

/// The spans of LINE: the faces its inserter gave, the others inferred
/// from its kind.
pub fn highlight(theme: &Theme, line: &BufferLine) -> Vec<Span> {
    let c: Vec<char> = line.text.chars().collect();
    let mut p = vec![Paint::Face(Face::Default); c.len()];
    paint_kind(&line.kind, &c, &mut p);
    if !line.faces.is_empty() {
        let starts: Vec<usize> = line.text.char_indices().map(|(i, _)| i).collect();
        let at = |byte: usize| starts.partition_point(|&s| s < byte);
        for &(a, b, face) in &line.faces {
            fill(&mut p, at(a), at(b), face);
        }
    }
    spans_of(theme, &c, &p)
}

fn spans_of(theme: &Theme, c: &[char], p: &[Paint]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let mut j = i + 1;
        while j < c.len() && p[j] == p[i] {
            j += 1;
        }
        let style = match p[i] {
            Paint::Face(f) => theme.style(f),
            Paint::Graph(n) => theme.graph(n),
        };
        out.push(Span { text: c[i..j].iter().collect(), style });
        i = j;
    }
    out
}

/// Whether TEXT (of the text view) is a diff or a patch.
pub fn looks_like_diff(text: &str) -> bool {
    text.starts_with("diff --git") || text.starts_with("@@ ") || text.contains("\ndiff --git") || text.contains("\n@@ ")
}

/// The face of LINE of the text view (FIRST: its first line; DIFF: the
/// text is a diff).
pub fn text_face(line: &str, diff: bool, first: bool) -> Face {
    if first && line.starts_with("$ ") {
        Face::Prompt
    } else if line.starts_with("! ") {
        Face::FileDeleted
    } else if !diff {
        Face::Default
    } else if line.starts_with("diff ") || line.starts_with("+++ ") || line.starts_with("--- ") || line.starts_with("index ") {
        Face::FileHeading
    } else if line.starts_with("@@") {
        Face::HunkRange
    } else if line.starts_with('+') {
        Face::Added
    } else if line.starts_with('-') {
        Face::Removed
    } else if line.starts_with("commit ") {
        Face::Hash
    } else {
        Face::Default
    }
}

/// LINE wrapped at W columns (by display width; a wide character that
/// doesn't fit goes to the next row).
pub fn wrap(line: &str, w: usize) -> Vec<String> {
    let (line, _) = sanitize_for_terminal(line, 0);
    if line.is_empty() {
        return vec![String::new()];
    }
    let w = w.max(2);
    let mut rows = Vec::new();
    let mut cur = String::new();
    let mut used = 0;
    for ch in line.chars() {
        let cw = char_width(ch);
        if used + cw > w {
            rows.push(std::mem::take(&mut cur));
            used = 0;
        }
        cur.push(ch);
        used += cw;
    }
    rows.push(cur);
    rows
}

fn set_style(out: &mut impl Write, theme: &Theme, s: Style) -> std::io::Result<()> {
    queue!(out, SetAttribute(Attribute::Reset))?;
    if let Some(fg) = s.fg {
        queue!(out, SetForegroundColor(theme.color(fg)))?;
    }
    if let Some(bg) = s.bg {
        queue!(out, SetBackgroundColor(theme.color(bg)))?;
    }
    let basic = matches!(theme.depth, super::theme::Depth::Basic);
    for (on, a) in [
        (s.bold, Attribute::Bold),
        (s.dim, Attribute::Dim),
        // the Linux console shows italics as a color
        (s.italic && !basic, Attribute::Italic),
        (s.underline, Attribute::Underlined),
        (s.reverse, Attribute::Reverse),
    ] {
        if on {
            queue!(out, SetAttribute(a))?;
        }
    }
    Ok(())
}

/// Paint ROW at line Y, exactly W columns wide.
pub fn write_row(out: &mut impl Write, theme: &Theme, y: u16, row: &Row, w: usize) -> std::io::Result<()> {
    queue!(out, crossterm::cursor::MoveTo(0, y))?;
    let mut col = 0;
    let mut current: Option<Style> = None;
    for s in &row.spans {
        if col >= w {
            break;
        }
        let (text, _) = sanitize_for_terminal(&s.text, col);
        let text = truncate(&text, w - col);
        if text.is_empty() {
            continue;
        }
        if current != Some(s.style) {
            set_style(out, theme, s.style)?;
            current = Some(s.style);
        }
        queue!(out, Print(text))?;
        col += str_width(text);
    }
    if col < w {
        if current != Some(row.fill) {
            set_style(out, theme, row.fill)?;
        }
        queue!(out, Print(" ".repeat(w - col)))?;
    }
    queue!(out, SetAttribute(Attribute::Reset))?;
    Ok(())
}

/// Paint ROWS from the top of a W x H screen, blank rows below them, in
/// one synchronized update (no tearing in terminals that support it;
/// others ignore it).
pub fn write_rows(out: &mut impl Write, theme: &Theme, rows: &[Row], w: usize, h: usize) -> std::io::Result<()> {
    queue!(out, crossterm::terminal::BeginSynchronizedUpdate, crossterm::cursor::Hide)?;
    let blank = Row::default();
    for y in 0..h {
        write_row(out, theme, y as u16, rows.get(y).unwrap_or(&blank), w)?;
    }
    queue!(out, crossterm::terminal::EndSynchronizedUpdate)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Depth;

    fn line(text: &str, kind: LineKind) -> BufferLine {
        BufferLine {
            text: text.into(),
            kind,
            ident: String::new(),
            heading: false,
            depth: 0,
            has_body: false,
            ancestors: Vec::new(),
            hidden: None,
            pos: 0,
            hide: false,
            margin: String::new(),
            faces: Vec::new(),
        }
    }

    fn theme() -> Theme {
        Theme { depth: Depth::True, ..Theme::default() }
    }

    /// The text of the spans of LINE that have the style of FACE.
    fn with_face(line: &BufferLine, face: Face) -> Vec<String> {
        let t = theme();
        highlight(&t, line).into_iter().filter(|s| s.style == t.style(face)).map(|s| s.text).collect()
    }

    #[test]
    fn spans_keep_the_text() {
        let l = line("abc1234 main origin/main 修复登录", LineKind::Commit { section: "recent".into(), hash: "abc1234".into() });
        let text: String = highlight(&theme(), &l).iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, l.text);
    }

    #[test]
    fn commit_lines() {
        let mut l = line("abc1234 main origin/main fix", LineKind::Commit { section: "recent".into(), hash: "abc1234".into() });
        l.faces = vec![(8, 12, Face::BranchCurrent), (13, 24, Face::BranchRemote)];
        assert_eq!(with_face(&l, Face::Hash), ["abc1234"]);
        assert_eq!(with_face(&l, Face::BranchCurrent), ["main"]);
        assert_eq!(with_face(&l, Face::BranchRemote), ["origin/main"]);
        let l = line("| * def5678 message", LineKind::Commit { section: "log".into(), hash: "def5678".into() });
        assert_eq!(with_face(&l, Face::Hash), ["def5678"]);
        let l = line("pick 1234abc subject", LineKind::Commit { section: "rebase".into(), hash: "1234abc".into() });
        assert_eq!(with_face(&l, Face::Keyword), ["pick"]);
        assert_eq!(with_face(&l, Face::Hash), ["1234abc"]);
        let l = line("Parent:     1234abc 修复", LineKind::Commit { section: "parent".into(), hash: "1234abc".into() });
        assert_eq!(with_face(&l, Face::Label), ["Parent:"]);
        assert_eq!(with_face(&l, Face::Hash), ["1234abc"]);
        // "onto" is a word, not a graph starting with "o"
        let l = line("onto 1234abc subject", LineKind::Commit { section: "rebase".into(), hash: "1234abc".into() });
        assert_eq!(with_face(&l, Face::Keyword), ["onto"]);
        let l = line("+ 1234abc cherry", LineKind::Commit { section: "cherry".into(), hash: "1234abc".into() });
        assert_eq!(with_face(&l, Face::FileNew), ["+"]);
        assert_eq!(with_face(&l, Face::Hash), ["1234abc"]);
    }

    #[test]
    fn headings_files_and_diffs() {
        let l = line("Unstaged changes (2)", LineKind::SectionHeader { section: "unstaged".into(), n: 2 });
        assert_eq!(with_face(&l, Face::SectionHeading), ["Unstaged changes"]);
        assert_eq!(with_face(&l, Face::Count), [" (2)"]);
        let l = line("Head:     main subject", LineKind::Head);
        assert_eq!(with_face(&l, Face::Label), ["Head:"]);
        assert_eq!(with_face(&l, Face::BranchCurrent), ["main"]);
        let l = line("Merge:    origin/main subject", LineKind::Head);
        assert_eq!(with_face(&l, Face::BranchRemote), ["origin/main"]);
        let l = line("modified   src/a.rs", LineKind::File { section: "unstaged".into(), path: "src/a.rs".into() });
        assert_eq!(with_face(&l, Face::FileModified), ["modified"]);
        assert_eq!(with_face(&l, Face::FileHeading), ["src/a.rs"]);
        let l = line("new file   b.rs", LineKind::File { section: "staged".into(), path: "b.rs".into() });
        assert_eq!(with_face(&l, Face::FileNew), ["new file"]);
        let l = line("@@ -1,2 +1,3 @@ fn main", LineKind::Hunk { section: "s".into(), path: "a".into(), hunk: 0 });
        assert_eq!(with_face(&l, Face::HunkRange), ["@@ -1,2 +1,3 @@"]);
        let dl = |t: &str| line(t, LineKind::DiffLine { section: "s".into(), path: "a".into(), hunk: 0, line: 0 });
        assert_eq!(with_face(&dl("+added"), Face::Added), ["+added"]);
        assert_eq!(with_face(&dl("-gone"), Face::Removed), ["-gone"]);
        assert_eq!(with_face(&dl("<<<<<<< HEAD"), Face::Conflict), ["<<<<<<< HEAD"]);
    }

    #[test]
    fn refs_lines() {
        use crate::ui::refs::RefKind;
        let l = line("@ main3> origin/main subject", LineKind::Ref { kind: RefKind::Local, name: "main".into() });
        assert_eq!(with_face(&l, Face::Head), ["@"]);
        assert_eq!(with_face(&l, Face::BranchCurrent), ["main"]);
        assert_eq!(with_face(&l, Face::Ahead), ["3>"]);
        assert_eq!(with_face(&l, Face::BranchRemote), ["origin/main"]);
        let l = line("  v1.0 release", LineKind::Ref { kind: RefKind::Tag, name: "v1.0".into() });
        assert_eq!(with_face(&l, Face::Tag), ["v1.0"]);
    }

    #[test]
    fn rows_fit_their_width() {
        let left = Row::text("修复登录问题", Style::default());
        let right = Row::text("main", Style::default());
        let r = Row::split(left, right, 12);
        assert_eq!(r.width(), 12);
        assert!(r.plain().ends_with("main"));
        assert_eq!(r.plain(), "修复登录main");
        let r = Row::split(Row::text("ab", Style::default()), Row::text("xyz", Style::default()), 8);
        assert_eq!(r.plain(), "ab   xyz");
    }

    #[test]
    fn wrapping_by_width() {
        assert_eq!(wrap("abcdef", 4), ["abcd", "ef"]);
        assert_eq!(wrap("修复登录", 5), ["修复", "登录"]);
        assert_eq!(wrap("", 4), [""]);
        assert_eq!(wrap("a\tb", 80), ["a       b"]);
    }

    #[test]
    fn painted_rows_have_the_exact_width() {
        let t = theme();
        let mut out = Vec::new();
        let mut row = Row::new(Style::default());
        row.push("修复", t.style(Face::Hash)).push("\tx", Style::default());
        write_row(&mut out, &t, 0, &row, 20).unwrap();
        let s = String::from_utf8(out).unwrap();
        // strip the escape sequences
        let mut plain = String::new();
        let mut esc = false;
        for ch in s.chars() {
            match (esc, ch) {
                (false, '\u{1b}') => esc = true,
                (true, c) if c.is_ascii_alphabetic() => esc = false,
                (true, _) => {}
                (false, c) => plain.push(c),
            }
        }
        assert_eq!(str_width(&plain), 20, "{plain:?}");
        assert!(plain.starts_with("修复    x"), "{plain:?}");
    }
}
