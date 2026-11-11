//! ui/app/input.rs — minibuffer prompts, completions, history and yes/no questions.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

impl App {

    /// The prompts read so far, oldest first.
    pub fn prompts(&self) -> Vec<String> {
        self.prompts.borrow().clone()
    }

    /// Answers typed as KEYS (key labels) into the minibuffer of the
    /// next prompts, one answer per call, after the scripted answers.
    pub fn script_keys(&self, keys: &[&str]) {
        self.scripted_keys.borrow_mut().push_back(keys.iter().map(|k| key_from_label(k)).collect());
    }

    /// The values read by the infix of the argument ARG, newest first.
    pub fn infix_history(&self, arg: &str) -> Vec<String> {
        self.infix_history.borrow().get(arg).cloned().unwrap_or_default()
    }

    /// The values the prefix KEY was used with, newest first.
    pub fn prefix_history(&self, key: &str) -> Vec<Vec<String>> {
        self.history.get(key).cloned().unwrap_or_default()
    }

    /// minibuffer-history, newest first.
    pub fn minibuffer_history(&self) -> Vec<String> {
        self.minibuffer_history.borrow().clone()
    }

    /// magit-revision-history, newest first.
    pub fn revision_history(&self) -> Vec<String> {
        self.revision_history.borrow().clone()
    }

    /// transient-history-file: read the history saved by earlier sessions
    /// (merged after the values used in this one), and save to FILE when
    /// quitting.
    pub fn load_history(&mut self, file: std::path::PathBuf) {
        let h = crate::ui::transient::read_history(&file);
        for (k, vs) in h.prefixes {
            let e = self.history.entry(k).or_default();
            for v in vs {
                if !e.contains(&v) {
                    e.push(v);
                }
            }
        }
        let mut ih = self.infix_history.borrow_mut();
        for (k, vs) in h.infixes {
            let e = ih.entry(k).or_default();
            for v in vs {
                if !e.contains(&v) {
                    e.push(v);
                }
            }
        }
        drop(ih);
        self.history_file = Some(file);
    }

    /// transient-maybe-save-history (kill-emacs-hook)
    pub fn save_history(&mut self) {
        let Some(f) = &self.history_file else { return };
        let h = crate::ui::transient::History {
            prefixes: self.history.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            infixes: self.infix_history.borrow().clone(),
        };
        if let Err(e) = crate::ui::transient::write_history(f, &h) {
            let msg = format!("Error saving transient history: {e}");
            self.set_msg(&msg);
        }
    }

    /// Answers consumed by prompts instead of reading the terminal (tests).
    pub fn script_answers(&mut self, answers: &[&str]) {
        self.scripted.borrow_mut().extend(answers.iter().map(|s| s.to_string()));
    }

    pub(super) fn read_line_prompt(&self, prompt: &str, default: Option<&str>) -> Result<Option<String>> {
        Ok(self.read_line_raw(prompt, default)?.filter(|s| !s.is_empty()))
    }

    /// Read the value of the infix of ARG (with its history), trimmed, the
    /// DEFAULT when empty.
    pub(super) fn read_infix(&self, prompt: &str, default: Option<&str>, arg: &str) -> Result<Option<String>> {
        Ok(self.read_line_hist(prompt, default, Hist::Infix(arg), None)?.map(|i| {
            let s = i.trim().to_string();
            match default {
                Some(d) if s.is_empty() => d.to_string(),
                _ => s,
            }
        }))
    }

    /// Read a line (the DEFAULT when empty) with minibuffer-history; only
    /// EOF and C-g give none.
    pub(super) fn read_line_raw(&self, prompt: &str, default: Option<&str>) -> Result<Option<String>> {
        self.completing_read(prompt, default, Hist::Minibuffer, None)
    }

    /// completing-read: read a line with completion against TABLE (none:
    /// read-string), M-p / M-n moving through HIST, to which the input is
    /// added (the DEFAULT for an empty input, like read-from-minibuffer).
    /// The input trimmed, the DEFAULT when empty; only EOF and C-g give
    /// none.
    pub(super) fn completing_read(
        &self,
        prompt: &str,
        default: Option<&str>,
        hist: Hist,
        table: Option<Completion>,
    ) -> Result<Option<String>> {
        let Some(input) = self.read_line_hist(prompt, default, hist, table)? else {
            return Ok(None);
        };
        let s = input.trim().to_string();
        let s = match default {
            Some(d) if s.is_empty() => d.to_string(),
            _ => s,
        };
        let list = match hist {
            Hist::Minibuffer => Some(&self.minibuffer_history),
            Hist::Revision => Some(&self.revision_history),
            Hist::Infix(_) | Hist::None => None,
        };
        if let Some(l) = list {
            add_to_history(&mut l.borrow_mut(), &s, false);
        }
        Ok(Some(s))
    }

