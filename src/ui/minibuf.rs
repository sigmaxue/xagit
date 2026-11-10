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
    let Some(rest) = cand.strip_prefix(pieces[0].as_str()) else { return false };
    let mut rest = rest;
    for p in &pieces[1..] {
        match rest.find(p.as_str()) {
            Some(i) => rest = &rest[i + p.len()..],
            None => return false,
        }
    }
    true
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
pub fn completion_rows(cands: &[String], width: usize) -> Vec<String> {
    let mut sorted: Vec<&String> = cands.iter().collect();
    sorted.sort();
    let mut rows = vec![format!("{} possible completions:", sorted.len())];
    let col = sorted.iter().map(|c| c.chars().count()).max().unwrap_or(0) + 2;
    let per_row = (width / col.max(1)).max(1);
    for chunk in sorted.chunks(per_row) {
        let mut row = String::new();
        for (i, c) in chunk.iter().enumerate() {
            row.push_str(c);
            if i + 1 < chunk.len() {
                row.push_str(&" ".repeat(col - c.chars().count()));
            }
        }
        rows.push(row);
    }
    rows
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
