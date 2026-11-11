//! ui/app/diffsel.rs — diff selection (magit-diff-scope): what is at point.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

impl App {

    /// magit-apply--get-selection + magit-diff-scope + magit-diff-type.
    pub(super) fn diff_selection(&self) -> Option<DiffSel> {
        if let Some(secs) = self.region_sections(true) {
            let kinds: Vec<&LineKind> = secs.iter().map(|&s| &self.buf.all[self.sec(s).start].kind).collect();
            let files: Vec<(String, String)> = kinds
                .iter()
                .filter_map(|k| match k {
                    LineKind::File { section, path } => Some((section.clone(), path.clone())),
                    _ => None,
                })
                .collect();
            if files.len() == kinds.len() {
                let typ = files[0].0.clone();
                return Some(DiffSel::Files { typ, paths: files.into_iter().map(|f| f.1).collect() });
            }
            let hunks: Vec<(String, String, usize)> = kinds
                .iter()
                .filter_map(|k| match k {
                    LineKind::Hunk { section, path, hunk } => Some((section.clone(), path.clone(), *hunk)),
                    _ => None,
                })
                .collect();
            if hunks.len() == kinds.len() {
                let (typ, path) = (hunks[0].0.clone(), hunks[0].1.clone());
                return Some(DiffSel::Hunks { typ, path, hunks: hunks.into_iter().map(|h| h.2).collect() });
            }
        }
        let s = self.cur_sec();
        if s == 0 {
            return None;
        }
        let start = self.sec(s).start;
        let is_list = |t: &str| matches!(t, "untracked" | "unstaged" | "staged");
        match &self.buf.all[start].kind {
            LineKind::Hunk { section, path, hunk } => {
                if let Some(m) = self.mark {
                    let p = self.point();
                    let line = |q: usize| match self.buf.all[q].kind {
                        LineKind::DiffLine { line, .. } => Some(line),
                        _ => None,
                    };
                    if self.buf.section_at(m) == s && m != start && p != start {
                        if let (Some(a), Some(b)) = (line(m), line(p)) {
                            return Some(DiffSel::Region {
                                typ: section.clone(),
                                path: path.clone(),
                                hunk: *hunk,
                                from: a.min(b),
                                to: a.max(b),
                            });
                        }
                    }
                }
                Some(DiffSel::Hunk { typ: section.clone(), path: path.clone(), hunk: *hunk })
            }
            LineKind::File { section, path } => Some(DiffSel::File { typ: section.clone(), path: path.clone() }),
            LineKind::Meta { section, path, kind } => Some(DiffSel::Meta {
                typ: section.clone(),
                path: path.clone(),
                scope: if kind == "chmod" { Scope::Chmod } else { Scope::Rename },
            }),
            LineKind::SectionHeader { section, .. } if is_list(section) => Some(DiffSel::List { typ: section.clone() }),
            _ => None,
        }
    }

    pub(super) fn context(&self) -> usize {
        diff_context(&self.buffer_args())
    }

    pub(super) fn file_diff(&self, typ: &str, path: &str) -> Result<FileDiff> {
        self.buf
            .diff(typ, path)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Cannot get required diff headers"))
    }

    /// Paths of the files a file / files / list selection covers.
    pub(super) fn sel_paths(&self, sel: &DiffSel) -> Vec<String> {
        match sel {
            DiffSel::File { path, .. } => vec![path.clone()],
            DiffSel::Files { paths, .. } => paths.clone(),
            DiffSel::List { typ } if typ == "untracked" => self.git.untracked_files(),
            DiffSel::List { typ } => self.buf.files(typ).iter().map(|f| f.path.clone()).collect(),
            DiffSel::Hunk { path, .. }
            | DiffSel::Hunks { path, .. }
            | DiffSel::Region { path, .. }
            | DiffSel::Meta { path, .. } => vec![path.clone()],
        }
    }

    /// magit-apply--ignore-whitespace-p: whole files have to be applied as
    /// patches when the diff ignores whitespace (and no file is binary).
    pub(super) fn ignore_whitespace(&self, sel: &DiffSel) -> bool {
        let group = sel.typ();
        if !matches!(self.diff_type(group), "unstaged" | "staged") || !sel.is_files() {
            return false;
        }
        let flags = ["--ignore-space-at-eol", "--ignore-space-change", "--ignore-all-space", "--ignore-blank-lines"];
        self.buffer_args().iter().any(|a| flags.contains(&a.as_str()))
            && !self.sel_paths(sel).iter().any(|p| self.buf.diff(group, p).is_some_and(|d| d.binary))
    }