    /// Read a branch or commit (magit-read-branch-or-commit and friends):
    /// completing the names of the refs, with magit-revision-history.
    pub(super) fn read_rev(&self, prompt: &str, default: Option<&str>) -> Result<Option<String>> {
        let table = Completion { candidates: self.git.refnames(), ..Default::default() };
        Ok(self.completing_read(prompt, default, Hist::Revision, Some(table))?.filter(|s| !s.is_empty()))
    }

    /// Read the input of a line in the minibuffer, M-p / M-n moving
    /// through the history HIST, completing against TABLE. The input isn't
    /// added to the history.
    pub(super) fn read_line_hist(
        &self,
        prompt: &str,
        default: Option<&str>,
        hist: Hist,
        table: Option<Completion>,
    ) -> Result<Option<String>> {
        self.prompts.borrow_mut().push(prompt.to_string());
        let initial = self.initial_input.borrow_mut().take();
        if let Some(ans) = self.scripted.borrow_mut().pop_front() {
            return Ok(Some(ans));
        }
        let history = match hist {
            Hist::Infix(k) => self.infix_history.borrow().get(k).cloned().unwrap_or_default(),
            Hist::Minibuffer => self.minibuffer_history.borrow().clone(),
            Hist::Revision => self.revision_history.borrow().clone(),
            Hist::None => Vec::new(),
        };
        let mut ed = LineEditor::new(history, default.map(str::to_string));
        if let Some(t) = table {
            ed = ed.with_completion(t);
        }
        if let Some(i) = initial {
            ed = ed.with_initial_input(&i);
        }
        if let Some(keys) = self.scripted_keys.borrow_mut().pop_front() {
            for k in &keys {
                if let Some(a) = self.instant_answer(k) {
                    return Ok(Some(a));
                }
                match ed.key(k) {
                    Step::Done(s) => return Ok(Some(s)),
                    Step::Abort => return Ok(None),
                    Step::Continue | Step::Message(_) => {}
                }
            }
            return Ok(Some(ed.text()));
        }
        if self.in_terminal {
            return self.minibuffer(prompt, default, ed);
        }
        {
            // Leave raw mode temporarily to read a normal line.
            let _ = disable_raw_mode();
            let mut out = stdout();
            let (_, h) = crossterm::terminal::size().unwrap_or((80, 24));
            let _ = execute!(
                out,
                crossterm::cursor::MoveTo(0, h.saturating_sub(1)),
                Clear(ClearType::CurrentLine),
                crossterm::style::Print(prompt),
                crossterm::style::Print(match default {
                    _ if prompt.ends_with(' ') => String::new(),
                    Some(d) => format!(" (default: {d}): "),
                    None => ": ".into(),
                })
            );
            let _ = out.flush();
            let mut buf = String::new();
            let result = std::io::stdin().read_line(&mut buf);
            let _ = enable_raw_mode();
            let n = result.map_err(|e| anyhow::anyhow!("Failed to read prompt input: {e}"))?;
            if n == 0 {
                return Ok(None); // EOF / C-d
            }
            Ok(Some(buf.trim_end_matches(['\n', '\r']).to_string()))
        }
    }

