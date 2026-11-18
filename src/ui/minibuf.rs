//! ui/minibuf.rs — reading a line in the echo area, like the Emacs
//! minibuffer: the usual editing keys, and M-p / M-n (and the arrows)
//! moving through a history list (previous-history-element and
//! next-history-element), M-n past the newest input giving the default
//! (the "future history").
//!
//! With a completion table (completing-read) TAB completes the input
//! (minibuffer-complete, the basic then the partial-completion style), `?`
//! lists the possible completions (minibuffer-completion-help), and with
//! require-match RET only exits with one of them
//! (minibuffer-complete-and-exit).

use super::keys::Key;

/// The completion table of a completing-read.
#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub candidates: Vec<String>,
    /// REQUIRE-MATCH t: RET only exits with a candidate (or empty input)
    pub require_match: bool,
    /// completing-read-multiple: the input is a list separated by commas,
    /// the last element is completed
    pub multiple: bool,
}

/// What completing the input did (completion--do-completion).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    NoMatch,
    /// the input was the only completion already
    Sole,
    /// completed to the only completion
    Finished,
    /// a completion, but not the only one (COMPLETED: the input changed)
    Exact { completed: bool },
    /// not (yet) a completion
    Partial,
}

/// The characters partial-completion treats as word delimiters
/// (completion-pcm-word-delimiters).
const PCM_DELIMITERS: &[char] = &['-', '_', '.', '/', ':', '|', ' '];

/// Whether CAND matches the partial-completion pattern INPUT: each
/// delimiter in INPUT may be preceded by any text in CAND, `*` matches
/// any text, and the start is anchored ("o/m" matches "origin/master").
fn pcm_match(input: &str, cand: &str) -> bool {
    pcm_ranges(input, cand).is_some()
}

/// The byte ranges of CAND that INPUT matches, in order (None when it
/// does not match). The ranges are what the minibuffer paints in the
/// candidate's accent color.
fn pcm_ranges(input: &str, cand: &str) -> Option<Vec<(usize, usize)>> {
    let mut pieces: Vec<String> = vec![String::new()];
    for c in input.chars() {
        if c == '*' {
            pieces.push(String::new());
        } else if PCM_DELIMITERS.contains(&c) {
            pieces.push(c.to_string());
        } else if let Some(p) = pieces.last_mut() {
            p.push(c);
        }
    }
    let head = pieces[0].as_str();
    if !cand.starts_with(head) {
        return None;
    }
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    if !head.is_empty() {
        ranges.push((0, head.len()));
    }
    let mut at = head.len();
    for p in &pieces[1..] {
        if p.is_empty() {
            continue;
        }
        let i = cand[at..].find(p.as_str())?;
        let start = at + i;
        ranges.push((start, start + p.len()));
        at = start + p.len();
    }
    Some(ranges)
}

/// The parts of TEXT, each marked as matched by INPUT or not. The input
/// is matched with the same partial-completion rules as completion, so
/// what is highlighted is exactly what made the candidate match.
pub fn highlight_parts<'a>(text: &'a str, input: &str) -> Vec<(&'a str, bool)> {
    let Some(ranges) = pcm_ranges(input, text) else {
        return vec![(text, false)];
    };
    let mut out = Vec::new();
    let mut at = 0usize;
    for (start, end) in ranges {
        let start = start.max(at).min(text.len());
        let end = end.max(start).min(text.len());
        if start > at {
            out.push((&text[at..start], false));
        }
        if end > start {
            out.push((&text[start..end], true));
        }
        at = end;
    }
    if at < text.len() {
        out.push((&text[at..], false));
    }
    if out.is_empty() {
        out.push((text, false));
    }
    out
}

fn common_prefix(words: &[&String]) -> String {
    let Some(first) = words.first() else { return String::new() };
    let mut n = first.chars().count();
    for w in &words[1..] {
        n = n.min(first.chars().zip(w.chars()).take_while(|(a, b)| a == b).count());
    }
    first.chars().take(n).collect()
}

