//! ui/app/render.rs — drawing the frame: header, mode line, hints and the buffer body.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;
use crate::ui::commit_msg::Message;

/// The visible line split around the character at point: (before, at,
/// after).  AT is a space when the point is at the end of the line.
/// None when the point is past the part that fits in ROOM columns.
fn point_parts(line: &str, col: usize, room: usize) -> Option<(String, String, String)> {
    let shown = crate::ui::width::truncate(line, room);
    if col > shown.chars().count() {
        return None;
    }
    let mut chars = shown.chars();
    let before: String = chars.by_ref().take(col).collect();
    let at: String = chars.next().map(String::from).unwrap_or_else(|| " ".into());
    let after: String = chars.collect();
    Some((before, at, after))
}

/// Whether line I is shaded: every line of the block at point, plus the
/// point's own line (which may be outside a block, e.g. a comment line).
fn shaded(i: usize, point: usize, block: Option<(usize, usize)>) -> bool {
    i == point || block.is_some_and(|(a, b)| i >= a && i <= b)
}

impl App {

    /// The rows of the transient shown in a window at the bottom (none
    /// while no transient is active).
    pub fn transient_popup(&self) -> Vec<String> {
        let mut popup: Vec<String> = Vec::new();
        if let Some((name, active)) = self.stack.last() {
            if let Some(menu) = self.menus.get(name).filter(|_| self.transient_help.is_none()) {
                popup.push(String::new());
                popup.push(menu.title.clone());
                popup.extend(menu.render(active).lines().map(|l| format!("  {l}")));
                // transient-common-commands are shown after C-x, or
                // always with transient-show-common-commands
                if self.options.show_common_commands || self.pending.first().is_some_and(|k| k.ctrl && k.code == "x") {
                    let row = |(k, d): &(&str, &str)| format!("  {k:<8} {d}");
                    popup.push("  Value commands".into());
                    popup.extend(VALUE_COMMANDS.iter().map(row));
                    popup.push("  Sticky commands".into());
                    popup.extend(STICKY_COMMANDS.iter().map(row));
                    popup.push("  Customize".into());
                    popup.push(row(&("C-x t", self.toggle_common_description())));
                }
                // transient--insert-help
                if self.transient_helpp {
                    popup.push(String::new());
                    popup.push("Type a <KEY> to show help for that suffix command, or ? to show manual.".into());
                    popup.push("Type C-g to exit help.".into());
                }
                popup.push("  q/C-g quit popup".into());
            }
        }
        popup
    }

    /// The transient window, styled: a rule, the title, the columns of
    /// suffixes with their keys and values in faces, then the common
    /// commands and help (the rows of `transient_popup`).
    pub(super) fn popup_rows(&self, w: usize) -> Vec<Row> {
        let text = self.transient_popup();
        if text.is_empty() {
            return Vec::new();
        }
        let t = &self.theme;
        let cells = self
            .stack
            .last()
            .and_then(|(name, active)| self.menus.get(name).map(|m| m.render_cells(active)))
            .unwrap_or_default();
        let key_row = |line: &str, key_len: usize| {
            let mut r = Row::new(Style::default());
            let (k, d) = line.split_at(key_len.min(line.len()));
            let indent = k.len() - k.trim_start().len();
            r.push(&k[..indent], Style::default());
            r.push(k[indent..].to_string(), t.style(Face::PopupKey));
            r.push(d.to_string(), t.style(Face::PopupDesc));
            r
        };
        let mut rows = Vec::with_capacity(text.len());
        for (i, line) in text.iter().enumerate() {
            let row = if i == 0 {
                Row::text(t.glyphs.rule.to_string().repeat(w), t.style(Face::PopupRule))
            } else if i == 1 {
                let mut r = Row::new(Style::default());
                r.push(" ", Style::default()).push(line.clone(), t.style(Face::PopupTitle));
                r
            } else if let Some(cells) = cells.get(i - 2) {
                let mut r = Row::new(Style::default());
                r.push("  ", Style::default());
                for (text, cell) in cells {
                    let face = match cell {
                        crate::ui::transient::Cell::Key => Face::PopupKey,
                        crate::ui::transient::Cell::Value => Face::PopupValue,
                        crate::ui::transient::Cell::Inapt => Face::PopupInapt,
                        crate::ui::transient::Cell::Desc | crate::ui::transient::Cell::Pad => Face::PopupDesc,
                    };
                    r.push(text.clone(), t.style(face));
                }
                r
            } else if matches!(line.trim(), "Value commands" | "Sticky commands" | "Customize") {
                Row::text(line.clone(), t.style(Face::PopupGroup))
            } else if let Some(rest) = line.strip_prefix("  q/C-g") {
                let mut r = key_row("  q/C-g", 7);
                r.push(rest.to_string(), t.style(Face::Dim));
                r
            } else if line.starts_with("  ") && line.len() > 11 && line.as_bytes().get(10) == Some(&b' ') {
                // a common command: "  KEY      DESCRIPTION"
                key_row(line, 10)
            } else {
                Row::text(line.clone(), t.style(Face::Dim))
            };
            rows.push(row);
        }
        rows
    }

