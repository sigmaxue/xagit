//! ui/rebase_todo.rs — the git-rebase-todo list edited in the TUI, like
//! Magit's `git-rebase-mode` (git-rebase.el): change the action of the
//! commit lines, (un)comment and move lines, insert exec / break / noop /
//! pick lines, undo.  This is the pure model; app.rs shows it and runs git.

/// The kinds of rebase lines (git-rebase-line-regexps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineType {
    /// `<action> <commit> [[# ] <oneline>]`, fixup with `-C` / `-c`
    Commit,
    /// `exec <command>`
    Exec,
    /// `break` / `noop`
    Bare,
    /// `label` / `reset` / `update-ref` / `merge`
    Label,
}

/// A parsed line (git-rebase-current-line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub typ: Option<LineType>,
    /// the full action name ("pick", "fixup", ...)
    pub action: String,
    /// "-C" / "-c" of fixup
    pub options: Option<String>,
    pub target: String,
    /// whether the trailer was written after "# " (git >= 2.50)
    pub hash_sep: bool,
    pub trailer: String,
    pub comment: bool,
}

/// git-rebase-short-options and the long names.
fn full_action(a: &str) -> Option<&'static str> {
    Some(match a {
        "p" | "pick" => "pick",
        "r" | "reword" => "reword",
        "e" | "edit" => "edit",
        "s" | "squash" => "squash",
        "f" | "fixup" => "fixup",
        "d" | "drop" => "drop",
        "x" | "exec" => "exec",
        "b" | "break" => "break",
        "noop" => "noop",
        "l" | "label" => "label",
        "t" | "reset" => "reset",
        "u" | "update-ref" => "update-ref",
        "m" | "merge" => "merge",
        _ => return None,
    })
}

/// Parse LINE; CC is core.commentChar.
pub fn parse(line: &str, cc: &str) -> Parsed {
    let none = Parsed {
        typ: None,
        action: String::new(),
        options: None,
        target: String::new(),
        hash_sep: false,
        trailer: String::new(),
        comment: false,
    };
    let (comment, rest) = match line.strip_prefix(cc) {
        Some(r) if !cc.is_empty() => (true, r.trim_start_matches(' ')),
        _ => (false, line.trim_start_matches(' ')),
    };
    let (word, after) = rest.split_once(' ').unwrap_or((rest, ""));
    let Some(action) = full_action(word) else { return none };
    let p = |typ, target: &str, sep: bool, trailer: &str, options: Option<String>| Parsed {
        typ: Some(typ),
        action: action.to_string(),
        options,
        target: target.to_string(),
        hash_sep: sep,
        trailer: trailer.to_string(),
        comment,
    };
    let split_trailer = |s: &str| -> (bool, String) {
        match s.strip_prefix("# ") {
            Some(t) => (true, t.to_string()),
            None => (false, s.to_string()),
        }
    };
    match action {
        "pick" | "reword" | "edit" | "squash" | "fixup" | "drop" => {
            let mut after = after;
            let mut options = None;
            if action == "fixup" {
                if let Some(r) = after.strip_prefix("-C ").or_else(|| after.strip_prefix("-c ")) {
                    options = Some(after[..2].to_string());
                    after = r;
                }
            }
            let (target, trailer) = after.split_once(' ').unwrap_or((after, ""));
            if target.is_empty() {
                return none;
            }
            let (sep, trailer) = split_trailer(trailer);
            p(LineType::Commit, target, sep, &trailer, options)
        }
        "exec" if !after.trim().is_empty() => p(LineType::Exec, after, false, "", None),
        "break" | "noop" if after.trim().is_empty() => p(LineType::Bare, "", false, "", None),
        "label" | "reset" | "update-ref" | "merge" if !after.trim().is_empty() => {
            p(LineType::Label, after, false, "", None)
        }
        _ => none,
    }
}

/// The todo list being edited.
#[derive(Debug, Clone)]
pub struct Todo {
    pub lines: Vec<String>,
    /// core.commentChar
    pub cc: String,
    /// buffer-modified-p
    pub modified: bool,
    undo: Vec<(Vec<String>, usize)>,
}

impl Todo {
    pub fn new(text: &str, cc: &str) -> Todo {
        let cc = if cc.is_empty() { "#" } else { cc };
        Todo {
            lines: text.lines().map(str::to_string).collect(),
            cc: cc.to_string(),
            modified: false,
            undo: Vec::new(),
        }
    }

    /// The text written back for git (newline-terminated).
    pub fn text(&self) -> String {
        let mut s = self.lines.join("\n");
        s.push('\n');
        s
    }

    pub fn parsed(&self, i: usize) -> Parsed {
        parse(self.lines.get(i).map_or("", String::as_str), &self.cc)
    }

    /// git-rebase-line-p
    pub fn is_rebase_line(&self, i: usize) -> bool {
        self.parsed(i).typ.is_some()
    }

    /// Whether git would do anything: an uncommented action line.
    pub fn has_actions(&self) -> bool {
        (0..self.lines.len()).any(|i| {
            let p = self.parsed(i);
            p.typ.is_some() && !p.comment
        })
    }

