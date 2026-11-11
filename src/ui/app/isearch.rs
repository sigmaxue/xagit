//! C-s / C-r: searching the buffer shown.

use super::*;

impl App {
    /// The query highlighted in the current buffer, if a search is active.
    pub fn search_query(&self) -> Option<&str> {
        self.search_query.as_deref()
    }

    /// The visible lines matching QUERY, including the right margin. The
    /// list is rebuilt for every step: a refresh or a section toggle can
    /// replace `lines`, so caching line numbers would go stale.
    fn search_matches(&self, query: &str) -> Vec<usize> {
        let needle = folded_bytes(query);
        if needle.is_empty() {
            return Vec::new();
        }
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                let mut text = l.text.clone();
                if !l.margin.is_empty() {
                    text.push(' ');
                    text.push_str(&l.margin);
                }
                let folded = folded_bytes(&text);
                folded.windows(needle.len()).any(|w| w == needle.as_slice())
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Whether KEY ends the search prompt with the text typed so far.
    /// Like Emacs' isearch, C-s / C-r while entering a query submit it, so
    /// repeating the key searches without a RET in between; the key also
    /// decides the direction. Only active while `isearch` reads a query.
    pub(super) fn search_submit_key(&self, key: &Key) -> bool {
        let mut slot = self.search_read.borrow_mut();
        if *slot != Some(SearchRead::Reading) {
            return false;
        }
        *slot = Some(match (key.ctrl, key.code.as_str()) {
            (true, "s") => SearchRead::Forward,
            (true, "r") => SearchRead::Backward,
            _ => return false,
        });
        true
    }

    /// C-s / C-r: read a query on the first call (or with a prefix arg),
    /// then move point through the matches of the current buffer.
    pub(super) fn isearch(&mut self, forward: bool) -> Result<()> {
        if self.view_text.is_some() || self.mode == BufferMode::RebaseTodo || self.mode == BufferMode::Resolve {
            self.set_msg("Search is only available in Magit buffers");
            return Ok(());
        }
        let repeat = self.search_query.is_some() && self.prefix_arg.is_none();
        let (query, forward) = if repeat {
            (self.search_query.clone().unwrap_or_default(), forward)
        } else {
            let default = (!self.last_search.is_empty()).then_some(self.last_search.as_str());
            *self.search_read.borrow_mut() = Some(SearchRead::Reading);
            let read = self.read_line_prompt("Search: ", default);
            // take before `?`: the prompt must not stay active on an error
            let direction = match self.search_read.borrow_mut().take() {
                Some(SearchRead::Forward) => true,
                Some(SearchRead::Backward) => false,
                _ => forward,
            };
            match read? {
                Some(q) => (q, direction),
                None => {
                    self.clear_search_state();
                    self.set_msg("Search cancelled");
                    return Ok(());
                }
            }
        };
        if query.is_empty() {
            self.clear_search_state();
            self.set_msg("Search cancelled");
            return Ok(());
        }
        self.last_search = query.clone();
        self.search_query = Some(query.clone());
        let matches = self.search_matches(&query);
        self.search_goto(&matches, forward, !repeat, &query);
        Ok(())
    }

    /// Move point to the next match, wrapping around at either end.
    fn search_goto(&mut self, matches: &[usize], forward: bool, include_current: bool, query: &str) {
        let short: String = query.chars().take(40).collect();
        if matches.is_empty() {
            self.set_msg(&format!("[no match] {short}"));
            return;
        }
        let cur = self.cursor;
        let next = if forward {
            matches
                .iter()
                .copied()
                .find(|&i| if include_current { i >= cur } else { i > cur })
                .or_else(|| matches.first().copied())
        } else {
            matches
                .iter()
                .rev()
                .copied()
                .find(|&i| if include_current { i <= cur } else { i < cur })
                .or_else(|| matches.last().copied())
        };
        if let Some(i) = next {
            self.cursor = i;
        }
        let n = matches.len();
        self.set_msg(&format!("{n} matches for {short}"));
    }
}