    /// The name of the buffer's major mode shown in the mode line.
    pub(super) fn mode_name(&self) -> String {
        if self.view_text.is_some() {
            return "TEXT".into();
        }
        let dbg = format!("{:?}", self.mode);
        dbg.chars().take_while(|c| c.is_alphanumeric()).collect::<String>().to_uppercase()
    }

    /// The header line: the program, the buffer, and on the right the
    /// repository and its HEAD.
    pub(super) fn header_row(&self, w: usize, branch: &str) -> Row {
        let t = &self.theme;
        let bar = t.style(Face::HeaderBar);
        let mut left = Row::new(bar);
        left.push(" xagit ", t.style(Face::Badge)).push(" ", bar);
        let title = match &self.view_text {
            Some((title, _)) => title.clone(),
            None if self.mode == BufferMode::TextEdit => self.text_edit_title().unwrap_or_else(|| "edit".into()),
            None if self.mode == BufferMode::Status || self.buf.title.is_empty() => "Status".into(),
            None => self.buf.title.clone(),
        };
        left.push(crate::git::exec::sanitize(&title), t.style(Face::HeaderTitle));
        let mut right = Row::new(bar);
        let repo = self.git.toplevel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !repo.is_empty() {
            right.push(repo, t.style(Face::ModeLineDim)).push(t.glyphs.sep, t.style(Face::ModeLineDim));
        }
        let face = if branch.starts_with("detached") { Face::Head } else { Face::BranchCurrent };
        right.push(branch.to_string(), t.style(face).over(bar)).push(" ", bar);
        // the title gives way to the branch on narrow terminals
        let right = if right.width() + 8 > w { Row::new(bar) } else { right };
        Row::split(left, right, w)
    }

    /// One line of a built-in message buffer: the whole non-comment block
    /// at point is shaded (not just the point's line), comment lines are
    /// dim, and the point's line carries the cursor gutter with the
    /// character at point highlighted (a block at the end of the line).
    pub(super) fn message_row(&self, m: &Message, i: usize, w: usize, dim_comments: bool, block: Option<(usize, usize)>) -> Row {
        let t = &self.theme;
        let line = &m.lines[i];
        let on_point = i == m.row;
        let base = if shaded(i, m.row, block) { t.style(Face::CursorLine) } else { Style::default() };
        let face = if dim_comments && line.starts_with(m.comment_char) { Face::Dim } else { Face::Default };
        let text = t.style(face).over(base);
        let mut r = Row::new(base);
        let gutter = if on_point { t.glyphs.cursor } else { ' ' };
        r.push(format!("{gutter} "), t.style(Face::Gutter).over(base));
        // one column is left for the point's block at the end of a line
        let room = w.saturating_sub(3);
        let parts = if on_point { point_parts(line, m.col, room) } else { None };
        match parts {
            Some((before, at, after)) => {
                r.push(before, text).push(at, t.style(Face::Point).over(base)).push(after, text);
            }
            None => {
                r.push(crate::ui::width::truncate(line, room).to_string(), text);
            }
        }
        r
    }

