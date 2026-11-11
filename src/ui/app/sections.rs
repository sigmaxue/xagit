//! ui/app/sections.rs — section navigation and visibility (magit-section).
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

impl App {

    pub(super) fn sec(&self, s: usize) -> &Section {
        &self.buf.sections[s]
    }

    /// The section at point (magit-current-section); 0 is the root.
    pub(super) fn cur_sec(&self) -> usize {
        self.buf.section_at(self.point())
    }

    pub(super) fn hidden(&self, s: usize) -> bool {
        s != 0 && self.buf.is_hidden(&self.vis, &self.buf.sections[s].ident)
    }

    pub(super) fn set_hidden(&mut self, s: usize, hidden: bool) {
        if s != 0 {
            let id = self.buf.sections[s].ident.clone();
            self.vis.insert(id, hidden);
        }
    }

    pub(super) fn children(&self, s: usize) -> Vec<usize> {
        self.buf.sections[s].children.clone()
    }

    pub(super) fn goto_sec(&mut self, s: usize) {
        let p = self.buf.sections[s].start;
        self.cursor = self.visible_index_near(p);
    }

    /// forward-line N (used by movement commands on the root section)
    pub(super) fn forward_line(&mut self, n: isize) {
        let max = self.lines.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + n).clamp(0, max.max(0)) as usize;
    }

    /// magit-section-siblings: the next / previous sibling of S.
    pub(super) fn sibling(&self, s: usize, next: bool) -> Option<usize> {
        let ch = &self.sec(self.sec(s).parent?).children;
        let i = ch.iter().position(|&c| c == s)?;
        if next {
            ch.get(i + 1).copied()
        } else {
            i.checked_sub(1).map(|j| ch[j])
        }
    }

    /// Point is on the empty last line of S (`(= (oref s end) (1+ (point)))`).
    pub(super) fn at_section_end(&self, s: usize, p: usize) -> bool {
        let sec = self.sec(s);
        sec.end == p + 1 && p != sec.start && self.buf.all.get(p).is_some_and(|l| l.text.is_empty())
    }

    /// magit-section-forward (n)
    pub(super) fn section_forward(&mut self) {
        let p = self.point();
        let s = self.cur_sec();
        if s == 0 {
            return self.forward_line(1);
        }
        let mut next = if !self.hidden(s) && !self.at_section_end(s, p) {
            self.sec(s).children.first().copied()
        } else {
            None
        };
        let mut cur = Some(s);
        while next.is_none() {
            let Some(c) = cur else { break };
            next = self.sibling(c, true);
            if next.is_none() {
                cur = self.sec(c).parent;
            }
        }
        match next {
            Some(n) => self.goto_sec(n),
            None => self.set_msg("No next section"),
        }
    }

    /// magit-section-backward (p)
    pub(super) fn section_backward(&mut self) {
        let p = self.point();
        if p == 0 {
            return self.set_msg("No previous section");
        }
        let s = self.cur_sec();
        let sec = self.sec(s).clone();
        if self.at_section_end(s, p) && !sec.children.is_empty() {
            let last = *sec.children.last().unwrap_or(&s);
            return self.goto_sec(last);
        }
        if sec.parent.is_some() && p != sec.start {
            return self.goto_sec(s);
        }
        let prev = match self.sibling(s, false) {
            Some(mut prev) => {
                while !self.hidden(prev) && !self.sec(prev).children.is_empty() {
                    prev = *self.sec(prev).children.last().unwrap_or(&prev);
                }
                Some(prev)
            }
            None => sec.parent,
        };
        match prev {
            Some(x) => self.goto_sec(x),
            None => self.forward_line(-1),
        }
    }

    /// magit-section-forward-sibling (M-n)
    pub(super) fn sibling_forward(&mut self) {
        let s = self.cur_sec();
        if s == 0 {
            return self.forward_line(1);
        }
        match self.sibling(s, true) {
            Some(n) => self.goto_sec(n),
            None => self.section_forward(),
        }
    }

    /// magit-section-backward-sibling (M-p)
    pub(super) fn sibling_backward(&mut self) {
        let s = self.cur_sec();
        if s == 0 {
            return self.forward_line(-1);
        }
        match self.sibling(s, false) {
            Some(n) => self.goto_sec(n),
            None => self.section_backward(),
        }
    }

    /// magit-section-up (^)
    pub(super) fn section_up(&mut self) {
        match self.sec(self.cur_sec()).parent {
            Some(par) => self.goto_sec(par),
            None => self.set_msg("No parent section"),
        }
    }

    pub(super) fn show_sec(&mut self, s: usize) {
        self.set_hidden(s, false);
    }

    pub(super) fn hide_sec(&mut self, s: usize) {
        self.set_hidden(s, true);
    }

    pub(super) fn hide_children(&mut self, s: usize) {
        for c in self.children(s) {
            self.hide_sec(c);
        }
    }

    /// magit-section-show-headings
    pub(super) fn show_headings(&mut self, s: usize) {
        self.show_headings_1(s);
        self.show_sec(s);
    }

    pub(super) fn show_headings_1(&mut self, s: usize) {
        for c in self.children(s) {
            self.set_hidden(c, false);
            if !self.sec(c).children.is_empty() || !self.sec(c).content {
                self.show_headings_1(c);
            }
        }
    }

    /// magit-section-show-children
    pub(super) fn show_children(&mut self, s: usize, depth: Option<usize>) {
        self.show_children_1(s, depth);
        self.show_sec(s);
    }

    pub(super) fn show_children_1(&mut self, s: usize, depth: Option<usize>) {
        for c in self.children(s) {
            self.set_hidden(c, false);
            match depth {
                Some(0) => self.hide_sec(c),
                Some(d) => self.show_children_1(c, Some(d - 1)),
                None => self.show_children_1(c, None),
            }
        }
    }

    /// magit-section-hidden-body
    pub(super) fn hidden_body(&self, s: usize) -> bool {
        let ch = &self.sec(s).children;
        if ch.is_empty() {
            self.sec(s).content && self.hidden(s)
        } else {
            ch.iter().any(|&c| self.hidden_body(c))
        }
    }

    /// magit-section-toggle (TAB)
    pub(super) fn toggle_section(&mut self) {
        let p = self.point();
        let s = self.cur_sec();
        if s == 0 {
            return self.set_msg("Cannot hide root section");
        }
        if self.hidden(s) {
            self.show_sec(s);
        } else {
            self.hide_sec(s);
        }
        self.relayout_at(p);
    }

    /// The cycle step shared by magit-section-cycle and -cycle-global for
    /// the children CH of a shown section; returns false when nothing applied.
    pub(super) fn cycle_children(&mut self, s: usize, ch: &[usize]) -> bool {
        if ch.iter().any(|&c| self.hidden(c)) && ch.iter().any(|&c| !self.sec(c).children.is_empty()) {
            self.show_headings(s);
        } else if ch.iter().any(|&c| self.hidden_body(c)) {
            self.show_children(s, None);
        } else {
            return false;
        }
        true
    }

    /// magit-section-cycle (C-TAB)
    pub(super) fn cycle_section(&mut self) {
        let p = self.point();
        let s = self.cur_sec();
        if s == 0 {
            return self.cycle_global();
        }
        if self.hidden(s) {
            self.show_sec(s);
            self.hide_children(s);
        } else {
            let ch = self.children(s);
            if !self.cycle_children(s, &ch) {
                self.hide_sec(s);
            }
        }
        self.relayout_at(p);
    }

    /// magit-section-cycle-global (S-TAB)
    pub(super) fn cycle_global(&mut self) {
        let p = self.point();
        let ch = self.children(0);
        if !self.cycle_children(0, &ch) {
            for c in ch {
                self.hide_sec(c);
            }
        }
        self.relayout_at(p);
    }

    /// magit-section-cycle-diffs (M-TAB): the file sections of the staged
    /// and unstaged changes.
    pub(super) fn cycle_diffs(&mut self) {
        let p = self.point();
        let mut files = Vec::new();
        if self.mode == BufferMode::Status {
            for id in ["staged", "unstaged"] {
                if let Some(s) = (1..self.buf.sections.len()).find(|&s| self.sec(s).ident == id) {
                    if self.hidden(s) {
                        self.show_sec(s);
                    }
                    files.extend(self.children(s));
                }
            }
        } else {
            files = self
                .children(0)
                .into_iter()
                .filter(|&s| matches!(self.buf.all[self.sec(s).start].kind, LineKind::File { .. }))
                .collect();
        }
        if files.is_empty() {
            return self.relayout_at(p);
        }
        if files.iter().any(|&f| self.hidden(f)) {
            for &f in &files {
                self.show_sec(f);
                self.hide_children(f);
            }
        } else {
            let hunks: Vec<usize> = files.iter().flat_map(|&f| self.children(f)).collect();
            if hunks.iter().any(|&h| self.hidden(h)) && hunks.iter().any(|&h| !self.sec(h).children.is_empty()) {
                for &f in &files {
                    self.show_headings(f);
                }
            } else if hunks.iter().any(|&h| self.hidden_body(h)) {
                for &f in &files {
                    self.show_children(f, None);
                }
            } else {
                for &f in &files {
                    self.hide_sec(f);
                }
            }
        }
        self.relayout_at(p);
    }

    /// magit-section-show-level: LEVEL > 0 for the sections around point
    /// (1..4), LEVEL < 0 for all sections (M-1..M-4).
    pub(super) fn show_level(&mut self, level: i32) {
        if level < 0 {
            let lvl = level.unsigned_abs() as usize;
            let mut s = self.cur_sec();
            let mut p = self.point();
            while self.sec(s).level > lvl {
                s = self.sec(s).parent.unwrap_or(0);
                p = self.sec(s).start;
            }
            self.show_children(0, Some(lvl.saturating_sub(1)));
            return self.relayout_at(p);
        }
        let lvl = level as usize;
        let sections = self.region_sections(false).unwrap_or_else(|| vec![self.cur_sec()]);
        let mut p = self.point();
        for section in sections {
            let mut s = section;
            let mut i = self.sec(s).level;
            loop {
                if i < lvl {
                    self.show_children(s, Some(lvl - i - 1));
                    break;
                }
                if i == lvl {
                    self.hide_sec(s);
                    break;
                }
                s = self.sec(s).parent.unwrap_or(0);
                i -= 1;
            }
            p = self.sec(s).start;
        }
        self.relayout_at(p);
    }

    /// magit-region-sections: sibling sections selected by a region that
    /// begins and ends in their headings. With MULTIPLE, a single section
    /// does not count.
    pub(super) fn region_sections(&self, multiple: bool) -> Option<Vec<usize>> {
        let m = self.mark?;
        let p = self.point();
        let (rb, re) = (m.min(p), m.max(p));
        let sb = self.buf.section_at(rb);
        let se = self.buf.section_at(re);
        if se == 0 || (sb == se && (multiple || re > rb)) {
            return None;
        }
        if self.sec(sb).start != rb || self.sec(se).start != re {
            return None;
        }
        let ch = &self.sec(self.sec(sb).parent?).children;
        let i = ch.iter().position(|&c| c == sb)?;
        let j = ch.iter().position(|&c| c == se)?;
        (j >= i).then(|| ch[i..=j].to_vec())
    }

}
