//! The suffixes of magit-bisect (magit-bisect.el).

use super::*;

/// transient-arg-value: the value of the option ARG in ARGS.
fn arg_value<'a>(args: &'a [String], arg: &str) -> Option<&'a str> {
    args.iter().find_map(|a| a.strip_prefix(arg)).filter(|v| !v.is_empty())
}

impl App {
    /// magit-bisect-terms: the lines of BISECT_TERMS (new, old).
    fn bisect_terms(&self) -> Vec<String> {
        std::fs::read_to_string(self.git.gitdir.join("BISECT_TERMS"))
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// magit-region-values '(commit branch) t: the selected commits and
    /// branches (at least two), top to bottom.
    pub(super) fn region_revs(&self) -> Option<Vec<String>> {
        let secs = self.region_sections(true)?;
        secs.iter()
            .map(|&s| match &self.buf.all[self.sec(s).start].kind {
                LineKind::Commit { hash, .. } => Some(hash.clone()),
                LineKind::Ref { kind: RefKind::Local | RefKind::Remote, name } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// magit-bisect-start-read-args: the bad and good revisions of the
    /// region when one is an ancestor of the other, else read.
    fn bisect_start_read_args(&mut self, args: &[String]) -> Result<Option<(String, String)>> {
        if let Some(revs) = self.region_revs() {
            let (a, b) = (revs[0].clone(), revs[revs.len() - 1].clone());
            let ancestor = |x: &str, y: &str| self.git.git_ok(&["merge-base", "--is-ancestor", x, y]);
            if ancestor(&a, &b) {
                return Ok(Some((b, a)));
            }
            if ancestor(&b, &a) {
                return Ok(Some((a, b)));
            }
        }
        let new = arg_value(args, "--term-new=").unwrap_or("bad").to_string();
        let old = arg_value(args, "--term-old=").unwrap_or("Good").to_string();
        let Some(bad) = self.read_branch_or_commit(&format!("Start bisect with {new} revision"))? else {
            return Ok(None);
        };
        let Some(good) = self.read_other_branch_or_commit_excluding(&format!("{old} revision"), Some(&bad))? else {
            return Ok(None);
        };
        Ok(Some((bad, good)))
    }

    /// magit-bisect-start--assert: a merge-base of BAD and GOOD, and no
    /// uncommitted changes.
    fn bisect_start_assert(&mut self, bad: &str, good: &str, args: &[String]) -> bool {
        if !self.git.git_ok(&["merge-base", bad, good]) {
            let old = arg_value(args, "--term-old=").unwrap_or("Good");
            let new = arg_value(args, "--term-new=").unwrap_or("bad");
            self.set_msg(&format!("{old} `{good}' or merge-base has to be an ancestor of {new} `{bad}')"));
            return false;
        }
        if self.git.anything_modified() {
            self.set_msg("Cannot bisect with uncommitted changes");
            return false;
        }
        true
    }

    /// Run `git bisect ARGS`, its output saved as BISECT_CMD_OUTPUT (for
    /// magit-insert-bisect-output) when it succeeds.
    fn bisect_git(&mut self, args: &[String]) -> bool {
        let mut a: Vec<String> = vec!["bisect".into()];
        a.extend(args.iter().cloned());
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        // Force an English locale: `magit-insert-bisect-output` detects the
        // "Bisecting:" line and the "<hash> is the first bad commit" line,
        // both of which are localized by git otherwise (breaking parsing and
        // the status-buffer heading in non-English environments).
        match self.git.git_output_env(&[("LC_ALL", "C")], &refs) {
            Ok(out) if out.status.success() => {
                let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&out.stderr));
                let _ = std::fs::write(self.git.gitdir.join("BISECT_CMD_OUTPUT"), text);
                true
            }
            Ok(out) => {
                let mut text = String::from_utf8_lossy(&out.stderr).trim().to_string();
                if text.is_empty() {
                    text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                }
                self.set_msg(&format!("git bisect {}: {text}", args.first().map_or("", String::as_str)));
                false
            }
            Err(e) => {
                self.set_msg(&format!("{e:#}"));
                false
            }
        }
    }

    /// magit-git-bisect: "Not bisecting" unless bisecting (or NO_ASSERT).
    fn git_bisect(&mut self, args: &[String], no_assert: bool) {
        if !no_assert && !self.git.bisect_in_progress() {
            self.set_msg("Not bisecting");
            return;
        }
        self.set_msg("Bisecting...");
        if self.bisect_git(args) {
            self.set_msg("Bisecting...done");
        }
        self.refresh();
    }

    /// The suffixes of the magit-bisect transient.
    pub(super) fn bisect_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let s = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };
        match cmd {
            "bisect-start" => {
                if self.git.bisect_in_progress() {
                    self.set_msg("Already bisecting");
                    return Ok(());
                }
                let Some((bad, good)) = self.bisect_start_read_args(args)? else {
                    return Ok(());
                };
                if !self.bisect_start_assert(&bad, &good, args) {
                    return Ok(());
                }
                let mut a = s(&["start"]);
                a.extend(args.iter().cloned());
                a.extend([bad, good]);
                self.git_bisect(&a, true);
            }
            "bisect-good" | "bisect-bad" => {
                let terms = self.bisect_terms();
                let term = if cmd == "bisect-bad" { terms.first() } else { terms.get(1) };
                match term.cloned() {
                    Some(t) => self.git_bisect(&[t], false),
                    None => self.set_msg("Not bisecting"),
                }
            }
            "bisect-mark" => {
                let terms = self.bisect_terms();
                let (Some(new), Some(old)) = (terms.first().cloned(), terms.get(1).cloned()) else {
                    self.set_msg("Not bisecting");
                    return Ok(());
                };
                let prompt = format!("Mark HEAD as {new} ([n]ew) or {old} ([o]ld)");
                match self.read_char_choice(&prompt, &['n', 'o'])? {
                    Some('n') => self.git_bisect(&[new], false),
                    Some(_) => self.git_bisect(&[old], false),
                    None => {}
                }
            }
            "bisect-skip" => self.git_bisect(&s(&["skip"]), false),
            "bisect-reset" => {
                if !self.confirm("reset-bisect", "Reset bisect")? {
                    return Ok(());
                }
                let res = self.git.git_fmt(&s(&["bisect", "reset"]));
                let _ = std::fs::remove_file(self.git.gitdir.join("BISECT_CMD_OUTPUT"));
                self.done("", res);
            }
            _ => {
                // magit-bisect-run: "git bisect start; git bisect run"
                // when not bisecting yet
                let start = if self.git.bisect_in_progress() {
                    None
                } else {
                    match self.bisect_start_read_args(args)? {
                        Some(revs) => Some(revs),
                        None => return Ok(()),
                    }
                };
                let Some(cmdline) = self.read_line_prompt("Bisect shell command: ", None)? else {
                    return Ok(());
                };
                if let Some((bad, good)) = start {
                    if !self.bisect_start_assert(&bad, &good, args) {
                        return Ok(());
                    }
                    let mut a = s(&["start"]);
                    a.extend([bad, good]);
                    a.extend(args.iter().cloned());
                    let ok = self.bisect_git(&a);
                    self.refresh();
                    if !ok {
                        return Ok(());
                    }
                }
                let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
                self.git_bisect(&[String::from("run"), shell, "-c".into(), cmdline], false);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        let args = ["--no-checkout".to_string(), "--term-old=fast".to_string()];
        assert_eq!(arg_value(&args, "--term-old="), Some("fast"));
        assert_eq!(arg_value(&args, "--term-new="), None);
    }
}
