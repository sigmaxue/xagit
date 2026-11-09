//! A subset of Emacs regular expressions (string-match-p), for the
//! regexps of options such as magit-refs-filter-alist: literals, `.`,
//! `[...]` / `[^...]`, `^` and `$` (where Emacs treats them as anchors),
//! the postfix `*`, `+` and `?` (and their non-greedy forms),
//! `\(...\)`, `\(?:...\)`, `\|`, `\{N,M\}`, `\`` and `\'`, `\w` / `\W`
//! and `\s-` / `\S-`; any other escaped character is literal.

#[derive(Debug, Clone)]
enum Node {
    Char(char),
    Any,
    Class { negated: bool, items: Vec<(char, char)> },
    Word(bool),
    Space(bool),
    Start,
    End,
    Group(Vec<Vec<Node>>),
    Repeat { node: Box<Node>, min: usize, max: Option<usize> },
}

/// A compiled regexp.
#[derive(Debug, Clone)]
pub struct Regexp {
    alts: Vec<Vec<Node>>,
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self, off: usize) -> Option<char> {
        self.s.get(self.i + off).copied()
    }

    fn at_group_end(&self) -> bool {
        self.peek(0) == Some('\\') && matches!(self.peek(1), Some(')') | Some('|'))
    }

    /// Alternatives up to the end or `\)`.
    fn alts(&mut self) -> Result<Vec<Vec<Node>>, String> {
        let mut alts = vec![self.seq()?];
        while self.peek(0) == Some('\\') && self.peek(1) == Some('|') {
            self.i += 2;
            alts.push(self.seq()?);
        }
        Ok(alts)
    }

    fn seq(&mut self) -> Result<Vec<Node>, String> {
        let mut seq: Vec<Node> = Vec::new();
        while let Some(c) = self.peek(0) {
            if self.at_group_end() {
                break;
            }
            let node = match c {
                '^' if seq.is_empty() => {
                    self.i += 1;
                    Node::Start
                }
                '$' if self.i + 1 == self.s.len() || {
                    self.i += 1;
                    let end = self.at_group_end();
                    self.i -= 1;
                    end
                } =>
                {
                    self.i += 1;
                    Node::End
                }
                '*' | '+' | '?' if !seq.is_empty() => {
                    self.i += 1;
                    let (min, max) = match c {
                        '*' => (0, None),
                        '+' => (1, None),
                        _ => (0, Some(1)),
                    };
                    // the non-greedy forms match the same strings
                    if self.peek(0) == Some('?') {
                        self.i += 1;
                    }
                    let prev = seq.pop().unwrap_or(Node::Char(c));
                    seq.push(Node::Repeat { node: Box::new(prev), min, max });
                    continue;
                }
                '.' => {
                    self.i += 1;
                    Node::Any
                }
                '[' => self.class()?,
                '\\' => match self.escape()? {
                    Some(n) => n,
                    None => {
                        // \{N,M\}
                        let prev = seq.pop().ok_or("Invalid preceding regular expression")?;
                        let (min, max) = self.interval()?;
                        seq.push(Node::Repeat { node: Box::new(prev), min, max });
                        continue;
                    }
                },
                c => {
                    self.i += 1;
                    Node::Char(c)
                }
            };
            seq.push(node);
        }
        Ok(seq)
    }

    /// After `\`: a node, or None for `\{` (the interval follows).
    fn escape(&mut self) -> Result<Option<Node>, String> {
        let c = self.peek(1).ok_or("Trailing backslash")?;
        self.i += 2;
        Ok(Some(match c {
            '(' => {
                if self.peek(0) == Some('?') && self.peek(1) == Some(':') {
                    self.i += 2;
                }
                let alts = self.alts()?;
                if !(self.peek(0) == Some('\\') && self.peek(1) == Some(')')) {
                    return Err("Unmatched ( or \\(".into());
                }
                self.i += 2;
                Node::Group(alts)
            }
            ')' => return Err("Unmatched ) or \\)".into()),
            '{' => return Ok(None),
            '`' => Node::Start,
            '\'' => Node::End,
            'w' => Node::Word(true),
            'W' => Node::Word(false),
            's' | 'S' => {
                let syntax = self.peek(0).ok_or("Invalid syntax designator")?;
                self.i += 1;
                if syntax != '-' && syntax != ' ' {
                    return Err("Invalid syntax designator".into());
                }
                Node::Space(c == 's')
            }
            c => Node::Char(c),
        }))
    }

    /// `N\}`, `N,\}`, `N,M\}` or `,M\}` after `\{`.
    fn interval(&mut self) -> Result<(usize, Option<usize>), String> {
        let num = |p: &mut Self| {
            let start = p.i;
            while p.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                p.i += 1;
            }
            p.s[start..p.i].iter().collect::<String>().parse::<usize>().ok()
        };
        let min = num(self);
        let max = if self.peek(0) == Some(',') {
            self.i += 1;
            num(self)
        } else {
            Some(min.unwrap_or(0))
        };
        if !(self.peek(0) == Some('\\') && self.peek(1) == Some('}')) {
            return Err("Invalid content of \\{\\}".into());
        }
        self.i += 2;
        Ok((min.unwrap_or(0), max))
    }

    /// A bracket expression (`]` first is literal, `[:class:]` ignored
    /// but for the common ones).
    fn class(&mut self) -> Result<Node, String> {
        self.i += 1;
        let negated = self.peek(0) == Some('^');
        if negated {
            self.i += 1;
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek(0).ok_or("Unmatched [ or [^")?;
            if c == ']' && !first {
                self.i += 1;
                break;
            }
            first = false;
            if c == '[' && self.peek(1) == Some(':') {
                let rest: String = self.s[self.i..].iter().collect();
                if let Some(end) = rest.find(":]") {
                    let name = &rest[2..end];
                    items.extend(match name {
                        "digit" => vec![('0', '9')],
                        "alpha" => vec![('a', 'z'), ('A', 'Z')],
                        "alnum" => vec![('a', 'z'), ('A', 'Z'), ('0', '9')],
                        "upper" => vec![('A', 'Z')],
                        "lower" => vec![('a', 'z')],
                        "space" | "blank" => vec![(' ', ' '), ('\t', '\t'), ('\n', '\n')],
                        "xdigit" => vec![('0', '9'), ('a', 'f'), ('A', 'F')],
                        _ => return Err("Invalid character class name".into()),
                    });
                    self.i += rest[..end + 2].chars().count();
                    continue;
                }
            }
            self.i += 1;
            if self.peek(0) == Some('-') && self.peek(1).is_some_and(|e| e != ']') {
                let end = self.peek(1).unwrap_or(c);
                self.i += 2;
                items.push((c, end));
            } else {
                items.push((c, c));
            }
        }
        Ok(Node::Class { negated, items })
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

/// Match SEQ at I in TEXT, then the continuation K.
fn match_seq(seq: &[Node], text: &[char], i: usize, k: &dyn Fn(usize) -> bool) -> bool {
    match seq.split_first() {
        None => k(i),
        Some((n, rest)) => match_node(n, text, i, &|j| match_seq(rest, text, j, k)),
    }
}

fn match_node(n: &Node, text: &[char], i: usize, k: &dyn Fn(usize) -> bool) -> bool {
    let next = text.get(i).copied();
    let one = |ok: bool| ok && k(i + 1);
    match n {
        Node::Char(c) => one(next == Some(*c)),
        Node::Any => one(next.is_some_and(|c| c != '\n')),
        Node::Class { negated, items } => {
            one(next.is_some_and(|c| items.iter().any(|(a, b)| (*a..=*b).contains(&c)) != *negated))
        }
        Node::Word(w) => one(next.is_some_and(|c| is_word(c) == *w)),
        Node::Space(s) => one(next.is_some_and(|c| c.is_whitespace() == *s)),
        Node::Start => i == 0 && k(i),
        Node::End => i == text.len() && k(i),
        Node::Group(alts) => alts.iter().any(|seq| match_seq(seq, text, i, k)),
        Node::Repeat { node, min, max } => match_repeat(node, *min, *max, 0, text, i, k),
    }
}

/// Greedy: one more repetition (that consumes something), else stop.
fn match_repeat(
    node: &Node,
    min: usize,
    max: Option<usize>,
    count: usize,
    text: &[char],
    i: usize,
    k: &dyn Fn(usize) -> bool,
) -> bool {
    if max.is_none_or(|m| count < m)
        && match_node(node, text, i, &|j| (j > i || count < min) && match_repeat(node, min, max, count + 1, text, j, k))
    {
        return true;
    }
    count >= min && k(i)
}

impl Regexp {
    pub fn new(pattern: &str) -> Result<Regexp, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { s: &chars, i: 0 };
        let alts = p.alts()?;
        if p.i < chars.len() {
            return Err("Unmatched ) or \\)".into());
        }
        Ok(Regexp { alts })
    }

    /// string-match-p: whether the regexp matches anywhere in TEXT.
    pub fn is_match(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        let top = [Node::Group(self.alts.clone())];
        (0..=chars.len()).any(|i| match_seq(&top, &chars, i, &|_| true))
    }
}

