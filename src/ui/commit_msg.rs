//! ui/commit_msg.rs — the built-in COMMIT_EDITMSG buffer (an xagit
//! addition, replacing the external editor for the `c` menu): a small
//! multi-line text editor with a point (row, column), C-n/C-p movement and
//! C-c C-c to commit.

/// The state before an edit (undo).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    lines: Vec<String>,
    row: usize,
    col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    /// ordinary character input: a run of it is one undo unit
    Typing,
    Other,
}

/// How many undo steps are kept.
const UNDO_LIMIT: usize = 200;

/// The message being edited. `col` is a character index into `lines[row]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
    /// comment lines (starting with the comment char) and everything below
    /// the first "scissors" line are stripped by git before committing
    pub comment_char: char,
    undo: Vec<Snapshot>,
    last_edit: Option<EditKind>,
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
        Message { lines, row, col, comment_char, undo: Vec::new(), last_edit: None }
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

    /// Remember the state before an edit.  A run of typed characters is
    /// one undo unit; any other edit (or a cursor move) starts a new one.
    fn save(&mut self, kind: EditKind) {
        if kind == EditKind::Typing && self.last_edit == Some(EditKind::Typing) {
            return;
        }
        if self.undo.len() >= UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(Snapshot { lines: self.lines.clone(), row: self.row, col: self.col });
        self.last_edit = Some(kind);
    }

    /// C-/ (C-_): restore the state before the last edit, including point.
    /// Returns false when there is nothing left to undo.
    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        self.lines = s.lines;
        self.row = s.row;
        self.col = s.col;
        self.last_edit = None;
        true
    }

    /// The point shown in the mode line: line:column/total lines.
    pub fn position(&self) -> String {
        format!("{}:{}/{}", self.row + 1, self.col + 1, self.lines.len())
    }

    /// (first line, last line, last line holding text) of every run of
    /// non-comment lines.  Comment lines separate the blocks; a run of
    /// only blank lines is not a block.
    fn blocks(&self) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        let mut start: Option<usize> = None;
        let mut end = 0;
        let mut last: Option<usize> = None;
        for (i, line) in self.lines.iter().enumerate() {
            if line.starts_with(self.comment_char) {
                if let (Some(a), Some(b)) = (start.take(), last.take()) {
                    out.push((a, end, b));
                }
                continue;
            }
            if start.is_none() {
                start = Some(i);
            }
            end = i;
            if !line.trim().is_empty() {
                last = Some(i);
            }
        }
        if let (Some(a), Some(b)) = (start, last) {
            out.push((a, end, b));
        }
        out
    }

    /// (first, last) line of the non-comment block at point, so the whole
    /// block can be shaded; None on a comment line (or an empty block).
    pub fn current_block(&self) -> Option<(usize, usize)> {
        self.blocks()
            .iter()
            .copied()
            .find(|(a, b, _)| self.row >= *a && self.row <= *b)
            .map(|(a, b, _)| (a, b))
    }

    /// M-p / M-n: move the point to the previous/next block of non-comment
    /// lines, skipping the comment instructions altogether.  It lands at
    /// the end of the block's last text line, where typing continues.  N
    /// is the number of blocks (a prefix argument); negative moves up.
    pub fn move_block(&mut self, n: isize) {
        if n == 0 {
            return;
        }
        self.last_edit = None;
        let blocks = self.blocks();
        let target = match blocks.iter().position(|(a, b, _)| self.row >= *a && self.row <= *b) {
            Some(k) => {
                let j = k as isize + n;
                (j >= 0 && j < blocks.len() as isize).then(|| blocks[j as usize])
            }
            // on a comment line: the nearest block in the direction of N
            None if n > 0 => blocks.iter().copied().find(|(a, _, _)| *a > self.row),
            None => blocks.iter().rev().copied().find(|(_, b, _)| *b < self.row),
        };
        if let Some((_, _, last)) = target {
            self.row = last;
            self.col = self.lines[last].chars().count();
            self.clamp();
        }
    }

    /// M-k: kill the whole non-comment block at point, whatever the column
    /// or which line of the block point is on.  On a comment line the
    /// nearest message block is killed (below first, else above); comment
    /// lines themselves are never killed.  An empty editable line is left
    /// where the block was.  Returns false when there is no block to kill.
    pub fn kill_block(&mut self) -> bool {
        let blocks = self.blocks();
        let Some((a, b, _)) = blocks
            .iter()
            .copied()
            .find(|(a, b, _)| self.row >= *a && self.row <= *b)
            .or_else(|| blocks.iter().copied().find(|(a, _, _)| *a > self.row))
            .or_else(|| blocks.iter().rev().copied().find(|(_, b, _)| *b < self.row))
        else {
            return false;
        };
        self.save(EditKind::Other);
        self.lines.drain(a..=b);
        // leave a line to type on where the message was, before the comments
        let at = a.min(self.lines.len());
        if self.lines.get(at).map_or(true, |l| l.starts_with(self.comment_char)) {
            self.lines.insert(at, String::new());
        }
        self.row = at;
        self.col = 0;
        self.clamp();
        true
    }

    /// C-n / C-p (and up/down): move the point one row, keeping the column
    /// as close as possible.
    pub fn move_line(&mut self, d: isize) {
        self.last_edit = None;
        let n = self.lines.len() as isize;
        self.row = (self.row as isize + d).clamp(0, n - 1).max(0) as usize;
        self.clamp();
    }

    /// C-b / C-f (and left / right): move one character.
    pub fn move_col(&mut self, d: isize) {
        self.last_edit = None;
        let n = self.lines[self.row].chars().count() as isize;
        self.col = (self.col as isize + d).clamp(0, n).max(0) as usize;
    }

    /// C-a: beginning of the line.
    pub fn home(&mut self) {
        self.last_edit = None;
        self.col = 0;
    }

    /// C-e: end of the line.
    pub fn end(&mut self) {
        self.last_edit = None;
        self.col = self.lines[self.row].chars().count();
    }

    pub fn insert(&mut self, c: char) {
        self.save(EditKind::Typing);
        let line = &mut self.lines[self.row];
        let at = line.char_indices().nth(self.col).map(|(i, _)| i).unwrap_or(line.len());
        line.insert(at, c);
        self.col += 1;
    }

    /// RET: split the line at the point.
    pub fn newline(&mut self) {
        self.save(EditKind::Other);
        let line = &mut self.lines[self.row];
        let at = line.char_indices().nth(self.col).map(|(i, _)| i).unwrap_or(line.len());
        let rest = line.split_off(at);
        self.lines.insert(self.row + 1, rest);
        self.row += 1;
        self.col = 0;
    }

    /// DEL/backspace (or C-d at the point). Returns false at the very start.
    pub fn delete_backward(&mut self) -> bool {
        if self.col == 0 && self.row == 0 {
            return false;
        }
        self.save(EditKind::Other);
        if self.col > 0 {
            let line = &mut self.lines[self.row];
            let at = line.char_indices().nth(self.col - 1).map(|(i, _)| i).unwrap_or(line.len());
            line.remove(at);
            self.col -= 1;
        } else {
            let cur = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
            self.lines[self.row].push_str(&cur);
        }
        true
    }

    /// C-d: delete the character under the point.
    pub fn delete_forward(&mut self) {
        let n = self.lines[self.row].chars().count();
        if self.col >= n && self.row + 1 >= self.lines.len() {
            return;
        }
        self.save(EditKind::Other);
        if self.col < n {
            let line = &mut self.lines[self.row];
            let at = line.char_indices().nth(self.col).map(|(i, _)| i).unwrap_or(line.len());
            line.remove(at);
        } else {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    /// C-k: kill the whole line at point, whatever the column.  A single
    /// line is cleared instead of removed: there is always a line to edit.
    pub fn kill_line(&mut self) {
        self.save(EditKind::Other);
        if self.lines.len() > 1 {
            self.lines.remove(self.row);
        } else {
            self.lines[0].clear();
        }
        self.col = 0;
        self.clamp();
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

    const BLOCKS: &str = "subject\n\nbody\n# Please enter the commit message\n# more instructions\n\nsecond block\n# tail\n";

    #[test]
    fn move_block_skips_comment_blocks() {
        let mut m = Message::new(BLOCKS, '#');
        assert_eq!((m.row, m.col), (2, 4), "point starts in the message block");
        assert_eq!(m.position(), "3:5/8");
        m.move_block(1);
        assert_eq!((m.row, m.col), (6, 12), "the comment block is skipped");
        assert_eq!(m.position(), "7:13/8");
        m.move_block(-1);
        assert_eq!(m.row, 2);
        m.move_block(-1);
        assert_eq!(m.row, 2, "no block before the first");
        m.move_block(1);
        assert_eq!(m.row, 6);
        m.move_block(1);
        assert_eq!(m.row, 6, "no block after the last");
        // from a comment line, the nearest block in the direction taken
        m.move_line(-3); // on the first instruction line
        assert_eq!(m.row, 3);
        m.move_block(1);
        assert_eq!(m.row, 6);
        m.move_line(-2);
        m.move_block(-1);
        assert_eq!(m.row, 2);
    }

    #[test]
    fn move_block_with_a_prefix_argument() {
        let mut m = Message::new("one\n# c\ntwo\n# c\nthree\n", '#');
        assert_eq!(m.row, 0);
        m.move_block(2);
        assert_eq!(m.row, 4, "two blocks down, comments skipped");
        m.move_block(-2);
        assert_eq!(m.row, 0);
    }

    #[test]
    fn move_block_on_plain_text() {
        // '\0' is used by the text buffers whose lines are never comments
        let mut m = Message::new("one\ntwo\n", '\0');
        assert_eq!(m.position(), "2:4/2");
        m.move_block(1);
        assert_eq!(m.row, 1, "the whole buffer is one block");
        m.move_block(-1);
        assert_eq!(m.row, 1);
    }

    #[test]
    fn position_follows_the_point() {
        let mut m = Message::new("", '#');
        assert_eq!(m.position(), "1:1/1");
        m.insert('a');
        assert_eq!(m.position(), "1:2/1");
        m.newline();
        assert_eq!(m.position(), "2:1/2");
    }

    #[test]
    fn kill_line_ignores_the_column() {
        let mut m = Message::new("abc\ndef\n# comment\n", '#');
        m.row = 0;
        m.col = 1; // in the middle of "abc"
        m.kill_line();
        assert_eq!(m.lines, ["def", "# comment"]);
        assert_eq!((m.row, m.col), (0, 0));
        m.kill_line();
        assert_eq!(m.lines, ["# comment"]);
        m.kill_line();
        assert_eq!(m.lines, [""], "a single line is cleared, not removed");
        assert_eq!(m.position(), "1:1/1");
    }

    #[test]
    fn kill_block_ignores_the_column() {
        let mut m = Message::new(BLOCKS, '#');
        m.row = 0;
        m.col = 3; // in the middle of "subject"
        assert!(m.kill_block());
        assert_eq!((m.row, m.col), (0, 0), "point on the empty line left behind");
        let mut n = Message::new(BLOCKS, '#');
        n.row = 2;
        n.col = 4; // at the end of "body"
        assert!(n.kill_block());
        assert_eq!(n.lines, m.lines, "the block is killed wherever point is");
        assert!(!m.lines.iter().any(|l| l.contains("subject") || l.contains("body")), "{:?}", m.lines);
        assert!(m.lines.iter().any(|l| l.starts_with('#')), "comments stay: {:?}", m.lines);
        // from a comment line, the nearest message block is killed
        m.row = 2;
        m.col = 7;
        assert!(m.kill_block());
        assert!(!m.lines.iter().any(|l| l.contains("second block")), "{:?}", m.lines);
        // nothing but comments: no block to kill
        let mut only = Message::new("# only\n# comments\n", '#');
        assert!(!only.kill_block());
        assert_eq!(only.lines, ["# only", "# comments"]);
    }

    #[test]
    fn undo_restores_edits_and_the_point() {
        let mut m = Message::new("subject\n\nbody\n# comment\n", '#');
        let before = m.text();
        let point = (m.row, m.col);
        for c in "abc".chars() {
            m.insert(c);
        }
        assert_eq!(m.text(), "subject\n\nbodyabc\n# comment\n");
        assert!(m.undo(), "a run of typing is one undo unit");
        assert_eq!(m.text(), before);
        assert_eq!((m.row, m.col), point);
        // kills are undone in reverse order
        m.row = 0;
        m.col = 1;
        m.kill_line();
        m.kill_block();
        assert_eq!(m.lines, ["", "# comment"]);
        assert!(m.undo());
        assert_eq!(m.lines, ["", "body", "# comment"]);
        assert!(m.undo());
        assert_eq!(m.text(), before);
        assert_eq!((m.row, m.col), (0, 1));
        assert!(!m.undo());
    }

    #[test]
    fn undo_covers_newline_and_deletes() {
        let mut m = Message::new("ab\n", '#');
        m.newline();
        assert_eq!(m.text(), "ab\n\n");
        assert!(m.undo());
        assert_eq!(m.text(), "ab\n");
        m.delete_backward();
        assert_eq!(m.text(), "a\n");
        assert!(m.undo());
        assert_eq!(m.text(), "ab\n");
        m.home();
        m.delete_forward();
        assert_eq!(m.text(), "b\n");
        assert!(m.undo());
        assert_eq!(m.text(), "ab\n");
    }

    #[test]
    fn cursor_movement_splits_typing_into_undo_units() {
        let mut m = Message::new("", '#');
        m.insert('a');
        m.move_col(-1);
        m.insert('b');
        assert_eq!(m.text(), "ba\n");
        assert!(m.undo());
        assert_eq!(m.text(), "a\n");
        assert!(m.undo());
        assert_eq!(m.text(), "\n");
        assert!(!m.undo());
    }

    #[test]
    fn current_block_covers_every_line_of_a_run() {
        let mut m = Message::new(BLOCKS, '#');
        // point starts on the last text line of the first block
        assert_eq!(m.current_block(), Some((0, 2)));
        m.row = 0;
        assert_eq!(m.current_block(), Some((0, 2)), "the first line too");
        m.row = 1;
        assert_eq!(m.current_block(), Some((0, 2)), "the blank line inside the block too");
        m.move_block(1);
        assert_eq!(m.current_block(), Some((5, 6)), "the second block");
        m.row = 3;
        assert_eq!(m.current_block(), None, "a comment line is not in a block");
    }
}