/// The rows of the *Completions* buffer for CANDS in WIDTH columns: the
/// header ("N possible completions:") and the candidates sorted, in
/// columns across (completions-format horizontal).
pub fn completion_rows(cands: &[String], _width: usize) -> Vec<String> {
    // One candidate per row (a selectable vertical list), instead of the
    // old multi-column *Completions* grid.  C-n/C-p move a highlight over
    // these rows in the minibuffer.
    let mut sorted: Vec<&String> = cands.iter().collect();
    sorted.sort();
    let mut rows = vec![format!("{} possible completions:", sorted.len())];
    for c in sorted {
        rows.push(format!("  {c}"));
    }
    rows
}

/// One row of the panel above the minibuffer prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelLine {
    /// the count, dimmed, with the keys and position at the right
    Header(String),
    /// a candidate, with the list cursor on it or not
    Item { text: String, selected: bool },
    /// a line of help text (read-multiple-choice)
    Note(String),
}

/// The panel above the prompt. POSITION is the 1-based place of the
/// selection and the number of candidates; HINT names the keys that
/// move it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionPanel {
    pub lines: Vec<PanelLine>,
    pub position: Option<(usize, usize)>,
    pub hint: Option<&'static str>,
}

impl CompletionPanel {
    fn empty() -> CompletionPanel {
        CompletionPanel { lines: Vec::new(), position: None, hint: None }
    }
}

/// The rows of ED's completions panel, in ROOM screen rows. A navigated
/// list is a window onto the candidates, centred on the selection so
/// C-n/C-p never move the highlight out of view; the other lists keep
/// only as many rows as fit. ABOVE is the help text shown when there is
/// no completion list.
pub fn completion_panel(ed: &LineEditor, above: &[String], room: usize) -> CompletionPanel {
    if room == 0 {
        return CompletionPanel::empty();
    }
    if ed.list_nav() {
        let shown = ed.completions().unwrap_or_default();
        let Some(header) = shown.first() else { return CompletionPanel::empty() };
        let all: Vec<&str> = shown
            .iter()
            .skip(1)
            .map(|r| r.strip_prefix("> ").or_else(|| r.strip_prefix("  ")).unwrap_or(r.as_str()))
            .collect();
        let total = all.len();
        if total == 0 {
            return CompletionPanel::empty();
        }
        let sel = ed.nav_sel.min(total - 1);
        let body = room - 1;
        let start = if total <= body { 0 } else { sel.saturating_sub(body / 2).min(total - body) };
        let mut lines = vec![PanelLine::Header(header.clone())];
        for (i, text) in all.iter().enumerate().skip(start).take(body) {
            lines.push(PanelLine::Item { text: (*text).to_string(), selected: i == sel });
        }
        return CompletionPanel { lines, position: Some((sel + 1, total)), hint: Some("C-n/C-p  RET") };
    }
    if let Some(cands) = ed.completions() {
        let rows = completion_rows(cands, 0);
        let mut it = rows.iter();
        let Some(header) = it.next() else { return CompletionPanel::empty() };
        let mut lines = vec![PanelLine::Header(header.clone())];
        for r in it.take(room - 1) {
            lines.push(PanelLine::Item { text: r.strip_prefix("  ").unwrap_or(r.as_str()).to_string(), selected: false });
        }
        return CompletionPanel { lines, position: None, hint: None };
    }
    CompletionPanel {
        lines: above.iter().take(room).map(|s| PanelLine::Note(s.clone())).collect(),
        position: None,
        hint: None,
    }
}

/// What a key did in the minibuffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// keep reading
    Continue,
    /// RET: the input (empty: the caller uses the default)
    Done(String),
    /// C-g / ESC: keyboard-quit
    Abort,
    /// keep reading, showing MSG after the input (minibuffer-message)
    Message(String),
}

