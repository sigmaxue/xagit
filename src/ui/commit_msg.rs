//! ui/commit_msg.rs — the built-in COMMIT_EDITMSG buffer (an xagit
//! addition, replacing the external editor for the `c` menu): a small
//! multi-line text editor with a point (row, column), C-n/C-p movement and
//! C-c C-c to commit.

/// The message being edited. `col` is a character index into `lines[row]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
    /// comment lines (starting with the comment char) and everything below
    /// the first "scissors" line are stripped by git before committing
    pub comment_char: char,
}

impl Message {
    /// A message buffer pre-filled with TEXT.
    pub fn new(text: &str, comment_char: char) -> Message {
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        // point at the end of the last non-comment, non-blank body line
        // (or the very first line for an empty message): that is where the
        // user finishes the subject / amends an existing message
        let row = lines
            .iter()
            .enumerate()
            .take_while(|(_, l)| !l.starts_with(comment_char))
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, _)| i)
            .last()
            .unwrap_or(0);
        let col = lines[row].chars().count();
        Message { lines, row, col, comment_char }
    }

    pub fn text(&self) -> String {
        let mut s = self.lines.join("\n");
        s.push('\n');
        s
    }

    /// The message git would commit: comments and the scissors trailer
    /// removed, leading/trailing blank lines trimmed.
    pub fn committed_message(&self) -> String {
        let cc = self.comment_char;
        let mut out: Vec<String> = Vec::new();
        for l in &self.lines {
            if l.starts_with(cc) && l.contains("------------------------ >8 ------------------------") {
                break;
            }
            if l.starts_with(cc) {
                continue;
            }
            out.push(l.trim_end().to_string());
        }
        while out.first().is_some_and(|l| l.trim().is_empty()) {
            out.remove(0);
        }
        while out.last().is_some_and(|l| l.trim().is_empty()) {
            out.pop();
        }
        out.join("\n")
    }

    fn clamp(&mut self) {
        self.row = self.row.min(self.lines.len() - 1);
        self.col = self.col.min(self.lines[self.row].chars().count());
    }

    /// C-n / C-p (and up/down): move the point one row, keeping the column
    /// as close as possible.
    pub fn move_line(&mut self, d: isize) {
        let n = self.lines.len() as isize;
        self.row = (self.row as isize + d).clamp(0, n - 1).max(0) as usize;
        self.clamp();
    }

    /// C-b / C-f (and left / right): move one character.
    pub fn move_col(&mut self, d: isize) {
        let n = self.lines[self.row].chars().count() as isize;
        self.col = (self.col as isize + d).clamp(0, n).max(0) as usize;
    }

    /// C-a: beginning of the line.
    pub fn home(&mut self) {
        self.col = 0;
    }

    /// C-e: end of the line.
    pub fn end(&mut self) {
        self.col = self.lines[self.row].chars().count();
    }

    pub fn insert(&mut self, c: char) {
        let line = &mut self.lines[self.row];
        let at = line.char_indices().nth(self.col).map(|(i, _)| i).unwrap_or(line.len());
        line.insert(at, c);
        self.col += 1;
    }

    /// RET: split the line at the point.
    pub fn newline(&mut self) {
        let line = &mut self.lines[self.row];
        let at = line.char_indices().nth(self.col).map(|(i, _)| i).unwrap_or(line.len());
        let rest = line.split_off(at);
        self.lines.insert(self.row + 1, rest);
        self.row += 1;
        self.col = 0;
    }

    /// DEL/backspace (or C-d at the point). Returns false at the very start.
    pub fn delete_backward(&mut self) -> bool {
        if self.col > 0 {
            let line = &mut self.lines[self.row];
            let at = line.char_indices().nth(self.col - 1).map(|(i, _)| i).unwrap_or(line.len());
            line.remove(at);
            self.col -= 1;
            true
        } else if self.row > 0 {
            let cur = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
            self.lines[self.row].push_str(&cur);
            true
        } else {
            false
        }
    }

    /// C-d: delete the character under the point.
    pub fn delete_forward(&mut self) {
        let line = &mut self.lines[self.row];
        let n = line.chars().count();
        if self.col < n {
            let at = line.char_indices().nth(self.col).map(|(i, _)| i).unwrap_or(line.len());
            line.remove(at);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    /// C-k: kill to the end of the line (and join, if already at the end).
    pub fn kill_line(&mut self) {
        let line = &mut self.lines[self.row];
        let at = line.char_indices().nth(self.col).map(|(i, _)| i).unwrap_or(line.len());
        line.truncate(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing() {
        let mut m = Message::new("", '#');
        assert_eq!(m.committed_message(), "");
        for c in "hello".chars() {
            m.insert(c);
        }
        m.newline();
        for c in "world".chars() {
            m.insert(c);
        }
        assert_eq!(m.text(), "hello\nworld\n");
        assert_eq!((m.row, m.col), (1, 5));
        m.move_line(-1);
        m.move_col(-3); // "he|llo"
        m.insert('X');
        assert_eq!(m.lines[0], "heXllo");
        m.delete_backward();
        assert_eq!(m.lines[0], "hello");
        m.move_col(100);
        m.delete_forward(); // joins the next line
        assert_eq!(m.text(), "helloworld\n");
        m.move_line(1); // stays at 0
        assert_eq!(m.row, 0);
    }

    #[test]
    fn comments_are_stripped() {
        let text = "\nsubject\n\nbody\n# Please enter the commit message\n# ------------------------ >8 ------------------------\n# diff\n";
        let m = Message::new(text, '#');
        assert_eq!(m.committed_message(), "subject\n\nbody");
        assert_eq!(m.row, 3, "point at the end of the body");
    }

    #[test]
    fn point_starts_after_a_template() {
        let m = Message::new("fix: \n# comment\n", '#');
        // point stays on the subject line, at its end
        assert_eq!(m.row, 0);
        assert_eq!(m.col, 5);
    }
}