    /// The mode line: the mode, the echo area message, the position.
    pub(super) fn mode_line(&self, w: usize, position: String) -> Row {
        let t = &self.theme;
        let bar = t.style(Face::ModeLine);
        let mut left = Row::new(bar);
        left.push(format!(" {} ", self.mode_name()), t.style(Face::ModeBadge)).push(" ", bar);
        if let Some(busy) = self.busy_label() {
            left.push(busy, t.style(Face::Keyword).over(bar)).push("  ", bar);
        }
        let lower = self.message.to_lowercase();
        let error = ["error", "fatal", "cannot", "could not", "unable", "failed", "[no match]"]
            .iter()
            .any(|p| lower.starts_with(p))
            || lower.contains(" failed");
        left.push(self.message.clone(), t.style(if error { Face::Error } else { Face::Message }));
        let mut right = Row::new(bar);
        right.push(position, t.style(Face::ModeLineDim)).push(" ", bar);
        Row::split(left, right, w)
    }

    /// The key hints of the bottom line, as many as fit.
    pub(super) fn hint_row(&self, w: usize, popup: bool) -> Row {
        let t = &self.theme;
        let hints: &[(&str, &str)] = if popup {
            &[("C-x", "common commands"), ("?", "help on a key"), ("C-u", "prefix argument")]
        } else if self.view_text.is_some() {
            &[("SPC/DEL", "page"), ("n/p", "scroll"), ("q", "close")]
        } else if self.mode == BufferMode::TextEdit {
            &[
                ("C-c C-c", "save"),
                ("C-c C-k", "cancel"),
                ("C-n/C-p", "line"),
                ("M-p/M-n", "block"),
                ("?", "help"),
            ]
        } else if self.mode == BufferMode::Commit {
            &[
                ("C-c C-c", "commit"),
                ("C-c C-k", "cancel"),
                ("C-n/C-p", "line"),
                ("M-p/M-n", "block"),
                ("?", "help"),
            ]
        } else if self.mode == BufferMode::Resolve {
            &[
                ("a/b/d", "ours/theirs/both"),
                ("u", "undecide"),
                ("n/p", "conflict"),
                ("SPC/DEL", "page"),
                ("C-c C-c", "save"),
                ("C-c C-k", "cancel"),
                ("?", "help"),
            ]
        } else if self.mode == BufferMode::RebaseTodo {
            &[
                ("C-c C-c", "rebase"),
                ("C-c C-k", "abort"),
                ("c/r/e/s/f/d", "action"),
                ("k", "comment"),
                ("M-p/M-n", "move"),
                ("?", "help"),
            ]
        } else if self.search_query.is_some() {
            &[("C-s", "next"), ("C-r", "prev"), ("C-g", "clear"), ("q", "back"), ("C-x C-c", "quit")]
        } else {
            &[
                ("h/?", "dispatch"),
                ("g", "refresh"),
                ("C-s", "search"),
                ("TAB", "toggle"),
                ("RET", "visit"),
                ("q", "back"),
                ("C-x C-c", "quit"),
                ("n/p", "section"),
                ("C-n/C-p", "line"),
            ]
        };
        let mut r = Row::new(Style::default());
        r.push(" ", Style::default());
        for (k, d) in hints {
            let need = crate::ui::width::str_width(k) + 1 + crate::ui::width::str_width(d) + 3;
            if r.width() + need > w {
                break;
            }
            r.push(*k, t.style(Face::HintKey)).push(" ", Style::default()).push(*d, t.style(Face::HintText));
            r.push("   ", Style::default());
        }
        r
    }