#[cfg(test)]
mod tests {
    use super::Regexp;

    fn m(re: &str, s: &str) -> bool {
        Regexp::new(re).unwrap().is_match(s)
    }

    #[test]
    fn matching() {
        assert!(m("^refs/pullreqs/", "refs/pullreqs/12"));
        assert!(!m("^pr", "origin/pr-1"));
        assert!(m("pr-[0-9]+$", "origin/pr-12"));
        assert!(!m("pr-[0-9]+$", "origin/pr-12x"));
        assert!(m("\\`origin/\\(main\\|master\\)\\'", "origin/master"));
        assert!(!m("\\`origin/\\(main\\|master\\)\\'", "origin/maint"));
        assert!(m("a.c", "abc") && !m("a\\.c", "abc") && m("a\\.c", "a.c"));
        assert!(m("^[^/]*$", "master") && !m("^[^/]*$", "origin/master"));
        assert!(m("^v[[:digit:]]\\{1,2\\}\\.", "v12.0") && !m("^v[[:digit:]]\\{1,2\\}\\.", "v123.0"));
        assert!(m("x*", "") && m("^a?b", "b") && m("ab*?c", "abbc"));
        assert!(m("a$b", "a$b"), "$ not at the end is literal");
        assert!(m("*a", "*a"), "* at the start is literal");
        assert!(m("^\\w+-\\w+$", "fix-it") && m("a\\s-b", "a b"));
        assert!(Regexp::new("\\(a").is_err() && Regexp::new("[a").is_err() && Regexp::new("a\\)").is_err());
    }
}
