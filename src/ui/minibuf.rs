//! ui/minibuf.rs — reading a line in the echo area, like the Emacs
//! minibuffer: the usual editing keys, and M-p / M-n (and the arrows)
//! moving through a history list (previous-history-element and
//! next-history-element), M-n past the newest input giving the default
//! (the "future history").

use super::keys::Key;

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
}

impl LineEditor {
    pub fn new(history: Vec<String>, default: Option<String>) -> LineEditor {
        LineEditor { text: Vec::new(), point: 0, history, pos: 0, input: String::new(), default }
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
