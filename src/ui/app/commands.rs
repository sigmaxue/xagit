//! ui/app/commands.rs — command dispatch (run_command) and infix option readers.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

/// Resolve INPUT to an absolute path relative to BASE (the directory xagit
/// was started in).  Unlike `canonicalize` this also works for a path that
/// does not exist yet (a new repository): it only normalizes `.`/`..`.
pub(super) fn absolute_in(base: &std::path::Path, input: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(input);
    let joined = if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
    let mut out = std::path::PathBuf::new();
    for c in joined.components() {
        match c {
            std::path::Component::ParentDir => { out.pop(); }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl App {
    /// Run a repository-level git command (init / clone) in DIR.  Its
    /// output is captured so a failure is reported with git's stderr and a
    /// hint; when git wanted a credential / host-key prompt, run it once
    /// more in the terminal so the user can answer.
    fn run_repo_git(&mut self, dir: &std::path::Path, args: &[&str]) -> Result<()> {
        match self.git.git_output_at(dir, args) {
            Ok(()) => Ok(()),
            Err((detail, true)) => {
                let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
                let strs: Vec<&str> = owned.iter().map(String::as_str).collect();
                let d = dir.to_path_buf();
                self.with_terminal(|g| g.git_tty_at(&d, &strs))
                    .map_err(|_| anyhow::anyhow!("{detail}"))
            }
            Err((detail, false)) => Err(anyhow::anyhow!("{detail}")),
        }
    }

    /// Read the value of the option ARG (transient-infix-read). None: the
    /// user quit (nothing changes); Some(None): unset; Some(Some(v)): set
    /// to ARG followed by V.
    pub(super) fn read_option(&mut self, arg: &str, reader: &Reader) -> Result<Option<Option<String>>> {
        let value = self.read_option_value(arg, reader)?;
        // transient-infix-read: the value read becomes the newest of the
        // infix's history
        // magit-tag:--local-user has the :history-key magit:--gpg-sign
        let key = if *reader == Reader::GpgKey { "--gpg-sign=" } else { arg };
        if let Some(Some(v)) = &value {
            add_to_history(self.infix_history.borrow_mut().entry(key.to_string()).or_default(), v, true);
        }
        Ok(value)
    }

    pub(super) fn read_option_value(&mut self, arg: &str, reader: &Reader) -> Result<Option<Option<String>>> {
        let value = match reader {
            Reader::Choices(choices) => {
                let prompt = format!("{arg} [unspecified, {}]", choices.join(", "));
                let Some(v) = self.read_infix(&prompt, Some("unspecified"), arg)?.filter(|v| !v.is_empty()) else {
                    return Ok(None);
                };
                if "unspecified".starts_with(v.as_str()) {
                    return Ok(Some(None));
                }
                match choices.iter().find(|c| c.starts_with(v.as_str())) {
                    Some(c) => c.clone(),
                    None => {
                        self.set_msg(&format!("{v} is not one of {}", choices.join(", ")));
                        return Ok(None);
                    }
                }
            }
            Reader::GpgKey => {
                let Some(v) = self.read_gpg_signing_key(arg)? else {
                    return Ok(None);
                };
                // only --gpg-sign has :allow-empty
                if v.is_empty() && arg != "--gpg-sign=" {
                    return Ok(None);
                }
                v
            }
            Reader::Pick(choices) => {
                let table = Completion { candidates: choices.clone(), require_match: true, multiple: false };
                let read = self.completing_read(arg, None, Hist::Infix(arg), Some(table))?;
                let Some(v) = read.filter(|v| !v.is_empty()) else {
                    return Ok(None);
                };
                v
            }
            // magit-read-files of magit-modified-files, the file at point
            // the initial input
            Reader::Files | Reader::TrackedFiles => {
                let candidates = if *reader == Reader::Files {
                    self.git.git_lines(&["diff-index", "-z", "--name-only", "--submodule=short", "HEAD", "--"])
                } else {
                    self.git.git_lines(&["ls-files", "-z", "--full-name"])
                };
                *self.initial_input.borrow_mut() = self.current_file_name();
                let table = Completion { candidates, require_match: false, multiple: true };
                let Some(input) = self.read_line_hist("Limit to file,s: ", None, Hist::Infix(arg), Some(table))? else {
                    return Ok(None);
                };
                let files: Vec<&str> = input.split(',').map(str::trim).filter(|f| !f.is_empty()).collect();
                if files.is_empty() {
                    return Ok(None);
                }
                files.join(" ")
            }
            Reader::Revision => {
                let mut candidates = vec!["HEAD".to_string()];
                candidates.extend(self.git.refnames());
                let default = self.branch_or_commit_at_point().or_else(|| self.git.current_branch());
                let table = Completion { candidates, ..Default::default() };
                let Some(v) = self.read_any(arg, default.as_deref(), Hist::Infix(arg), table)? else {
                    return Ok(None);
                };
                v
            }
            Reader::NotesRef => {
                let Some(v) = self.notes_read_ref(arg, arg)? else {
                    return Ok(None);
                };
                v
            }
            Reader::CharCase(prompt, clauses) => {
                let labels: Vec<&str> = clauses.iter().map(|(_, l, _)| l.as_str()).collect();
                let prompt = format!("{prompt}{}, or [C-g] to abort ", labels.join(", "));
                let keys: Vec<char> = clauses.iter().map(|(k, _, _)| *k).collect();
                let Some(c) = self.read_char_choice(&prompt, &keys)? else {
                    return Ok(None);
                };
                match clauses.iter().find(|(k, _, _)| *k == c) {
                    Some((_, _, v)) => v.clone(),
                    None => return Ok(None),
                }
            }
            Reader::Person | Reader::PatchBase | Reader::RangeOrCommit | Reader::ExistingDirectory => {
                let Some(v) = self.read_patch_infix(reader, arg)? else {
                    return Ok(None);
                };
                v
            }
            Reader::SubtreePrefix => {
                let Some(v) = self.subtree_read_prefix(arg)? else {
                    return Ok(None);
                };
                v
            }
            Reader::FileTrace => {
                let Some(file) = self.read_line_prompt("File", self.current_file().map(|(_, p)| p).as_deref())? else {
                    return Ok(None);
                };
                let Some(trace) = self.read_line_prompt("Trace", None)? else {
                    return Ok(None);
                };
                format!("{trace}:{file}")
            }
            _ => {
                // an option with :allow-empty is the argument alone when
                // the answer is empty
                let read = self.read_infix(arg, None, arg)?;
                let read = if *reader == Reader::OptionalNumber { read } else { read.filter(|v| !v.is_empty()) };
                let Some(v) = read else {
                    return Ok(None);
                };
                let valid = match reader {
                    Reader::Number { zero } => v.parse::<u64>().is_ok_and(|n| *zero || n > 0),
                    Reader::OptionalNumber => v.is_empty() || v.parse::<u64>().is_ok_and(|n| n > 0),
                    Reader::Similarity => {
                        let digits = v.strip_suffix('%').unwrap_or(&v);
                        !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
                    }
                    _ => true,
                };
                if !valid {
                    let what = match reader {
                        Reader::Number { zero: true } => "a non-negative integer".to_string(),
                        Reader::Number { zero: false } | Reader::OptionalNumber => "a positive integer".to_string(),
                        _ => "a percentage ending in %, or a fraction, omitting the implied leading \"0.\"".to_string(),
                    };
                    self.set_msg(&format!("Please enter {what}"));
                    return Ok(None);
                }
                v
            }
        };
        Ok(Some(Some(value)))
    }
    pub(super) fn args_from(&self, active: &BTreeMap<String, bool>) -> Vec<String> {
        active
            .iter()
            .filter(|(_, on)| **on)
            .map(|(k, _)| k.clone())
            .collect()
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn run_command(&mut self, cmd: &str, active: &BTreeMap<String, bool>) -> Result<()> {
        // magit-branch-configure / magit-remote-configure: the scope the
        // current branch or remote, unless with a prefix argument or
        // invoked from magit-branch / magit-remote, then the one read
        if cmd == "branch-config-menu" || cmd == "remote-configure-menu" {
            let branch = cmd == "branch-config-menu";
            let from = if branch { "branch-menu" } else { "remote-menu" };
            let current = if branch { self.git.current_branch() } else { self.current_remote() };
            let direct = self.prefix_arg.is_none() && self.invoking_menu.as_deref() != Some(from);
            let scope = match current.filter(|_| direct) {
                Some(s) => s,
                None => {
                    let read = if branch {
                        self.read_local_branch("Configure branch")?
                    } else {
                        self.read_remote("Configure remote")?
                    };
                    let Some(s) = read else {
                        return Ok(());
                    };
                    s
                }
            };
            self.var_scopes.insert(cmd.to_string(), Some(scope));
            self.open_menu(cmd);
            return Ok(());
        }
        // the git variable infixes (from M-x: the scope read)
        if let Some(v) = gitvar::lookup(cmd) {
            let menu = self.invoking_menu.clone();
            return self.git_var_command(v, menu.as_deref());
        }
        // a menu used as a command (e.g. dispatch "b Branch") opens that menu
        if self.menus.contains_key(cmd) {
            self.open_menu(cmd);
            return Ok(());
        }
        let args = self.args_from(active);
        macro_rules! g {
            () => {
                &self.git
            };
        }
        match cmd {
            "refresh" => {
                self.refresh();
                self.set_msg("refreshed");
            }
            // magit-blame (B)
            "blame-addition" => self.blame(BlameType::Addition, args)?,
            "blame-removal" => self.blame(BlameType::Removal, args)?,
            "blame-reverse" => self.blame(BlameType::Final, args)?,
            "blame-echo" => self.blame_echo(args)?,
            "blame-quit" => self.blame_quit(),
            // magit-file-dispatch
            "file-stage" | "file-unstage" | "file-untrack" | "file-rename" | "file-delete" | "file-checkout"
            | "diff-buffer-file" | "log-buffer-file" | "log-trace-definition" | "blob-previous" | "blob-next"
            | "find-file" | "blob-visit-file" | "status-here" | "edit-line-commit" => self.file_command(cmd)?,
            "stage-files" => self.stage_files()?,
            "unstage-files" => self.unstage_files()?,
            "display-repository-buffer" | "display-buffer" => self.display_repository_buffer()?,
            "execute-extended-command" => self.execute_extended_command()?,
            "quit" => self.mode_bury_buffer(),
            "toggle-section" => self.toggle_section(),
            "visit" => self.visit_thing()?,
            "jump" => self.jump_command(),
            "jump-to-stashes" | "jump-to-untracked" | "jump-to-unstaged" | "jump-to-staged" | "jump-to-unpulled-from-upstream"
            | "jump-to-unpulled-from-pushremote" | "jump-to-unpushed-to-upstream" | "jump-to-unpushed-to-pushremote" => {
                self.section_jumper(cmd)
            }
            "imenu" => self.imenu()?,
            // magit-status-quick: the status buffer (shown again)
            "status-quick" => self.switch_to_mode(BufferMode::Status),
            "help" => {
                self.view_text = Some((
                    "Key bindings".into(),
                    "h/? dispatch popup — all commands live there.\n\
                     n/p move, j jump to section, ^ section up\n\
                     s stage, u unstage, S stage all, U unstage all\n\
                     k discard, K untrack, x reset quickly, a apply, v reverse\n\
                     c commit, b branch, z stash, P push, F pull, f fetch\n\
                     m merge, r rebase, X reset, t tag, M remote\n\
                     l log, d diff, L/D change log/diff arguments, A cherry-pick, V revert\n\
                     w apply patches, W patch, o submodule, $ git process log\n\
                     g refresh, TAB toggle section, RET visit, q quit"
                        .into(),
                ));
            }

            // -------- staging
            "stage" => self.stage_cmd()?,
            "stage-all" => self.stage_modified()?,
            "unstage" => self.unstage_cmd()?,
            "unstage-all" => self.unstage_all()?,
            "discard" if self.ref_thing().is_some() => match self.ref_thing() {
                Some(RefThing::Tag) => self.run_command("tag-delete", &BTreeMap::new())?,
                Some(RefThing::Remote) => self.run_command("remote-remove", &BTreeMap::new())?,
                _ => self.branch_delete()?,
            },
            "discard" => self.discard_cmd()?,
            // `K` is magit-file-untrack: the full command (region,
            // confirmation, "Already untracked" and prefix --force) is
            // implemented by file-untrack, also bound in file-dispatch.
            "untrack" => self.file_command("file-untrack")?,
            "diff-show-or-scroll-up" => self.show_or_scroll(true)?,
            "diff-show-or-scroll-down" => self.show_or_scroll(false)?,
            "diff-more-context" | "diff-less-context" => {
                // by the numeric prefix argument
                let count = self.prefix_numeric();
                let count = if cmd == "diff-less-context" { -count } else { count };
                self.set_diff_context(|n| Some((n as i64 + count).max(0) as usize));
            }
            "diff-default-context" => self.set_diff_context(|_| None),

            // -------- commit menu: the message is written in the editor
            // (magit-run-git-with-editor); the terminal is handed over
            "commit-create" => {
                let Some(args) = self.commit_assert(&args, false, false)? else {
                    return Ok(());
                };
                self.commit_msg_open(args, Vec::new(), "committed")?;
            }
            // magit-commit-extend (magit-commit-extend-override-date)
            "commit-extend" => {
                let Some(args) = self.commit_assert(&args, false, false)? else {
                    return Ok(());
                };
                if !self.amend_assert("HEAD")? {
                    return Ok(());
                }
                // with a prefix argument keep the committer date
                let date = self.keep_committer_date();
                let env: Vec<(&str, &str)> = date.iter().map(|d| ("GIT_COMMITTER_DATE", d.as_str())).collect();
                let res = g!().commit_extend_env(&args, &env);
                self.done("extended HEAD commit", res.map(|()| String::new()));
            }
            "commit-amend" => {
                if !self.amend_assert("HEAD")? {
                    return Ok(());
                }
                let mut a = vec!["--amend".to_string()];
                a.extend(args.iter().cloned());
                self.commit_msg_open(a, Vec::new(), "amended HEAD commit")?;
            }
            // magit-commit-reword: the message only, not the staged changes
            "commit-reword" => {
                if !self.amend_assert("HEAD")? {
                    return Ok(());
                }
                let mut a = vec!["--amend".to_string(), "--only".to_string()];
                a.extend(args.iter().cloned());
                if !a.iter().any(|x| x == "--allow-empty") {
                    a.push("--allow-empty".into());
                }
                // with a prefix argument keep the committer date
                let date = self.keep_committer_date();
                let env: Vec<(String, String)> = date.iter().map(|d| ("GIT_COMMITTER_DATE".to_string(), d.clone())).collect();
                self.commit_msg_open(a, env, "reworded HEAD commit")?;
            }
            // magit-commit-squash-internal: with magit-commit-squash-confirm
            // the target is always selected from the log, starting at the
            // commit at point
            "commit-fixup" | "commit-squash" | "commit-alter" | "commit-augment" | "commit-revise"
            | "commit-instant-fixup" | "commit-instant-squash" => {
                let (option, edit, nopatch, rebase) = match cmd {
                    "commit-fixup" => ("--fixup=", false, false, false),
                    "commit-squash" => ("--squash=", false, false, false),
                    "commit-alter" => ("--fixup=amend:", true, false, false),
                    "commit-augment" => ("--squash=", true, false, false),
                    "commit-revise" => ("--fixup=reword:", true, true, false),
                    "commit-instant-fixup" => ("--fixup=", false, false, true),
                    _ => ("--squash=", false, false, true),
                };
                let Some(args) = self.commit_assert(&args, nopatch, !edit)? else {
                    return Ok(());
                };
                let commit = self.commit_at_point();
                if let Some(c) = &commit {
                    if rebase && !self.git.is_ancestor(c, "HEAD") {
                        let prompt = format!(
                            "{} isn't an ancestor of HEAD.  [c]reate without rebasing, [s]elect other, [a]bort",
                            self.git.rev_short(c)
                        );
                        let answer = self.read_line_prompt(&prompt, None)?.unwrap_or_default();
                        let rebase = match answer.chars().next() {
                            Some('c') => false,
                            Some('s') => true,
                            _ => {
                                self.set_msg("Quit");
                                return Ok(());
                            }
                        };
                        let action = SelectAction::CommitSquash { option: option.into(), args, edit, rebase };
                        let initial = (!rebase).then(|| c.clone());
                        let msg = format!("Type %p on a commit to {} into it,", &option[2..]);
                        self.log_select(action, Some(&msg), initial);
                        return Ok(());
                    }
                }
                let action = SelectAction::CommitSquash { option: option.into(), args, edit, rebase };
                let msg = format!("Type %p on a commit to {} into it,", &option[2..]);
                self.log_select(action, Some(&msg), commit);
            }

            // -------- branch menu
            // magit-checkout: a local branch becomes the current branch,
            // anything else (a remote branch, a tag, a commit) detaches HEAD
            "checkout" => {
                let Some(commit) = self.read_other_branch_or_commit("Checkout")? else {
                    return Ok(());
                };
                self.checkout_rev(&commit, &args);
            }
            "branch-checkout" => self.branch_checkout(&args)?,
            // magit-branch-and-checkout
            "branch-and-checkout" => {
                let Some((name, start)) = self.branch_read_args("Create and checkout branch")? else {
                    return Ok(());
                };
                let res = if is_stash_ref(&start) {
                    g!().git_interactive(&["stash", "branch", &name, &start])
                } else {
                    let mut a: Vec<&str> = vec!["checkout"];
                    a.extend(args.iter().map(String::as_str));
                    a.extend(["-b", &name, &start]);
                    let res = g!().git_interactive(&a);
                    self.branch_maybe_adjust_upstream(&name, &start);
                    res
                };
                self.done("", res.map(|_| String::new()));
            }
            // magit-branch-create
            "branch-create" => {
                let Some((name, start)) = self.branch_read_args("Create branch")? else {
                    return Ok(());
                };
                let res = g!().git_interactive(&["branch", &name, &start]);
                self.branch_maybe_adjust_upstream(&name, &start);
                self.done("", res.map(|_| String::new()));
            }
            // magit-branch-spinoff / -spinout: FROM the last (oldest)
            // commit selected
            // magit-branch-orphan
            "branch-orphan" => {
                let Some((name, start)) = self.branch_read_args("Create and checkout orphan branch")? else {
                    return Ok(());
                };
                let res = g!().git_interactive(&["checkout", "--orphan", &name, &start]);
                self.done_keep_msg(res);
            }
            "branch-shelve" => self.branch_shelve()?,
            "branch-unshelve" => self.branch_unshelve()?,
            "delete-shelved-branch" => {
                if let Some(b) = self.read_shelved_branch("Delete shelved branch")? {
                    let res = g!().git_interactive(&["update-ref", "-d", &format!("refs/shelved/{b}")]);
                    self.done_keep_msg(res);
                }
            }
            "branch-spinoff" | "branch-spinout" => {
                let checkout = cmd == "branch-spinoff";
                let from = self.region_commit_values().and_then(|v| v.last().cloned());
                let prompt = if checkout { "Spin off branch" } else { "Spin out branch" };
                let Some(name) = self.read_string_ns(prompt)? else {
                    return Ok(());
                };
                self.branch_spinoff(&name, from.as_deref(), checkout);
            }
            // magit-branch-rename
            "branch-rename" => {
                let bs = g!().branches();
                // magit-read-local-branch
                let default = self.local_branch_at_point().or_else(|| g!().current_branch());
                let Some(old) = self.pick_rev("Rename branch", &bs, default.as_deref())?.cloned() else {
                    return Ok(());
                };
                let Some(new) = self.read_line_prompt(&format!("Rename branch '{old}' to"), None)? else {
                    return Ok(());
                };
                self.branch_rename(&old, &new)?;
            }
            "branch-reset" => self.branch_reset()?,
            "branch-delete" => self.branch_delete()?,

            // -------- remote menu
            "remote-add" | "remote-rename" | "remote-remove" | "remote-prune" | "remote-prune-refspecs"
            | "remote-unshallow" => self.remote_command(cmd, &args)?,

            // -------- fetch/pull/push (upstream = branch.<b>.remote / branch.<b>.merge)
            "fetch-from-pushremote" | "fetch-from-upstream" | "fetch-other" | "fetch-branch" | "fetch-refspec"
            | "fetch-all" | "fetch-modules" | "fetch-modules-run" | "pull-from-pushremote" | "pull-from-upstream"
            | "pull-into-upstream" | "pull-branch" => self.fetch_pull_command(cmd, &args)?,
            "push-current-to-pushremote" | "push-current-to-upstream" | "push-current" | "push-other" | "push-refspecs"
            | "push-matching" | "push-tags" | "push-tag" | "push-notes-ref" => self.push_command(cmd, &args)?,

            // -------- merge / rebase
            "merge-plain" | "merge-nocommit" | "merge-squash" | "merge-editmsg" | "merge-preview" => {
                let mut bs = g!().branches();
                bs.extend(g!().remote_branches());
                let Some(rev) = self.pick_rev("Merge revision", &bs, None)?.cloned() else {
                    return Ok(());
                };
                let res = match cmd {
                    "merge-plain" => g!().merge(&rev, &args),
                    "merge-nocommit" => g!().merge_nocommit(&rev, &args),
                    "merge-squash" => g!().merge_squash(&rev),
                    "merge-editmsg" => {
                        let into = g!().current_branch().unwrap_or_else(|| "HEAD".into());
                        let default = format!("Merge branch '{rev}' into {into}");
                        let Some(msg) = self.read_line_prompt("Merge message", Some(&default))? else {
                            return Ok(());
                        };
                        g!().merge_with_message(&rev, &msg, &args)
                    }
                    _ => {
                        match g!().merge_preview(&rev) {
                            Ok(p) if p.trim().is_empty() => self.set_msg("merge would not change anything"),
                            Ok(p) => self.view_text = Some((format!("preview merge of {rev}"), p)),
                            Err(e) => self.report_error(&e),
                        }
                        return Ok(());
                    }
                };
                // A merge that stops on conflicts is a state, not a failure;
                // report_error() names the keys that commit or abort it.
                match res {
                    Ok(()) => self.set_msg("merged"),
                    Err(e) => self.report_error(&e),
                }
                self.refresh();
            }
            "merge-commit" => {
                match g!().merge_commit() {
                    Ok(()) => self.set_msg("merge committed"),
                    Err(e) => self.report_error(&e),
                }
                self.refresh();
            }
            // magit-merge-abort
            "merge-abort" => {
                if !g!().merge_in_progress() {
                    self.set_msg("No merge in progress");
                    return Ok(());
                }
                if !self.confirm("abort-merge", "Abort merge")? {
                    return Ok(());
                }
                g!().merge_abort()?;
                self.set_msg("merge aborted");
                self.refresh();
            }
            // magit-rebase-onto-pushremote
            "rebase-onto-pushremote" => {
                let Some((branch, remote, _)) = self.select_push_remote("rebase onto that")? else {
                    return Ok(());
                };
                let res = self.git.rebase(&format!("{remote}/{branch}"), &args);
                self.after_rebase(res);
            }
            // magit-rebase-onto-upstream
            "rebase-onto-upstream" => {
                let Some(branch) = self.git.current_branch() else {
                    self.set_msg("No branch is checked out");
                    return Ok(());
                };
                let mut upstream = self.upstream_branch(&branch);
                if self.prefix_arg.is_some() || upstream.is_none() {
                    let prompt = format!("Set upstream of {branch} and rebase onto that");
                    let Some(up) = self.read_upstream_branch(&branch, &prompt)? else {
                        return Ok(());
                    };
                    self.git.git_interactive(&["branch", "--set-upstream-to", &up, &branch])?;
                    upstream = Some(up);
                }
                let res = self.git.rebase(&upstream.unwrap_or_default(), &args);
                self.after_rebase(res);
            }
            // magit-rebase-branch
            "rebase-branch" => {
                let Some(target) = self.read_other_branch_or_commit("Rebase onto")? else {
                    return Ok(());
                };
                let res = self.git.rebase(&target, &args);
                self.after_rebase(res);
            }
            // magit-rebase-autosquash: the commits after the merge-base
            // with the upstream, else after one selected from the log
            "rebase-autosquash" => match g!().autosquash_base() {
                Some(base) => self.rebase_autosquash(&base, &args)?,
                None => {
                    let msg = "Type %p on a commit to squash into it and then rebase as necessary,";
                    self.log_select(SelectAction::RebaseAutosquash { args: args.clone() }, Some(msg), None);
                }
            },
            "commit-reshelve" => self.commit_reshelve(&args)?,
            "commit-autofixup" | "commit-autofixup-run" | "commit-absorb-modules" => self.spread_command(cmd, &args)?,
            // magit-reshelve-since: the key of --gpg-sign= of the rebase
            // arguments
            "reshelve-since" => {
                let keyid = args.iter().find_map(|a| a.strip_prefix("--gpg-sign=")).map(str::to_string);
                self.reshelve_since_select(keyid)?;
            }
            // magit-rebase-continue: refuse without a rebase or with unstaged
            // changes (unresolved conflicts are unstaged too)
            "rebase-continue" => {
                if !self.git.rebase_in_progress() {
                    self.set_msg("No rebase in progress");
                    return Ok(());
                }
                if !self.git.git_ok(&["diff", "--quiet", "--ignore-submodules"]) {
                    self.set_msg("Cannot continue rebase with unstaged changes");
                    return Ok(());
                }
                let res = g!().rebase_continue();
                self.rebase_result(res, "rebase continued");
            }
            "rebase-skip" => {
                if !self.git.rebase_in_progress() {
                    self.set_msg("No rebase in progress");
                    return Ok(());
                }
                let res = g!().rebase_skip();
                self.rebase_result(res, "rebase skipped");
            }
            // magit-rebase-abort
            "rebase-abort" => {
                if !self.git.rebase_in_progress() {
                    self.set_msg("No rebase in progress");
                    return Ok(());
                }
                if !self.confirm("abort-rebase", "Abort this rebase")? {
                    return Ok(());
                }
                g!().rebase_abort()?;
                self.set_msg("rebase aborted");
                self.refresh();
            }

            // -------- reset
            "reset-mixed" | "reset-soft" | "reset-hard" | "reset-keep" | "reset-index"
            | "reset-worktree" | "reset-quickly" => {
                // magit-reset-quickly: hard with a prefix argument
                let hard = cmd == "reset-hard" || (cmd == "reset-quickly" && self.prefix_arg.is_some());
                let current = self.git.current_branch().unwrap_or_else(|| "detached head".into());
                let prompt = match cmd {
                    "reset-index" => "Reset index to".to_string(),
                    "reset-worktree" => "Reset worktree to".to_string(),
                    "reset-soft" => format!("Soft reset {current} to"),
                    _ if hard => format!("Hard reset {current} to"),
                    _ => format!("Reset {current} to"),
                };
                let Some(rev) = self.read_branch_or_commit(&prompt)? else {
                    return Ok(());
                };
                if cmd == "reset-worktree" {
                    self.wip_hook(&[], "before", "reset")?;
                    let res = self.git.reset_worktree(&rev);
                    if res.is_ok() {
                        self.wip_hook(&[], "after", "reset")?;
                    }
                    self.done("", res.map(|_| String::new()));
                    return Ok(());
                }
                let arg = match cmd {
                    "reset-index" => None,
                    "reset-soft" => Some("--soft"),
                    "reset-keep" => Some("--keep"),
                    _ if hard => Some("--hard"),
                    _ => Some("--mixed"),
                };
                self.reset_internal(arg, &rev, (cmd == "reset-index").then_some("."))?;
            }

            // -------- tag
            "tag-create" | "tag-release" | "tag-prune" => self.tag_command(cmd, &args)?,
            // magit-tag-delete: the selected tags, else one read
            "tag-delete" => {
                let tags = match self.region_refs(&[RefKind::Tag], false) {
                    Some(tags) => {
                        if !self.confirm_items("t", "Delete %s", "Delete %d tags", &tags)? {
                            return Ok(());
                        }
                        tags
                    }
                    None => {
                        let tags = g!().tags();
                        let default = self.tag_at_point();
                        match self.pick_rev("Delete tag", &tags, default.as_deref())? {
                            Some(t) => vec![t.clone()],
                            None => return Ok(()),
                        }
                    }
                };
                let mut a = vec!["tag", "-d"];
                a.extend(tags.iter().map(String::as_str));
                let res = g!().git_interactive(&a);
                self.done("tag deleted", res.map(|()| String::new()));
            }

            // -------- revert / cherry-pick
            // magit-revert-and-commit / magit-revert-no-commit: the selected
            // commits, else one read (magit-read-branch-or-commit)
            "revert" | "revert-no-commit" => {
                let no_commit = cmd == "revert-no-commit";
                let commits = match self.region_commits() {
                    Some(c) => c,
                    None => {
                        let prompt = if no_commit { "Revert changes" } else { "Revert commit" };
                        let default = self.commit_at_point().map(|c| self.git.rev_short(&c)).or(g!().current_branch());
                        let Some(r) = self.read_rev(prompt, default.as_deref())? else {
                            return Ok(());
                        };
                        vec![r]
                    }
                };
                let mut a = args.clone();
                if no_commit {
                    a.insert(0, "--no-commit".into());
                }
                self.cherry_pick_run(commits, a, true)?;
            }
            "sequencer-continue" | "sequencer-skip" | "sequencer-abort" => {
                let action = cmd.trim_start_matches("sequencer-");
                // magit-sequencer-abort
                if action == "abort" {
                    let (a, p) = match g!().sequencer_command() {
                        Ok("revert") => ("abort-revert", "Really abort revert"),
                        Ok(_) => ("abort-cherry-pick", "Really abort cherry-pick"),
                        Err(_) => {
                            self.set_msg("No cherry-pick or revert in progress");
                            return Ok(());
                        }
                    };
                    if !self.confirm(a, p)? {
                        return Ok(());
                    }
                }
                // magit-sequencer-continue / -skip: only cherry-pick or
                // revert sequences have these suffixes. Merge and rebase
                // have their own commands, so the broader
                // sequencer_in_progress() would let them fall through to a
                // "no cherry-pick or revert in progress" git failure.
                if action == "continue" || action == "skip" {
                    if !self.git.pick_or_revert_in_progress() {
                        self.set_msg("No cherry-pick or revert in progress");
                        return Ok(());
                    }
                    if action == "continue" && !self.git.unmerged_files().is_empty() {
                        self.set_msg("Cannot continue due to unresolved conflicts");
                        return Ok(());
                    }
                }
                match g!().sequencer(action) {
                    Ok(()) => self.set_msg(&format!("sequencer {action}")),
                    Err(e) => self.report_error(&e),
                }
                self.refresh();
            }
            // magit-apply / magit-stash-apply / magit-cherry-apply
            "apply" => self.apply_cmd(&args)?,
            // magit-reverse / magit-revert-no-commit
            "reverse" => self.reverse_cmd()?,
            // magit-cherry-copy / magit-cherry-apply: the selected commits
            // (oldest first), else one read (magit-read-other-branch-or-commit)
            "cherry-pick" | "cherry-apply" => {
                let commits = match self.region_commits() {
                    Some(mut c) => {
                        c.reverse();
                        c
                    }
                    None => {
                        let prompt = if cmd == "cherry-pick" { "Cherry-pick" } else { "Apply changes from commit" };
                        let current = g!().current_branch();
                        let default = self
                            .commit_at_point()
                            .map(|c| self.git.rev_short(&c))
                            .filter(|c| Some(c) != current.as_ref())
                            .or_else(|| self.previous_branch());
                        let Some(r) = self.read_rev(prompt, default.as_deref())? else {
                            return Ok(());
                        };
                        vec![r]
                    }
                };
                let a = if cmd == "cherry-apply" {
                    let mut a = vec!["--no-commit".to_string()];
                    a.extend(args.iter().filter(|x| *x != "--ff").cloned());
                    a
                } else {
                    args.clone()
                };
                self.cherry_pick_run(commits, a, false)?;
            }

            // -------- stash
            "stash-both" | "stash-index" | "stash-worktree" | "stash-keep-index" | "snapshot-both" | "snapshot-index"
            | "snapshot-worktree" | "stash-push" | "stash-apply" | "stash-pop" | "stash-branch" | "stash-branch-here"
            | "stash-format-patch" => self.stash_command(cmd, &args)?,
            "stash-show" => self.diff_view("stash")?,
            "stash-drop" => self.stash_drop()?,
            "stash-list" => self.setup_buffer(BufferMode::Stashes { refname: "refs/stash".into() }, Vec::new()),

            // -------- ignore / misc
            "ignore" | "ignore-subdir" | "ignore-gitdir" | "ignore-system" | "skip-worktree" | "no-skip-worktree"
            | "assume-unchanged" | "no-assume-unchanged" => self.gitignore_command(cmd)?,
            // magit-run (!), magit-ediff (E) and magit-git-mergetool
            "git-command" | "git-command-topdir" | "shell-command" | "shell-command-topdir" | "run-gitk"
            | "run-gitk-all" | "run-gitk-branches" | "run-git-gui" | "git-mergetool" | "git-mergetool-invoke"
            | "ediff-mergetool" | "ediff-resolve-rest" | "ediff-resolve-all" | "ediff-stage" => {
                self.run_menu_command(cmd, &args)?
            }
            // magit-show-refs: the menu in the refs buffer or with a
            // prefix argument, else the refs compared with HEAD
            "show-refs" => {
                if matches!(self.mode, BufferMode::Refs { .. }) || self.prefix_arg.is_some() {
                    self.open_menu("show-refs-menu");
                } else {
                    let args = self.prefix_args("show-refs-menu");
                    self.setup_buffer(BufferMode::Refs { upstream: "HEAD".into() }, args);
                }
            }
            "show-refs-head" => self.setup_buffer(BufferMode::Refs { upstream: "HEAD".into() }, args),
            "show-refs-current" => {
                let upstream = g!().current_branch().unwrap_or_else(|| "HEAD".into());
                self.setup_buffer(BufferMode::Refs { upstream }, args);
            }
            // magit-read-other-branch
            "show-refs-other" => {
                let current = g!().current_branch();
                let mut choices = g!().branches();
                choices.extend(g!().remote_branches());
                choices.retain(|b| Some(b) != current.as_ref());
                let atpoint = self.branch_at_point().filter(|b| Some(b) != current.as_ref());
                let previous = self.previous_branch();
                let default = atpoint.or(previous);
                if let Some(r) = self.pick_rev("Compare with", &choices, default.as_deref())?.cloned() {
                    self.setup_buffer(BufferMode::Refs { upstream: r }, args);
                }
            }
            // magit-refs-set-show-commit-count
            "refs-set-show-commit-count" => {
                if !matches!(self.mode, BufferMode::Refs { .. }) {
                    self.set_msg("Not in a refs buffer");
                    return Ok(());
                }
                let prompt = "Show commit counts for [a]ll refs, [b]ranches only, [n]othing";
                let answer = self.read_line_prompt(prompt, None)?.unwrap_or_default();
                self.refs_counts = match answer.chars().next() {
                    Some('a') => CommitCount::All,
                    Some('b') => CommitCount::Branches,
                    Some('n') => CommitCount::Nothing,
                    _ => {
                        self.set_msg("Quit");
                        return Ok(());
                    }
                };
                self.refresh();
            }
            // magit-delete-thing / magit-file-rename remapped in the
            // sections of the refs buffer
            "rename-thing" => match self.ref_thing() {
                Some(RefThing::Branch) => self.run_command("branch-rename", &BTreeMap::new())?,
                Some(RefThing::Remote) => self.run_command("remote-rename", &BTreeMap::new())?,
                _ => self.file_command("file-rename")?,
            },

            // -------- log / diff / show (arguments come from the l/d menus)
            "log-current" | "log-head" | "log-other" | "log-related" | "log-branches" | "log-all-branches"
            | "log-all" | "log-change" | "wip-log-index" | "wip-log-worktree" | "wip-log" | "wip-log-current" => {
                self.log_view(cmd)?
            }
            // magit-wip-commit: with a prefix argument the message is read
            "wip-commit" => {
                let msg = if self.prefix_arg.is_some() {
                    match self.read_line_prompt("Wip commit message", None)? {
                        Some(m) => m,
                        None => return Ok(()),
                    }
                } else {
                    "wip-save tracked files".to_string()
                };
                match self.options.wip.commit(&self.git, &[], Some(&msg)) {
                    Ok(m) => {
                        self.set_msg(m.as_deref().unwrap_or(""));
                        self.refresh();
                    }
                    Err(e) => self.set_msg(&e.to_string()),
                }
            }
            // magit-log-refresh (L g): new arguments for the log buffer shown
            "log-refresh" => match self.mode {
                BufferMode::Log { .. } | BufferMode::LogSelect { .. } => {
                    self.buf_args = args.clone();
                    self.refresh();
                }
                // the log sections of the status buffer
                BufferMode::Status => {
                    self.status_log_args = args.clone();
                    self.refresh();
                }
                _ => {
                    self.log_args = args.clone();
                    self.set_msg(&format!("log arguments: {}", self.log_args.join(" ")));
                }
            },
            "reflog-current" | "reflog-other" | "reflog-head" => {
                let refname = match cmd {
                    "reflog-head" => "HEAD".to_string(),
                    "reflog-current" => g!().current_branch().unwrap_or_else(|| "HEAD".into()),
                    _ => {
                        let default = match self.current_line().map(|l| l.kind.clone()) {
                            Some(LineKind::Commit { hash, .. }) => Some(self.git.rev_short(&hash)),
                            _ => g!().current_branch(),
                        };
                        let Some(r) = self.read_rev("Show reflog for", default.as_deref())? else {
                            return Ok(());
                        };
                        r
                    }
                };
                self.setup_buffer(BufferMode::Reflog { refname }, vec![format!("-n{REFLOG_LIMIT}")]);
            }
            // magit-shortlog-since / -range: `git shortlog ARGS REV` shown in
            // the *magit-shortlog* buffer
            "shortlog-since" | "shortlog-range" => {
                let rev = if cmd == "shortlog-since" {
                    let default = crate::ui::view::current_tag(g!()).map(|(t, _)| t);
                    let Some(c) = self.read_rev("Shortlog since", default.as_deref())? else {
                        return Ok(());
                    };
                    format!("{c}..")
                } else {
                    let default = match self.dwim() {
                        Dwim::Commit(c) => Some(self.git.rev_short(&c)),
                        Dwim::Range(r) => Some(r),
                        _ => g!().current_branch(),
                    };
                    let Some(r) = self.read_rev("Shortlog for revision or range", default.as_deref())? else {
                        return Ok(());
                    };
                    r
                };
                let mut a: Vec<&str> = vec!["shortlog"];
                a.extend(args.iter().map(String::as_str));
                a.push(&rev);
                match g!().git_string(&a) {
                    Ok(o) => self.view_text = Some(("*magit-shortlog*".into(), o)),
                    Err(e) => self.report_error(&e),
                }
            }
            "diff-unstaged" | "diff-staged" | "diff-worktree" | "diff-dwim" | "diff-range"
            | "show-commit" => {
                let kind = if cmd == "show-commit" { "commit" } else { &cmd[5..] };
                self.diff_view(kind)?;
            }
            // magit-diff-refresh (D g): new arguments for the buffer shown
            "diff-refresh" => {
                match self.mode {
                    BufferMode::Status => self.status_args = args.clone(),
                    _ => self.buf_args = args.clone(),
                }
                self.refresh();
            }
            "process-buffer" => {
                let log = crate::git::exec::process_log();
                let text = if log.is_empty() {
                    "No git commands run yet".to_string()
                } else {
                    log.join("\n")
                };
                self.view_text = Some(("git process log".into(), text));
            }
            "section-up" => self.section_up(),
            "diff-visit-worktree-file" => self.visit(true)?,

            // -------- repo-level
            "init" => {
                // PATH is relative to the directory xagit was started in
                // (`xagit ../test1` → ../test1), NOT the process cwd; the
                // default initializes exactly that directory.
                let base = self.cwd.clone();
                let default = base.to_string_lossy().into_owned();
                let Some(path) = self.read_line_prompt("Initialize repository at", Some(&default))? else {
                    return Ok(());
                };
                let abs = absolute_in(&base, &path);
                // `git init <dir>` creates the directory if needed, so it
                // also works when the target does not exist yet.
                self.run_repo_git(&base, &["init", &abs.to_string_lossy()])?;
                self.open_created_repository(&abs);
                self.set_msg(&format!("initialized {path}"));
            }
            "clone" => {
                let Some(url) = self.read_line_prompt("Repository url", None)? else {
                    return Ok(());
                };
                let base = self.cwd.clone();
                let target = self.read_line_prompt("Target directory", Some("."))?;
                let t = target.unwrap_or_else(|| ".".into());
                let dir = absolute_in(&base, &t);
                self.run_repo_git(&base, &["clone", &url, &dir.to_string_lossy()])?;
                self.open_created_repository(&dir);
                self.set_msg("cloned");
            }

            // All remaining commands are Magit extras
            _ => self.run_extra(cmd, &args)?,
        }
        Ok(())
    }

}