    pub(super) fn draw(&mut self) {
        let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));
        let (w, h) = (w as usize, h as usize);
        let t = self.theme.clone();
        let needle = self
            .search_query
            .as_ref()
            .filter(|q| !q.is_empty())
            .map(|q| folded_bytes(q));
        // a refresh running brings the label along; do not ask git meanwhile
        if self.head_label.is_empty() && !self.jobs.refreshing {
            self.head_label = self.git.head_desc();
        }
        let branch = self.head_label.clone();
        let popup = self.popup_rows(w);
        // header, mode line and hints around the body and the transient
        let body = h.saturating_sub(3 + popup.len()).max(1);
        let mut rows: Vec<Row> = vec![self.header_row(w, &branch)];

        let position;
        if self.mode == BufferMode::TextEdit && self.view_text.is_none() {
            let (r, pos) = self.text_edit_rows(body, w);
            rows.extend(r);
            position = pos;
        } else if self.mode == BufferMode::Commit && self.view_text.is_none() {
            let (r, pos) = self.commit_msg_rows(body, w);
            rows.extend(r);
            position = pos;
        } else if self.mode == BufferMode::Resolve && self.view_text.is_none() {
            let (r, pos) = self.resolve_rows(body, w);
            rows.extend(r);
            position = pos;
        } else if self.mode == BufferMode::RebaseTodo && self.view_text.is_none() {
            let mut scroll = self.scroll;
            let (r, pos) = self.rebase_todo_rows(body, &mut scroll);
            self.scroll = scroll;
            rows.extend(r);
            position = pos;
        } else if let Some((_, content)) = &self.view_text {
            let diff = paint::looks_like_diff(content);
            let mut wrapped: Vec<Row> = Vec::new();
            for (n, line) in content.lines().enumerate() {
                let style = t.style(paint::text_face(line, diff, n == 0));
                for part in paint::wrap(line, w) {
                    let mut r = Row::new(Style::default());
                    r.push(part, style);
                    wrapped.push(r);
                }
            }
            let total = wrapped.len();
            self.view_scroll = self.view_scroll.min(total.saturating_sub(1));
            position = format!("{}/{}", (self.view_scroll + 1).min(total), total);
            rows.extend(wrapped.into_iter().skip(self.view_scroll).take(body));
        } else {
            if self.cursor < self.scroll {
                self.scroll = self.cursor;
            } else if self.cursor >= self.scroll + body {
                self.scroll = self.cursor + 1 - body;
            }
            position = format!("{}/{}", (self.cursor + 1).min(self.lines.len()), self.lines.len());
            let region = self.region_bounds();
            // magit-section-highlight: the section at point
            let current = self.lines.get(self.cursor).map(|l| l.ident.clone()).filter(|s| !s.is_empty());
            let mw = self.buf.margin_width;
            for (i, line) in self.lines.iter().enumerate().skip(self.scroll).take(body) {
                let in_region = region.is_some_and(|(a, b)| i >= a && i <= b);
                let in_section =
                    current.as_ref().is_some_and(|c| line.ident == *c || line.ancestors.iter().any(|a| a == c));
                let base_face = if i == self.cursor {
                    Some(Face::CursorLine)
                } else if in_region {
                    Some(Face::Region)
                } else if in_section {
                    Some(Face::SectionHighlight)
                } else {
                    None
                };
                let base = base_face.map(|f| t.style(f)).unwrap_or_default();
                // the cursor and the region show over the faces' own
                // backgrounds (diff lines), the section highlight under them
                let strong = matches!(base_face, Some(Face::CursorLine | Face::Region));
                let on_base = |s: Style| {
                    let merged = s.over(base);
                    if strong {
                        Style { bg: base.bg.or(s.bg), ..merged }
                    } else {
                        merged
                    }
                };
                let mut r = Row::new(base);
                let gutter = if i == self.cursor {
                    t.glyphs.cursor
                } else if in_region {
                    t.glyphs.region
                } else {
                    ' '
                };
                r.push(gutter.to_string(), on_base(t.style(Face::Gutter)));
                // the nesting of sections shows as indentation: a heading
                // at its level, a body line at the level of its section's
                // heading (the fringe stays next to the heading); the
                // "Head:", "Merge:" … headers stay a table
                let level = if line.kind == LineKind::Head || line.ident.starts_with("headers") {
                    0
                } else if line.heading {
                    line.ancestors.len()
                } else {
                    line.ancestors.len().saturating_sub(1)
                };
                r.push(" ".repeat(2 * level.min(3)), base);
                let fringe = match line.hidden {
                    Some(true) => t.glyphs.folded,
                    Some(false) => t.glyphs.unfolded,
                    None => ' ',
                };
                r.push(fringe.to_string(), on_base(t.style(Face::Fringe)));
                for s in paint::highlight(&t, line) {
                    if let Some(q) = &needle {
                        for (part, matched) in search_parts(&s.text, q) {
                            let style = if matched { t.style(Face::SearchMatch) } else { on_base(s.style) };
                            r.push(part, style);
                        }
                    } else {
                        r.push(s.text, on_base(s.style));
                    }
                }
                if mw > 0 && w > mw + 2 {
                    // the right margin (magit-log-margin)
                    let mut m = Row::new(base);
                    let margin_style = on_base(t.style(Face::Margin));
                    if let Some(q) = &needle {
                        for (part, matched) in search_parts(&line.margin, q) {
                            let style = if matched { t.style(Face::SearchMatch) } else { margin_style };
                            m.push(part, style);
                        }
                    } else {
                        m.push(line.margin.clone(), margin_style);
                    }
                    let left = r.truncated(w - mw);
                    rows.push(Row::split(left, m, w));
                } else {
                    rows.push(r);
                }
            }
        }
        while rows.len() < body + 1 {
            rows.push(Row::default());
        }
        let has_popup = !popup.is_empty();
        rows.extend(popup);
        rows.push(self.mode_line(w, position));
        rows.push(self.hint_row(w, has_popup));
        rows.truncate(h);

        if self.in_terminal {
            // only the rows that changed, unless the terminal may show
            // something else than the last frame
            let full = std::mem::take(&mut self.repaint_all) || self.painted_size != (w, h);
            let screen = self.screen.borrow();
            let previous = (!full).then_some(screen.as_slice());
            let _ = paint::write_rows(&mut stdout(), &t, &rows, previous, w, h);
        }
        self.painted_size = (w, h);
        *self.screen.borrow_mut() = rows;
    }

    /// Repaint rows FROM..TO of the last frame (those the minibuffer's
    /// completions covered).
    pub(super) fn repaint_rows(&self, from: usize, to: usize) {
        let (w, _) = crossterm::terminal::size().unwrap_or((80, 24));
        let screen = self.screen.borrow();
        let mut out = stdout();
        let blank = Row::default();
        for y in from..to {
            let _ = paint::write_row(&mut out, &self.theme, y as u16, screen.get(y).unwrap_or(&blank), w as usize);
        }
        let _ = out.flush();
    }

}

