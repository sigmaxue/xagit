//! magit-status-jump (magit-status.el), its section jumpers
//! (magit-define-section-jumper) and imenu (magit--imenu-create-index).

use super::*;

/// The jumpers: (command, heading, section ident).
const JUMPERS: [(&str, &str, &str); 8] = [
    ("jump-to-stashes", "Stashes", "stashes"),
    ("jump-to-untracked", "Untracked files", "untracked"),
    ("jump-to-unstaged", "Unstaged changes", "unstaged"),
    ("jump-to-staged", "Staged changes", "staged"),
    ("jump-to-unpulled-from-upstream", "Unpulled from @{upstream}", "unpulled:..@{upstream}"),
    ("jump-to-unpulled-from-pushremote", "Unpulled from <push-remote>", "unpulled:..@{push}"),
    ("jump-to-unpushed-to-upstream", "Unpushed to @{upstream}", "unpushed:@{upstream}.."),
    ("jump-to-unpushed-to-pushremote", "Unpushed to <push-remote>", "unpushed:@{push}.."),
];

/// magit--imenu-index-name of a heading: a commit's hash and message
/// (the graph and refs in between replaced by a space), else without
/// a trailing " (N)".
fn index_name(heading: &str, commit: bool) -> String {
    if commit {
        if let Some((hash, rest)) = heading.split_once(' ') {
            let rest = rest.trim_start_matches([' ', '*', '|']);
            if !rest.is_empty() {
                return format!("{hash} {rest}");
            }
        }
        return heading.to_string();
    }
    if let Some(i) = heading.rfind(" (") {
        let n = &heading[i + 2..];
        if n.strip_suffix(')').is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())) {
            return heading[..i].to_string();
        }
    }
    heading.to_string()
}

impl App {
    /// The top-level section with IDENT (magit-get-section).
    fn top_section(&self, ident: &str) -> Option<usize> {
        self.children(0).into_iter().find(|&s| self.sec(s).ident == ident)
    }

    /// The section of a jumper with IDENT. "Recent commits" is where
    /// "Unpushed to @{upstream}" would be: magit-insert-unpushed-to-
    /// upstream-or-recent inserts it as an `unpushed` "@{upstream}.."
    /// section, so magit-jump-to-unpushed-to-upstream goes there.
    fn jumper_section(&self, ident: &str) -> Option<usize> {
        self.top_section(ident).or_else(|| match ident {
            "unpushed:@{upstream}.." => self.top_section("recent"),
            _ => None,
        })
    }

    /// The states of the magit-status-jump suffixes: (key, description,
    /// inapt).
    pub(super) fn status_jump_states(&self) -> Vec<(String, String, bool)> {
        let keys = ["z", "n", "u", "s", "fu", "fp", "pu", "pp"];
        keys.iter()
            .zip(JUMPERS)
            .map(|(k, (_, heading, ident))| {
                let desc = if *k == "pu" {
                    // magit-jump-to-unpushed-to-upstream's description
                    let upstream = self.git.upstream_shortname();
                    match upstream {
                        Some(u) if !self.git.git_ok(&["merge-base", "--is-ancestor", "HEAD", &u]) => "Unmerged into upstream",
                        _ => "Recent commits",
                    }
                } else {
                    heading
                };
                (k.to_string(), desc.to_string(), self.jumper_section(ident).is_none())
            })
            .collect()
    }

    /// `j`: magit-status-jump in the status buffer, else
    /// magit-jump-to-diffstat-or-diff.
    pub(super) fn jump_command(&mut self) {
        if self.mode == BufferMode::Status {
            self.open_menu("status-jump-menu");
        } else {
            self.jump_to_diffstat_or_diff();
        }
    }

    /// A section jumper: go to its section, with a prefix argument also
    /// show it (at the top of the window).
    pub(super) fn section_jumper(&mut self, cmd: &str) {
        let Some((_, heading, ident)) = JUMPERS.iter().find(|(c, _, _)| *c == cmd) else {
            return;
        };
        let Some(s) = self.jumper_section(ident) else {
            self.set_msg(&format!("Section \"{heading}\" wasn't found"));
            return;
        };
        let p = self.sec(s).start;
        if self.prefix_arg.is_some() {
            self.show_sec(s);
            self.relayout_at(p);
            self.goto_sec(s);
            self.scroll = self.cursor;
        } else {
            self.goto_sec(s);
        }
    }

    /// imenu with magit--imenu-group-types '(not branch commit): a
    /// top-level section with children, then one of them; their
    /// ancestors shown.
    pub(super) fn imenu(&mut self) -> Result<()> {
        let groups: Vec<(String, usize)> = self
            .children(0)
            .into_iter()
            .filter(|&s| !self.sec(s).children.is_empty())
            .filter(|&s| !matches!(self.buf.all[self.sec(s).start].kind, LineKind::Head | LineKind::Commit { .. }))
            .map(|s| (self.index_name_of(s), s))
            .collect();
        if groups.is_empty() {
            self.set_msg("No items suitable for an index found in this buffer");
            return Ok(());
        }
        let names: Vec<String> = groups.iter().map(|(n, _)| n.clone()).collect();
        let Some(group) = self.imenu_read(names)? else {
            return Ok(());
        };
        let Some(&(_, g)) = groups.iter().find(|(n, _)| *n == group) else {
            return Ok(());
        };
        let items: Vec<(String, usize)> = self.children(g).into_iter().map(|s| (self.index_name_of(s), s)).collect();
        let names: Vec<String> = items.iter().map(|(n, _)| n.clone()).collect();
        let Some(item) = self.imenu_read(names)? else {
            return Ok(());
        };
        let Some(&(_, s)) = items.iter().find(|(n, _)| *n == item) else {
            return Ok(());
        };
        // magit--imenu-goto-function
        let p = self.sec(s).start;
        let mut a = self.sec(s).parent;
        while let Some(x) = a.filter(|&x| x != 0) {
            if self.hidden(x) {
                self.show_sec(x);
            }
            a = self.sec(x).parent;
        }
        self.relayout_at(p);
        self.goto_sec(s);
        Ok(())
    }

    /// The index name of the section S.
    fn index_name_of(&self, s: usize) -> String {
        let line = &self.buf.all[self.sec(s).start];
        index_name(&line.text, matches!(line.kind, LineKind::Commit { .. }))
    }

    /// imenu--completion-buffer: one of NAMES (require-match).
    fn imenu_read(&mut self, names: Vec<String>) -> Result<Option<String>> {
        let table = Completion { candidates: names.clone(), require_match: true, multiple: false };
        let Some(v) = self.completing_read("Index item: ", None, Hist::Minibuffer, Some(table))? else {
            return Ok(None);
        };
        if !names.contains(&v) {
            self.set_msg("[No match]");
            return Ok(None);
        }
        Ok(Some(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_names() {
        assert_eq!(index_name("Unstaged changes (2)", false), "Unstaged changes");
        assert_eq!(index_name("modified   a (b)", false), "modified   a (b)");
        assert_eq!(index_name("abc1234 * | main fix", true), "abc1234 main fix");
    }
}
