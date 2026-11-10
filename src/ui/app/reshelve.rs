//! magit-reshelve-since (magit-extras.el): the author and committer
//! dates of the commits since a commit selected from the log changed,
//! one minute apart, with `git filter-branch`.

use super::*;
use crate::ui::minibuf::add_to_history;

/// magit--reshelve-history
const HISTORY: &str = "magit--reshelve-history";

/// The adjust of magit-reshelve-since: TIME at the start of its minute,
/// OFFSET minutes later.
fn adjust(time: i64, offset: i64) -> i64 {
    time - time.rem_euclid(60) + offset * 60
}

impl App {
    /// magit-commit-reshelve: the committer date of HEAD (and the author
    /// date if the user is its author, unless with a prefix argument)
    /// changed to a date read, the current time the initial input, the
    /// old date the history's latest element. ARGS are the commit
    /// arguments.
    pub(super) fn commit_reshelve(&mut self, args: &[String]) -> Result<()> {
        // magit-rev-author-p
        let author = |f: &str| self.git.git_string(&["log", "-1", &format!("--format={f}"), "HEAD"]).unwrap_or_default();
        let is_author = self.config("user.name").is_some_and(|n| n == author("%an").trim_end())
            || self.config("user.email").is_some_and(|e| e == author("%ae").trim_end());
        let update_author = is_author && self.prefix_arg.is_none();
        let format = format!("--format={}", if update_author { "%ad" } else { "%cd" });
        let old = self.git.git_string(&["log", "-1", &format, "--date=format:%F %T %z", "HEAD"]).unwrap_or_default();
        add_to_history(self.infix_history.borrow_mut().entry(HISTORY.into()).or_default(), old.trim_end(), true);
        let now = crate::ui::logbuf::format_time("%F %T %z", crate::ui::logbuf::now().floor() as i64);
        *self.initial_input.borrow_mut() = Some(now);
        let prompt = if update_author { "Change author and committer dates to: " } else { "Change committer date to: " };
        let Some(date) = self.read_line_hist(prompt, None, Hist::Infix(HISTORY), None)? else {
            return Ok(());
        };
        add_to_history(self.infix_history.borrow_mut().entry(HISTORY.into()).or_default(), &date, true);
        let mut a: Vec<String> = vec!["commit".into(), "--amend".into(), "--no-edit".into()];
        if update_author {
            a.push(format!("--date={date}"));
        }
        a.extend(args.iter().cloned());
        let strs: Vec<&str> = a.iter().map(String::as_str).collect();
        let res = self.git.git_interactive_env(&[("GIT_COMMITTER_DATE", date.as_str())], &strs);
        self.done("", res.map(|()| String::new()));
        Ok(())
    }

    /// magit-reshelve-since without a commit: the branch checked out, the
    /// backup ref of an earlier run overridden only if confirmed, then the
    /// commit selected from the log. KEYID is the value of --gpg-sign= of
    /// the rebase arguments.
    pub(super) fn reshelve_since_select(&mut self, keyid: Option<String>) -> Result<()> {
        let Some(current) = self.git.current_branch() else {
            self.set_msg("Refusing to reshelve detached head");
            return Ok(());
        };
        let backup = format!("refs/original/refs/heads/{current}");
        if self.git.git_ok(&["show-ref", "--verify", "--quiet", &backup])
            && !self.magit_y_or_n_p(&format!("Backup ref {backup} already exists.  Override? "), None)?
        {
            self.set_msg("Abort");
            return Ok(());
        }
        let initial = self.commit_at_point();
        let msg = "Type %p on a commit to reshelve it and the commits above it,";
        self.log_select(SelectAction::ReshelveSince { keyid }, Some(msg), initial);
        Ok(())
    }

    /// magit-reshelve-since with COMMIT: the date of COMMIT read (the
    /// initial input a minute before now for each commit, the history's
    /// latest element a minute after the parent of COMMIT), the next
    /// commits a minute later each.
    pub(super) fn reshelve_since(&mut self, commit: &str, keyid: Option<&str>) -> Result<()> {
        let Some(current) = self.git.current_branch() else {
            self.set_msg("Refusing to reshelve detached head");
            return Ok(());
        };
        let start = format!("{commit}^");
        let range = format!("{start}..{current}");
        let Some(parent_time) = self
            .git
            .git_string(&["log", "-1", "--format=%at", &start])
            .ok()
            .and_then(|t| t.trim().parse::<i64>().ok())
        else {
            self.set_msg(&format!("Cannot reshelve {commit}: it has no parent"));
            return Ok(());
        };
        let count = self
            .git
            .git_string(&["rev-list", "--count", &range])
            .ok()
            .and_then(|n| n.trim().parse::<i64>().ok())
            .unwrap_or(0);
        let fmt = |t: i64| crate::ui::logbuf::format_time("%F %T %z", t);
        let time_rev = fmt(adjust(parent_time, 1));
        let time_now = fmt(adjust(crate::ui::logbuf::now().floor() as i64, -count));
        add_to_history(self.infix_history.borrow_mut().entry(HISTORY.into()).or_default(), &time_rev, true);
        *self.initial_input.borrow_mut() = Some(time_now);
        let Some(input) = self.read_line_hist("Date for first commit: ", None, Hist::Infix(HISTORY), None)? else {
            return Ok(());
        };
        add_to_history(self.infix_history.borrow_mut().entry(HISTORY.into()).or_default(), &input, true);
        // date-to-time: git's date parser
        let Some(mut date) = self
            .git
            .git_string(&["rev-parse", &format!("--since={input}")])
            .ok()
            .and_then(|s| s.trim().strip_prefix("--max-age=").and_then(|n| n.parse::<i64>().ok()))
        else {
            self.set_msg(&format!("Invalid date: {input}"));
            return Ok(());
        };
        let author = !self.options.reshelve_since_committer_only;
        let cases: Vec<String> = self
            .git
            .git_lines(&["rev-list", "--reverse", &range])
            .into_iter()
            .map(|rev| {
                let mut case = format!("{rev}) ");
                if author {
                    case.push_str(&format!("export GIT_AUTHOR_DATE=\"{date}\"; "));
                }
                case.push_str(&format!("export GIT_COMMITTER_DATE=\"{date}\";;"));
                date += 60;
                case
            })
            .collect();
        let filter = format!("case $GIT_COMMIT in {}\nesac", cases.join(" "));
        let commit_filter = keyid.map(|k| format!("git commit-tree --gpg-sign={k} \"$@\";"));
        let mut a = vec!["filter-branch", "--force", "--env-filter", &filter];
        if let Some(f) = &commit_filter {
            a.extend(["--commit-filter", f.as_str()]);
        }
        a.extend([range.as_str(), "--"]);
        let res = self.git.git_interactive_env(&[("FILTER_BRANCH_SQUELCH_WARNING", "1")], &a);
        self.done("", res.map(|()| String::new()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjusted_times() {
        assert_eq!(adjust(125, 0), 120);
        assert_eq!(adjust(125, 1), 180);
        assert_eq!(adjust(125, -2), 0);
    }
}