/// The state of one minibuffer read.
#[derive(Debug, Clone)]
pub struct LineEditor {
    text: Vec<char>,
    point: usize,
    /// the history list, newest first
    history: Vec<String>,
    /// minibuffer-history-position: 0 the input, N the Nth history
    /// element, -1 the default
    pos: isize,
    /// the input before moving into the history
    input: String,
    default: Option<String>,
    table: Option<Completion>,
    /// the possible completions shown (the *Completions* buffer)
    shown: Option<Vec<String>>,
    /// the last key completed (TAB again shows the completions)
    last_complete: bool,
    /// select-from-list: C-n/C-p move a cursor over the candidate list
    /// (one candidate per row).  `nav_armed` means a list can be opened;
    /// `nav` means it is currently shown and actively navigated.
    nav: bool,
    nav_armed: bool,
    nav_list: Vec<String>,
    nav_sel: usize,
    /// a C-n/C-p key was used: RET should take the highlighted row
    nav_chosen: bool,
}

impl LineEditor {
    pub fn new(history: Vec<String>, default: Option<String>) -> LineEditor {
        LineEditor {
            text: Vec::new(),
            point: 0,
            history,
            pos: 0,
            input: String::new(),
            default,
            table: None,
            shown: None,
            last_complete: false,
            nav: false,
            nav_armed: false,
            nav_list: Vec::new(),
            nav_sel: 0,
            nav_chosen: false,
        }
    }

    /// Start with the input INITIAL, point after it (INITIAL-INPUT).
    pub fn with_initial_input(mut self, initial: &str) -> LineEditor {
        self.text = initial.chars().collect();
        self.point = self.text.len();
        self
    }

    /// Read with completion against TABLE (completing-read).
    pub fn with_completion(mut self, mut table: Completion) -> LineEditor {
        let mut seen = std::collections::HashSet::new();
        table.candidates.retain(|c| seen.insert(c.clone()));
        self.table = Some(table);
        self
    }

    /// Offer the candidates as a vertical, C-n/C-p navigable single-column
    /// list.  The list is shown from the start but the input stays empty:
    /// typing a value works exactly as before (no preselected text to
    /// append to).  C-n/C-p highlight a row; RET then takes that candidate.
    pub fn enable_list_nav(mut self) -> LineEditor {
        let mut list = self.table.as_ref().map(|t| t.candidates.clone()).unwrap_or_default();
        list.sort();
        self.nav_list = list;
        self.nav_armed = !self.nav_list.is_empty();
        self.nav = self.nav_armed;
        self.nav_sel = 0;
        self.nav_apply();
        self
    }

    /// Whether the current completions panel is a one-per-row select list.
    pub fn list_nav(&self) -> bool {
        self.nav
    }

    /// Refresh the list rows with the highlighted entry marked.  The input
    /// is NOT overwritten, so free typing is unaffected.
    fn nav_apply(&mut self) {
        if !self.nav || self.nav_list.is_empty() {
            return;
        }
        self.nav_sel = self.nav_sel.min(self.nav_list.len() - 1);
        let mut rows = vec![format!("{} possible completions:", self.nav_list.len())];
        for (i, c) in self.nav_list.iter().enumerate() {
            rows.push(format!("{} {}", if i == self.nav_sel { ">" } else { " " }, c));
        }
        self.shown = Some(rows);
    }

    /// The possible completions shown (the *Completions* buffer), if any.
    pub fn completions(&self) -> Option<&[String]> {
        self.shown.as_deref()
    }

    /// Where the input being completed starts: after the last separator
    /// with completing-read-multiple (crm-separator).
    fn field_start(&self) -> usize {
        if !self.table.as_ref().is_some_and(|t| t.multiple) {
            return 0;
        }
        let mut i = self.text.iter().rposition(|&c| c == ',').map_or(0, |p| p + 1);
        while i < self.text.len() && self.text[i].is_whitespace() {
            i += 1;
        }
        i
    }

    fn field(&self) -> String {
        self.text[self.field_start()..].iter().collect()
    }

    fn set_field(&mut self, s: &str) {
        self.text.truncate(self.field_start());
        self.text.extend(s.chars());
        self.point = self.text.len();
    }

    fn candidates(&self) -> &[String] {
        self.table.as_ref().map(|t| t.candidates.as_slice()).unwrap_or(&[])
    }