    /// The minibuffer in the terminal: the prompt and the input on the
    /// last line, a message after the input until the next key, the
    /// possible completions (if shown) in the rows above it.
    pub(super) fn minibuffer(&self, prompt: &str, default: Option<&str>, mut ed: LineEditor) -> Result<Option<String>> {
        let prompt = match default {
            // a prompt ending with a space is shown as is (y-or-n-p,
            // read-char-choice); others get Magit's ": "
            _ if prompt.ends_with(' ') => prompt.to_string(),
            Some(d) => format!("{prompt} (default: {d}): "),
            None => format!("{prompt}: "),
        };
        let mut msg = String::new();
        // the rows above the minibuffer covered by the completions
        let mut covered = 0usize;
        let t = &self.theme;
        let result = loop {
            let mut out = stdout();
            let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));
            let (w, h) = (w as usize, h as usize);
            let list = match ed.completions() {
                Some(c) => {
                    let mut rows = crate::ui::minibuf::completion_rows(c, w);
                    rows.truncate(h.saturating_sub(1) / 2);
                    rows
                }
                None => {
                    let mut rows: Vec<String> = self.minibuffer_above.borrow().clone();
                    rows.truncate(h.saturating_sub(1));
                    rows
                }
            };
            let last = h.saturating_sub(1);
            self.repaint_rows(last.saturating_sub(covered), last.saturating_sub(list.len()));
            // the completions on a panel above the prompt
            let panel = t.style(Face::SectionHighlight);
            for (i, r) in list.iter().enumerate() {
                let y = last - list.len() + i;
                let mut row = Row::new(panel);
                row.push(r.clone(), Style::default().over(panel));
                let _ = paint::write_row(&mut out, t, y as u16, &row, w);
            }
            covered = list.len();
            let text = ed.text();
            let mut row = Row::new(Style::default());
            row.push(prompt.clone(), t.style(Face::Prompt)).push(text.clone(), Style::default());
            if !msg.is_empty() {
                row.push(format!(" {msg}"), t.style(Face::Dim));
            }
            let before: String = text.chars().take(ed.point()).collect();
            let col = crate::ui::width::str_width(&prompt) + crate::ui::width::str_width(&before);
            let _ = paint::write_row(&mut out, t, last as u16, &row, w);
            let _ = execute!(
                out,
                crossterm::cursor::MoveTo(col.min(w.saturating_sub(1)) as u16, last as u16),
                crossterm::cursor::Show
            );
            let _ = out.flush();
            let Event::Key(ev) = event::read()? else { continue };
            if ev.kind == crossterm::event::KeyEventKind::Release {
                continue;
            }
            msg.clear();
            let key = Key::from_event(&ev);
            if let Some(a) = self.instant_answer(&key) {
                break Some(a);
            }
            match ed.key(&key) {
                Step::Done(s) => break Some(s),
                Step::Abort => break None,
                Step::Message(m) if m.starts_with('[') => msg = m,
                Step::Message(m) => msg = format!("[{m}]"),
                Step::Continue => {}
            }
        };
        // restore the rows the completions covered
        let (_, h) = crossterm::terminal::size().unwrap_or((80, 24));
        let last = (h as usize).saturating_sub(1);
        self.repaint_rows(last.saturating_sub(covered), last);
        let _ = execute!(stdout(), crossterm::cursor::Hide);
        Ok(result)
    }

    /// The answer KEY gives at once to the question being read.
    pub(super) fn instant_answer(&self, key: &Key) -> Option<String> {
        let keys = self.instant_keys.borrow();
        if keys.iter().any(|(k, _)| k == ANY_KEY) {
            // read-event: any key but C-g (quit)
            return match (key.ctrl, key.code.as_str()) {
                (true, "g") => None,
                (true, c) => Some(format!("C-{c}")),
                (false, c) => Some(c.to_string()),
            };
        }
        if key.ctrl {
            return None;
        }
        keys.iter().find(|(k, _)| *k == key.code).map(|(_, a)| a.clone())
    }

    /// read_line_hist with the KEYS answering at once.
    pub(super) fn read_instant(&self, prompt: &str, keys: Vec<(String, String)>) -> Result<Option<String>> {
        *self.instant_keys.borrow_mut() = keys;
        let r = self.read_line_hist(prompt, None, Hist::None, None);
        self.instant_keys.borrow_mut().clear();
        r
    }

    /// y-or-n-p: PROMPT and "(y or n) " (after a space unless it ends
    /// with one); `y`/`Y`/SPC answer yes, `n`/`N`/DEL no, anything else
    /// (RET too) asks again. The answer is echoed after the prompt.
    pub(super) fn yes_no(&mut self, prompt: &str) -> Result<bool> {
        let prompt = format!("{}(y or n) ", padded(prompt));
        loop {
            let keys = ["y", "Y", " ", "n", "N"].iter().map(|k| (k.to_string(), k.to_string()));
            let keys = keys.chain([("backspace".to_string(), "\u{7f}".to_string())]).collect();
            let Some(ans) = self.read_instant(&prompt, keys)? else {
                return Ok(false);
            };
            let yes = match ans.as_str() {
                "y" | "Y" | " " => true,
                "n" | "N" | "\u{7f}" => false,
                _ => {
                    self.set_msg("Please answer y or n");
                    continue;
                }
            };
            self.message = crate::git::exec::sanitize(&format!("{prompt}{}", if yes { 'y' } else { 'n' }));
            return Ok(yes);
        }
    }

    /// yes-or-no-p: PROMPT answered with "yes" or "no" (other answers
    /// ask again).
    pub(super) fn yes_or_no_p(&mut self, prompt: &str) -> Result<bool> {
        loop {
            let Some(ans) = self.read_line_hist(&format!("{}(yes or no) ", padded(prompt)), None, Hist::None, None)? else {
                return Ok(false);
            };
            match ans.trim_end_matches(['\n', '\r']).to_lowercase().as_str() {
                "yes" => return Ok(true),
                "no" => return Ok(false),
                _ => self.set_msg("Please answer yes or no."),
            }
        }
    }

    /// magit-y-or-n-p: yes-or-no-p if ACTION is in magit-slow-confirm
    /// (or that is t), else y-or-n-p.
    pub(super) fn magit_y_or_n_p(&mut self, prompt: &str, action: Option<&str>) -> Result<bool> {
        if self.options.slow_confirm(action) {
            self.yes_or_no_p(prompt)
        } else {
            self.yes_no(prompt)
        }
    }

    /// magit-completing-read of one of CHOICES (the first being the
    /// default without DEFAULT) with minibuffer-history; an input that
    /// is not a choice selects the first choice containing it.
    pub(super) fn pick<'a>(&mut self, prompt: &str, choices: &'a [String], default: Option<&str>) -> Result<Option<&'a String>> {
        self.pick_hist(prompt, choices, default, Hist::Minibuffer)
    }

    /// `pick` with magit-revision-history (magit-read-branch, -tag, ...).
    pub(super) fn pick_rev<'a>(&mut self, prompt: &str, choices: &'a [String], default: Option<&str>) -> Result<Option<&'a String>> {
        self.pick_hist(prompt, choices, default, Hist::Revision)
    }

    pub(super) fn pick_hist<'a>(
        &mut self,
        prompt: &str,
        choices: &'a [String],
        default: Option<&str>,
        hist: Hist,
    ) -> Result<Option<&'a String>> {
        if choices.is_empty() {
            self.message = "no candidates".into();
            return Ok(None);
        }
        let table = Completion { candidates: choices.to_vec(), ..Default::default() };
        let Some(inp) = self
            .completing_read(prompt, default.or(Some(&choices[0])), hist, Some(table))?
            .filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        if let Some(exact) = choices.iter().find(|c| c.as_str() == inp) {
            return Ok(Some(exact));
        }
        let hits: Vec<&String> = choices.iter().filter(|c| c.contains(inp.as_str())).collect();
        if hits.len() == 1 {
            Ok(Some(hits[0]))
        } else if hits.is_empty() {
            self.message = "no match".into();
            Ok(None)
        } else {
            Ok(Some(hits[0]))
        }
    }

    pub(super) fn set_msg(&mut self, m: &str) {
        let text = crate::git::exec::sanitize(m);
        self.message = text.lines().next().unwrap_or("").to_string();
        if text.contains('\n') || text.chars().count() > 200 {
            if text.contains('\n') { self.message.push_str(" (details: n/p scroll, q close)"); }
            self.view_text = Some(("Operation details (n/p scroll, q close)".into(), text));
            self.view_scroll = 0;
        }
    }

    /// Report a failed command with the whole error chain (anyhow's `{:#}`
    /// includes the git stderr attached as context).
    pub(super) fn report_error(&mut self, e: &anyhow::Error) {
        self.set_msg(&format!("{e:#}"));
    }

    /// Forget the region: MARK indexes the old buffer's `all` vector, so it
    /// must go away when that buffer is replaced.
    pub(super) fn clear_region(&mut self) {
        self.mark = None;
    }

    /// Forget the active search when leaving a buffer or cancelling it.
    /// A plain refresh deliberately keeps the query (see `on_refresh_done`).
    pub(super) fn clear_search_state(&mut self) {
        self.search_query = None;
    }

    /// Whether the region is active.
    pub fn region_active(&self) -> bool {
        self.mark.is_some()
    }

}
