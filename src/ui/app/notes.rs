//! The suffixes and readers of magit-notes (magit-notes.el).

use super::*;

/// The name of the notes ref REF read: "refs/notes/" prepended unless
/// it starts with "refs/".
fn qualify(r: &str) -> String {
    if r.starts_with("refs/") {
        r.to_string()
    } else {
        format!("refs/notes/{r}")
    }
}

fn unqualify(r: &str) -> &str {
    r.strip_prefix("refs/notes/").unwrap_or(r)
}

impl App {
    /// magit-list-notes-refnames: the notes refs without "refs/notes/".
    fn notes_refnames(&self) -> Vec<String> {
        self.git
            .git_lines(&["for-each-ref", "--format=%(refname)", "refs/notes"])
            .iter()
            .map(|r| unqualify(r).to_string())
            .collect()
    }

    /// magit-notes-read-ref: one of the notes refs (require-match), the
    /// value of core.notesRef the initial input; qualified.
    pub(super) fn notes_read_ref(&mut self, prompt: &str, hist: &str) -> Result<Option<String>> {
        let candidates = self.notes_refnames();
        *self.initial_input.borrow_mut() = self.config("core.notesRef").map(|d| unqualify(&d).to_string());
        let table = Completion { candidates: candidates.clone(), require_match: true, multiple: false };
        let Some(r) = self.read_line_hist(prompt, None, Hist::Infix(hist), Some(table))? else {
            return Ok(None);
        };
        let r = r.trim().to_string();
        if r.is_empty() {
            self.set_msg("Nothing selected");
            return Ok(None);
        }
        if !candidates.contains(&r) {
            self.set_msg("[No match]");
            return Ok(None);
        }
        add_to_history(self.infix_history.borrow_mut().entry(hist.to_string()).or_default(), &r, true);
        Ok(Some(qualify(&r)))
    }

    /// magit-notes-read-refs: notes refs separated by commas ("PROMPT: ",
    /// the values of notes.displayRef the initial input); qualified.
    pub(super) fn notes_read_refs(&mut self, prompt: &str, hist: &str) -> Result<Option<Vec<String>>> {
        let current: Vec<String> = self
            .git
            .git_lines(&["config", "--get-all", "notes.displayRef"])
            .iter()
            .map(|r| unqualify(r).to_string())
            .collect();
        *self.initial_input.borrow_mut() = Some(current.join(","));
        let table = Completion { candidates: self.notes_refnames(), require_match: false, multiple: true };
        let Some(input) = self.read_line_hist(&format!("{prompt}: "), None, Hist::Infix(hist), Some(table))? else {
            return Ok(None);
        };
        Ok(Some(input.split(',').map(str::trim).filter(|r| !r.is_empty()).map(qualify).collect()))
    }

    /// magit-notes-merging-p: NOTES_MERGE_WORKTREE has files.
    pub(super) fn notes_merging(&self) -> bool {
        std::fs::read_dir(self.git.gitdir.join("NOTES_MERGE_WORKTREE"))
            .map(|d| d.filter_map(|e| e.ok()).any(|e| !e.file_name().to_string_lossy().starts_with('.')))
            .unwrap_or(false)
    }

    /// magit-notes-read-args: magit-read-branch-or-commit (the stash at
    /// point the secondary default) and the value of --ref=.
    fn notes_read_args(&mut self, prompt: &str, args: &[String]) -> Result<Option<(String, Option<String>)>> {
        let stash = match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Stash { name, .. }) => Some(name.clone()),
            _ => None,
        };
        let default = self
            .branch_at_point()
            .or_else(|| self.commit_at_point().map(|c| self.git.rev_short(&c)))
            .or(stash)
            .or_else(|| self.git.current_branch());
        let Some(commit) = self.read_rev(prompt, default.as_deref())? else {
            return Ok(None);
        };
        let r = args.iter().find_map(|a| a.strip_prefix("--ref=")).filter(|r| !r.is_empty()).map(str::to_string);
        Ok(Some((commit, r)))
    }

    /// magit-run-git-with-editor: `git notes ARGS`.
    fn run_notes(&mut self, args: &[&str]) {
        let mut a = vec!["notes"];
        a.extend(args);
        let res = self.with_terminal(|g| g.git_with_editors(Editor::Accept, Editor::User, &a));
        if let Err(e) = res {
            self.report_error(&e);
        }
        self.refresh();
    }

    /// The suffixes of the magit-notes transient.
    pub(super) fn notes_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        match cmd {
            "notes-edit" => {
                let Some((commit, r)) = self.notes_read_args("Edit notes", args)? else {
                    return Ok(());
                };
                // read the current note and edit it in xagit's text buffer;
                // C-c C-c writes it back with `git notes add -f -F`
                let mut show = vec!["notes".to_string()];
                if let Some(r) = &r {
                    show.push(format!("--ref={r}"));
                }
                show.extend(["show".to_string(), commit.clone()]);
                let strs: Vec<&str> = show.iter().map(String::as_str).collect();
                let text = self.git.git_string(&strs).unwrap_or_default();
                self.text_edit_open(&text, "NOTES_EDITMSG", text_edit::TextEditSave::Notes { r, rev: commit });
            }
            "notes-remove" => {
                let Some((commit, r)) = self.notes_read_args("Remove notes", args)? else {
                    return Ok(());
                };
                let r = r.map(|r| format!("--ref={r}"));
                let mut a: Vec<&str> = r.iter().map(String::as_str).collect();
                a.extend(["remove", commit.as_str()]);
                self.run_notes(&a);
            }
            "notes-merge" => {
                let Some(r) = self.read_string_ns("Merge reference")? else {
                    return Ok(());
                };
                self.run_notes(&["merge", &r]);
            }
            "notes-merge-commit" => self.run_notes(&["merge", "--commit"]),
            "notes-merge-abort" => self.run_notes(&["merge", "--abort"]),
            _ => {
                // magit-notes-prune: with --dry-run the output shown
                if args.iter().any(|a| a == "--dry-run") {
                    let res = self.git.git_fmt(&["notes".into(), "prune".into(), "--dry-run".into()]);
                    match res {
                        Ok(out) => self.view_text = Some(("git notes prune --dry-run".into(), out)),
                        Err(e) => self.set_msg(&format!("{e:#}")),
                    }
                } else {
                    self.run_notes(&["prune"]);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs() {
        assert_eq!(qualify("commits"), "refs/notes/commits");
        assert_eq!(qualify("refs/foo"), "refs/foo");
        assert_eq!(unqualify("refs/notes/x"), "x");
    }
}