    fn save(&mut self, cursor: usize) {
        self.undo.push((self.lines.clone(), cursor));
        self.modified = true;
    }

    /// git-rebase-undo: the cursor before the change undone.
    pub fn undo(&mut self) -> Option<usize> {
        let (lines, cursor) = self.undo.pop()?;
        self.lines = lines;
        self.modified = !self.undo.is_empty();
        Some(cursor)
    }

    /// git-rebase-region-bounds: the lines of the region if both ends are
    /// rebase lines, else (FALLBACK) the current line if it is one.
    pub fn bounds(&self, cursor: usize, mark: Option<usize>, fallback: bool) -> Option<(usize, usize)> {
        match mark {
            Some(m) => {
                let (a, b) = (m.min(cursor), m.max(cursor));
                (self.is_rebase_line(a) && self.is_rebase_line(b)).then_some((a, b))
            }
            None if fallback && self.is_rebase_line(cursor) => Some((cursor, cursor)),
            None => None,
        }
    }

    /// git-rebase-set-action: ACTION on the commit lines of the region (or
    /// the current line); None (un)comments the lines of any type.  Returns
    /// the new cursor (git-rebase-auto-advance), None if there's nothing
    /// to act on (ding).
    pub fn set_action(&mut self, cursor: usize, mark: Option<usize>, action: Option<&str>) -> Option<usize> {
        let (a, b) = self.bounds(cursor, mark, true)?;
        self.save(cursor);
        for i in a..=b {
            let p = self.parsed(i);
            match (action, p.typ) {
                (Some(act), Some(LineType::Commit)) => {
                    let mut l = format!("{act} {}", p.target);
                    if !p.trailer.is_empty() {
                        l.push(' ');
                        if p.hash_sep {
                            l.push_str("# ");
                        }
                        l.push_str(&p.trailer);
                    }
                    self.lines[i] = l;
                }
                (None, Some(_)) => {
                    let line = &self.lines[i];
                    self.lines[i] = if p.comment {
                        let rest = &line[self.cc.len()..];
                        rest.strip_prefix(' ').unwrap_or(rest).to_string()
                    } else {
                        format!("{} {line}", self.cc)
                    };
                }
                _ => {}
            }
        }
        Some((b + 1).min(self.lines.len().saturating_sub(1)))
    }

    /// The lines that can be reordered: the rebase lines (and blank lines)
    /// before the instructions.
    fn movable_end(&self) -> usize {
        let mut i = 0;
        while i < self.lines.len() && (self.is_rebase_line(i) || self.lines[i].trim().is_empty()) {
            i += 1;
        }
        i
    }

    /// git-rebase-move-line-down by N (negative: up), the region or the
    /// current line. Returns the new (cursor, mark), None at the edge.
    pub fn move_lines(&mut self, cursor: usize, mark: Option<usize>, n: isize) -> Option<(usize, Option<usize>)> {
        let (a, b) = self.bounds(cursor, mark, false).unwrap_or((cursor, cursor));
        let end = self.movable_end();
        if b >= end || n == 0 {
            return None;
        }
        let (new_a, ok) = if n < 0 {
            let k = n.unsigned_abs();
            (a.wrapping_sub(k), a >= k)
        } else {
            let k = n as usize;
            (a + k, b + k < end)
        };
        if !ok {
            return None;
        }
        self.save(cursor);
        let block: Vec<String> = self.lines.drain(a..=b).collect();
        for (j, l) in block.into_iter().enumerate() {
            self.lines.insert(new_a + j, l);
        }
        let shift = |p: usize| (p as isize - a as isize + new_a as isize) as usize;
        Some((shift(cursor), mark.map(shift)))
    }

    /// git-rebase-set-bare-action ("break" / "noop") at CURSOR.
    pub fn set_bare_action(&mut self, cursor: usize, action: &str, arg: bool) -> Option<usize> {
        let p = self.parsed(cursor);
        let same = p.action == action;
        // nothing to do: the same bare action is already there, uncommented
        if !arg && p.typ.is_some() && same && !p.comment {
            return None;
        }
        self.save(cursor);
        if !arg && same {
            self.lines.remove(cursor);
        }
        self.lines.insert(cursor.min(self.lines.len()), action.to_string());
        Some((cursor + 1).min(self.lines.len().saturating_sub(1)))
    }

    /// The initial value of git-rebase-set-noncommit-action for ACTION: the
    /// target of the current line when it is such an action (and no ARG).
    pub fn noncommit_initial(&self, cursor: usize, action: &str, arg: bool) -> Option<String> {
        let p = self.parsed(cursor);
        (!arg && p.action == action).then_some(p.target)
    }

    /// git-rebase-set-noncommit-action with VALUE read for ACTION; INITIAL
    /// as given by `noncommit_initial`.
    pub fn set_noncommit_action(&mut self, cursor: usize, action: &str, value: &str, initial: Option<&str>) -> Option<usize> {
        match (value.is_empty(), initial) {
            (true, None) => None,
            (true, Some(_)) => {
                self.save(cursor);
                self.lines.remove(cursor);
                Some(cursor.min(self.lines.len().saturating_sub(1)))
            }
            (false, _) => {
                self.save(cursor);
                let at = if initial.is_some() {
                    self.lines.remove(cursor);
                    cursor
                } else {
                    (cursor + 1).min(self.lines.len())
                };
                self.lines.insert(at, format!("{action} {value}"));
                Some((at + 1).min(self.lines.len().saturating_sub(1)))
            }
        }
    }