    /// all-completions of INPUT: the candidates it is a prefix of (the
    /// basic style), else those matching it as partial-completion.
    fn all_completions(&self, input: &str) -> (Vec<&String>, bool) {
        let basic: Vec<&String> = self.candidates().iter().filter(|c| c.starts_with(input)).collect();
        if !basic.is_empty() {
            return (basic, true);
        }
        (self.candidates().iter().filter(|c| pcm_match(input, c)).collect(), false)
    }

    /// completion--do-completion: complete the input as far as possible.
    fn do_completion(&mut self) -> (Outcome, Step) {
        let input = self.field();
        let (matches, basic) = self.all_completions(&input);
        if matches.is_empty() {
            return (Outcome::NoMatch, Step::Message("[No match]".into()));
        }
        let new = match (basic, matches.len()) {
            (true, _) => common_prefix(&matches),
            (false, 1) => matches[0].clone(),
            (false, _) => input.clone(),
        };
        let completed = new != input;
        if completed {
            self.set_field(&new);
        }
        let exact = self.candidates().contains(&new);
        let (now, _) = self.all_completions(&new);
        let now: Vec<String> = now.into_iter().cloned().collect();
        if exact && now.len() == 1 {
            self.shown = None;
            return if completed {
                (Outcome::Finished, Step::Continue)
            } else {
                (Outcome::Sole, Step::Message("[Sole completion]".into()))
            };
        }
        if exact {
            if (!completed && self.last_complete) || self.shown.is_some() {
                self.shown = Some(now);
            }
            return (Outcome::Exact { completed }, Step::Message("[Complete, but not unique]".into()));
        }
        // completion-auto-help: no progress shows the completions
        if !completed || self.shown.is_some() {
            self.shown = Some(now);
        }
        (Outcome::Partial, Step::Continue)
    }

    /// minibuffer-complete-and-exit (RET with require-match).
    fn complete_and_exit(&mut self) -> Step {
        let text = self.text();
        if text.is_empty() || self.candidates().contains(&text) {
            return Step::Done(text);
        }
        match self.do_completion() {
            (Outcome::Sole | Outcome::Finished, _) => Step::Done(self.text()),
            (Outcome::Exact { completed: true }, _) => Step::Message("[Confirm]".into()),
            (_, step) => step,
        }
    }

    /// minibuffer-completion-help (`?`)
    fn completion_help(&mut self) -> Step {
        let input = self.field();
        let (matches, _) = self.all_completions(&input);
        match matches.as_slice() {
            [] => {
                self.shown = None;
                Step::Message("[No match]".into())
            }
            [only] if **only == input => {
                self.shown = None;
                Step::Message("[Sole completion]".into())
            }
            _ => {
                self.shown = Some(matches.into_iter().cloned().collect());
                Step::Continue
            }
        }
    }

    pub fn text(&self) -> String {
        self.text.iter().collect()
    }

    /// The part of the input matched against the candidates (the last
    /// field with completing-read-multiple).
    pub fn completion_input(&self) -> String {
        self.field()
    }

    /// The column of point in the input.
    pub fn point(&self) -> usize {
        self.point
    }

    fn set_text(&mut self, s: &str) {
        self.text = s.chars().collect();
        self.point = self.text.len();
    }

    /// goto-history-element N
    fn goto_history(&mut self, n: isize) -> Step {
        if n > self.history.len() as isize {
            return Step::Message("Beginning of history; no preceding item".into());
        }
        if n < 0 {
            let Some(d) = self.default.clone().filter(|_| n == -1) else {
                let msg = if self.default.is_some() { "End of defaults" } else { "End of history; no default available" };
                return Step::Message(msg.into());
            };
            if self.pos == 0 {
                self.input = self.text();
            }
            self.pos = n;
            self.set_text(&d);
            return Step::Continue;
        }
        if self.pos == 0 {
            self.input = self.text();
        }
        self.pos = n;
        let s = if n == 0 { self.input.clone() } else { self.history[n as usize - 1].clone() };
        self.set_text(&s);
        Step::Continue
    }