    /// Hunk / hunks / region (or pseudo-hunk) selection as (path, scope).
    pub(super) fn hunk_scope(sel: &DiffSel) -> Option<(String, Scope)> {
        match sel {
            DiffSel::Hunk { path, hunk, .. } => Some((path.clone(), Scope::Hunk(*hunk))),
            DiffSel::Hunks { path, hunks, .. } => Some((path.clone(), Scope::Hunks(hunks.clone()))),
            DiffSel::Region { path, hunk, from, to, .. } => {
                Some((path.clone(), Scope::Region { hunk: *hunk, from: *from, to: *to }))
            }
            DiffSel::Meta { path, scope, .. } => Some((path.clone(), scope.clone())),
            _ => None,
        }
    }

    /// The prompt of magit-discard-* / magit-reverse-* for a hunk selection.
    pub(super) fn hunk_prompt(verb: &str, sel: &DiffSel) -> String {
        match sel {
            DiffSel::Region { .. } => format!("{verb} region"),
            DiffSel::Hunks { hunks, path, .. } => format!("{verb} {} hunks from {path}", hunks.len()),
            _ => format!("{verb} hunk"),
        }
    }

    /// magit-confirm: ask PROMPT (with "?") unless ACTION is in
    /// magit-no-confirm ("t": always ask); declining shows "Abort".
    pub(super) fn confirm(&mut self, action: &str, prompt: &str) -> Result<bool> {
        let ok = self.confirm_noabort(action, prompt)?;
        if !ok {
            self.set_msg("Abort");
        }
        Ok(ok)
    }

    /// magit-confirm with NOABORT: declining isn't an error.
    pub(super) fn confirm_noabort(&mut self, action: &str, prompt: &str) -> Result<bool> {
        if self.options.no_confirm(action) {
            return Ok(true);
        }
        self.magit_y_or_n_p(&format!("{prompt}?"), Some(action))
    }

    /// magit-read-stash: one of the stashes ("stash@{N} subject",
    /// require-match), the one at point the default, else the first.
    pub(super) fn read_stash(&mut self, prompt: &str) -> Result<Option<String>> {
        let choices: Vec<String> = self.git.stashes().into_iter().map(|(n, m)| format!("{n} {m}")).collect();
        if choices.is_empty() {
            self.set_msg("No stashes");
            return Ok(None);
        }
        let atpoint = match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Stash { name, .. }) => Some(name),
            _ => None,
        };
        let default = atpoint
            .and_then(|a| choices.iter().find(|c| c.split(' ').next() == Some(a.as_str())).cloned())
            .unwrap_or_else(|| choices[0].clone());
        let table = Completion { candidates: choices.clone(), require_match: true, multiple: false };
        let Some(v) = self.completing_read(prompt, Some(&default), Hist::Minibuffer, Some(table))?.filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        // a choice, or just the name of one
        let name = choices.iter().filter_map(|c| c.split(' ').next()).find(|n| v == *n || v.starts_with(&format!("{n} ")));
        if name.is_none() {
            self.set_msg("[No match]");
        }
        Ok(name.map(str::to_string))
    }

    /// magit-stash-drop: the stashes selected by the region (confirmed,
    /// drop-stashes), else one read; dropped from the last.
    pub(super) fn stash_drop(&mut self) -> Result<()> {
        let region: Option<Vec<String>> = self.region_sections(false).and_then(|secs| {
            secs.iter()
                .map(|&s| match &self.buf.all[self.sec(s).start].kind {
                    LineKind::Stash { name, .. } => Some(name.clone()),
                    _ => None,
                })
                .collect()
        });
        let stashes = match region {
            Some(v) => {
                if !self.confirm_items("drop-stashes", "Drop stashes", "Drop %d stashes", &v)? {
                    return Ok(());
                }
                v
            }
            None => match self.read_stash("Drop stash")? {
                Some(s) => vec![s],
                None => return Ok(()),
            },
        };
        self.clear_region();
        let mut msg = String::new();
        for s in stashes.iter().rev() {
            msg = format!("Deleted refs/{s} (was {})", self.git.rev_short(s));
            if let Err(e) = self.git.stash_drop(s) {
                msg = format!("{e:#}");
                break;
            }
        }
        self.refresh();
        self.set_msg(&msg);
        Ok(())
    }

    /// magit-confirm with ITEMS: ONE with `%s` the item, else the items
    /// and MANY with `%d` their count; "Abort" when declined.
    pub(super) fn confirm_items(&mut self, action: &str, one: &str, many: &str, items: &[String]) -> Result<bool> {
        match items {
            [] => Ok(true),
            [i] => self.confirm(action, &one.replacen("%s", i, 1)),
            is => self.confirm(action, &format!("{}\n\n{}", is.join("\n"), many.replacen("%d", &is.len().to_string(), 1))),
        }
    }

    /// magit-confirm-files: `PROMPT "file"` or the file list and `PROMPT N files`.
    pub(super) fn confirm_files(&mut self, action: &str, prompt: &str, files: &[String]) -> Result<bool> {
        match files {
            [] => Ok(true),
            [f] => self.confirm(action, &format!("{prompt} \"{f}\"")),
            fs => self.confirm(action, &format!("{}\n\n{prompt} {} files", fs.join("\n"), fs.len())),
        }
    }

}