    /// git-rebase-insert: "pick <hash> <subject>" below CURSOR.
    pub fn insert_pick(&mut self, cursor: usize, oneline: &str) -> usize {
        self.save(cursor);
        let at = (cursor + 1).min(self.lines.len());
        self.lines.insert(at, format!("pick {oneline}"));
        (at + 1).min(self.lines.len().saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODO: &str = "pick aaa one\npick bbb two\npick ccc three\n\n# Rebase x..y onto x (3 commands)\n#\n# p, pick <commit> = use commit\n";

    #[test]
    fn parses_lines() {
        let p = parse("pick abc subject here", "#");
        assert_eq!((p.typ, p.action.as_str(), p.target.as_str(), p.trailer.as_str()), (Some(LineType::Commit), "pick", "abc", "subject here"));
        assert_eq!(parse("r abc s", "#").action, "reword");
        let f = parse("fixup -C abc s", "#");
        assert_eq!((f.options.as_deref(), f.target.as_str()), (Some("-C"), "abc"));
        assert!(parse("# pick abc s", "#").comment);
        assert!(parse("pick abc # s", "#").hash_sep);
        assert_eq!(parse("# p, pick <commit> = use commit", "#").typ, None);
        assert_eq!(parse("# Rebase a..b onto a", "#").typ, None);
        assert_eq!(parse("exec make test", "#").typ, Some(LineType::Exec));
        assert_eq!(parse("break", "#").typ, Some(LineType::Bare));
        assert_eq!(parse("", "#").typ, None);
    }

    #[test]
    fn set_action_advances_and_keeps_trailer() {
        let mut t = Todo::new(TODO, "#");
        assert_eq!(t.set_action(0, None, Some("reword")), Some(1));
        assert_eq!(t.lines[0], "reword aaa one");
        // region: both lines
        assert_eq!(t.set_action(1, Some(2), Some("fixup")), Some(3));
        assert_eq!(&t.lines[1..3], ["fixup bbb two", "fixup ccc three"]);
        // not a rebase line: nothing
        assert_eq!(t.set_action(4, None, Some("drop")), None);
        assert!(t.modified);
    }

    #[test]
    fn kill_toggles_comment() {
        let mut t = Todo::new(TODO, "#");
        t.set_action(1, None, None);
        assert_eq!(t.lines[1], "# pick bbb two");
        t.set_action(1, None, None);
        assert_eq!(t.lines[1], "pick bbb two");
        // an action on a commented commit line uncomments it
        t.set_action(1, None, None);
        t.set_action(1, None, Some("edit"));
        assert_eq!(t.lines[1], "edit bbb two");
    }

    #[test]
    fn move_lines_within_the_list() {
        let mut t = Todo::new(TODO, "#");
        assert_eq!(t.move_lines(0, None, -1), None, "top");
        assert_eq!(t.move_lines(2, None, -1), Some((1, None)));
        assert_eq!(&t.lines[0..3], ["pick aaa one", "pick ccc three", "pick bbb two"]);
        assert_eq!(t.move_lines(2, None, 1), Some((3, None)), "onto the blank line");
        assert_eq!(t.move_lines(5, None, 1), None, "instructions don't move");
        let mut t = Todo::new(TODO, "#");
        assert_eq!(t.move_lines(1, Some(0), 1), Some((2, Some(1))));
        assert_eq!(&t.lines[0..3], ["pick ccc three", "pick aaa one", "pick bbb two"]);
    }

    #[test]
    fn bare_noncommit_insert_and_undo() {
        let mut t = Todo::new(TODO, "#");
        assert_eq!(t.set_bare_action(1, "break", false), Some(2));
        assert_eq!(&t.lines[0..3], ["pick aaa one", "break", "pick bbb two"]);
        assert_eq!(t.set_bare_action(1, "break", false), None, "already a break");
        assert_eq!(t.set_noncommit_action(0, "exec", "make", None), Some(2));
        assert_eq!(t.lines[1], "exec make");
        assert_eq!(t.noncommit_initial(1, "exec", false).as_deref(), Some("make"));
        assert_eq!(t.set_noncommit_action(1, "exec", "", Some("make")), Some(1));
        assert_eq!(t.lines[1], "break");
        assert_eq!(t.insert_pick(0, "ddd four"), 2);
        assert_eq!(t.lines[1], "pick ddd four");
        for _ in 0..4 {
            assert!(t.undo().is_some());
        }
        assert_eq!(t.text(), TODO);
        assert!(!t.modified);
        assert!(t.undo().is_none());
    }

    #[test]
    fn has_actions() {
        let mut t = Todo::new(TODO, "#");
        assert!(t.has_actions());
        t.set_action(0, Some(2), None);
        assert!(!t.has_actions());
    }
}