    fn word_end(&self) -> usize {
        let mut i = self.point;
        while i < self.text.len() && !self.text[i].is_alphanumeric() {
            i += 1;
        }
        while i < self.text.len() && self.text[i].is_alphanumeric() {
            i += 1;
        }
        i
    }

    fn word_start(&self) -> usize {
        let mut i = self.point;
        while i > 0 && !self.text[i - 1].is_alphanumeric() {
            i -= 1;
        }
        while i > 0 && self.text[i - 1].is_alphanumeric() {
            i -= 1;
        }
        i
    }

    /// Handle KEY (minibuffer-local-map and the global editing keys).
    pub fn key(&mut self, key: &Key) -> Step {
        let last_complete = std::mem::take(&mut self.last_complete);
        // Single-column select list: C-n/C-p (and up/down) move the
        // highlight.  The list is shown but the input is left alone, so
        // typing works as before.  RET takes the highlighted candidate
        // only once a navigation key was used; otherwise it exits with
        // the typed text.  M-p/M-n stay bound to minibuffer history.
        if self.nav {
            match (key.ctrl, key.code.as_str()) {
                (true, "n") | (false, "down") | (true, "p") | (false, "up") => {
                    let down = key.code == "n" || key.code == "down";
                    if down {
                        self.nav_sel = (self.nav_sel + 1) % self.nav_list.len();
                    } else {
                        self.nav_sel = (self.nav_sel + self.nav_list.len() - 1) % self.nav_list.len();
                    }
                    self.nav_chosen = true;
                    self.nav_apply();
                    return Step::Continue;
                }
                // RET takes the highlighted row only after navigating;
                // otherwise the normal path applies (empty input uses the
                // default, typed text is completed as usual)
                (false, "enter") | (true, "j") | (true, "m") if self.nav_chosen => {
                    return Step::Done(self.nav_list[self.nav_sel].clone());
                }
                // C-g / ESC quit; plain RET falls through to normal exit
                (true, "g") | (false, "esc") | (false, "enter") | (true, "j") | (true, "m") => {}
                // TAB, ?, M-p/M-n, cursor edits and typing leave list
                // mode so the value is not prefixed by a candidate
                _ => {
                    self.nav = false;
                    self.nav_chosen = false;
                    self.shown = None;
                }
            }
        }
        if let Some(t) = &self.table {
            let require_match = t.require_match && !t.multiple;
            match (key.ctrl, key.code.as_str()) {
                (false, "tab") | (true, "i") => {
                    self.last_complete = last_complete;
                    let (_, step) = self.do_completion();
                    self.last_complete = true;
                    return step;
                }
                (false, "?") => return self.completion_help(),
                (false, "enter") | (true, "j") | (true, "m") if require_match => return self.complete_and_exit(),
                _ => {}
            }
        }
        match (key.ctrl, key.code.as_str()) {
            (false, "enter") | (true, "j") | (true, "m") => Step::Done(self.text()),
            (true, "g") | (false, "esc") => Step::Abort,
            (false, "M-p") | (false, "up") => self.goto_history(self.pos + 1),
            (false, "M-n") | (false, "down") => self.goto_history(self.pos - 1),
            (true, "a") | (false, "home") => {
                self.point = 0;
                Step::Continue
            }
            (true, "e") | (false, "end") => {
                self.point = self.text.len();
                Step::Continue
            }
            (true, "b") | (false, "left") => {
                self.point = self.point.saturating_sub(1);
                Step::Continue
            }
            (true, "f") | (false, "right") => {
                self.point = (self.point + 1).min(self.text.len());
                Step::Continue
            }
            (false, "M-b") => {
                self.point = self.word_start();
                Step::Continue
            }
            (false, "M-f") => {
                self.point = self.word_end();
                Step::Continue
            }
            (false, "backspace") | (true, "h") => {
                if self.point > 0 {
                    self.point -= 1;
                    self.text.remove(self.point);
                }
                Step::Continue
            }
            (true, "d") => {
                if self.point < self.text.len() {
                    self.text.remove(self.point);
                }
                Step::Continue
            }
            (false, "M-d") => {
                let end = self.word_end();
                self.text.drain(self.point..end);
                Step::Continue
            }
            (false, "M-backspace") | (true, "w") => {
                let start = self.word_start();
                self.text.drain(start..self.point);
                self.point = start;
                Step::Continue
            }
            (true, "k") => {
                self.text.truncate(self.point);
                Step::Continue
            }
            (false, c) if c.chars().count() == 1 => {
                let ch = c.chars().next().unwrap_or(' ');
                self.text.insert(self.point, ch);
                self.point += 1;
                Step::Continue
            }
            _ => Step::Message(format!("{} is undefined", key.label())),
        }
    }
}