#[cfg(test)]
mod tests {
    use super::{point_parts, shaded};

    #[test]
    fn point_is_highlighted_at_the_input_position() {
        let p = |s: &str, col| point_parts(s, col, 78);
        assert_eq!(p("abc", 0), Some((String::new(), "a".into(), "bc".into())));
        assert_eq!(p("abc", 1), Some(("a".into(), "b".into(), "c".into())));
        assert_eq!(p("abc", 3), Some(("abc".into(), " ".into(), String::new())), "a block at EOL");
        assert_eq!(p("abc", 4), None, "past the visible line");
        // CJK characters take two columns: the point's column stays aligned
        assert_eq!(point_parts("中文abc", 2, 5), Some(("中文".into(), "a".into(), String::new())));
    }

    #[test]
    fn the_whole_block_is_shaded() {
        let block = Some((0, 2)); // subject, blank line, body (point on the last)
        assert!(shaded(0, 2, block), "the first line of the block");
        assert!(shaded(1, 2, block), "the blank line inside the block");
        assert!(shaded(2, 2, block), "the point's line");
        assert!(!shaded(3, 2, block), "a comment line outside the block");
        assert!(shaded(5, 5, None), "the point's line outside any block");
        assert!(!shaded(4, 5, None), "an unrelated line");
    }
}