/// add-to-history: INPUT becomes the newest element of HIST (unless it is
/// empty, or the newest already); with DEDUP the whole list is then
/// without duplicates (transient's delete-dups).
pub fn add_to_history(hist: &mut Vec<String>, input: &str, dedup: bool) {
    if input.is_empty() {
        return;
    }
    if dedup {
        hist.insert(0, input.to_string());
        let mut seen = std::collections::HashSet::new();
        hist.retain(|h| seen.insert(h.clone()));
        return;
    }
    if hist.first().is_some_and(|h| h == input) {
        return;
    }
    hist.insert(0, input.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::keys::key_from_label;

    fn nav(cands: &[&str]) -> LineEditor {
        let table = Completion {
            candidates: cands.iter().map(|s| s.to_string()).collect(),
            require_match: true,
            multiple: false,
        };
        LineEditor::new(Vec::new(), None).with_completion(table).enable_list_nav()
    }

    #[test]
    fn pcm_ranges_mark_what_the_input_matched() {
        assert_eq!(pcm_ranges("mas", "master"), Some(vec![(0, 3)]));
        assert_eq!(pcm_ranges("o/m", "origin/master"), Some(vec![(0, 1), (6, 8)]));
        assert_eq!(pcm_ranges("", "master"), Some(vec![]));
        assert_eq!(pcm_ranges("zz", "master"), None);
        assert_eq!(highlight_parts("master", "mas"), [("mas", true), ("ter", false)]);
    }

    #[test]
    fn panel_keeps_the_selection_in_view() {
        let cands: Vec<String> = ('a'..='l').map(|c| c.to_string()).collect();
        let refs: Vec<&str> = cands.iter().map(String::as_str).collect();
        let mut ed = nav(&refs);
        ed.nav_sel = 10;
        ed.nav_apply();
        let p = completion_panel(&ed, &[], 6);
        assert_eq!(p.position, Some((11, 12)));
        assert_eq!(p.hint, Some("C-n/C-p  RET"));
        assert_eq!(p.lines.len(), 6);
        assert_eq!(p.lines[0], PanelLine::Header("12 possible completions:".into()));
        assert!(p.lines.contains(&PanelLine::Item { text: "k".into(), selected: true }));
    }

    #[test]
    fn panel_falls_back_to_help_lines() {
        let ed = LineEditor::new(Vec::new(), None);
        let p = completion_panel(&ed, &["first".into(), "second".into()], 1);
        assert_eq!(p.lines, vec![PanelLine::Note("first".into())]);
        assert_eq!(p.position, None);
    }

    #[test]
    fn completion_list_has_a_header_and_sorted_items() {
        let mut ed = LineEditor::new(Vec::new(), None).with_completion(Completion {
            candidates: vec!["bb".into(), "a".into(), "ccc".into()],
            require_match: false,
            multiple: false,
        });
        ed.key(&key_from_label("?"));
        assert_eq!(
            completion_panel(&ed, &[], 4).lines,
            vec![
                PanelLine::Header("3 possible completions:".into()),
                PanelLine::Item { text: "a".into(), selected: false },
                PanelLine::Item { text: "bb".into(), selected: false },
                PanelLine::Item { text: "ccc".into(), selected: false },
            ]
        );
    }
}
