//! ui/app/refs.rs — reading branches / tags / remotes, push and fetch commands.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

impl App {
    /// magit-commit-extend / -reword with a prefix argument (and
    /// magit-commit-*-override-date t): the committer date of HEAD, kept.
    pub(super) fn keep_committer_date(&self) -> Option<String> {
        self.prefix_arg?;
        self.git.git_string(&["log", "-1", "--format=%cD", "HEAD"]).ok()
    }

    /// magit-branch-rename OLD to NEW (with a prefix argument even if NEW
    /// exists), moving its push-remote setup (magit-branch-rename-push-
    /// target t): branch.OLD.pushRemote becomes branch.NEW.pushRemote and,
    /// if confirmed, the push-target is renamed on the remote.
    pub(super) fn branch_rename(&mut self, old: &str, new: &str) -> Result<()> {
        let old = old.strip_prefix("heads/").unwrap_or(old);
        let force = self.prefix_arg.is_some();
        if old == new {
            anyhow::bail!("Old and new branch names are the same");
        }
        if !force && self.git.git_ok(&["show-ref", "--verify", "--quiet", &format!("refs/heads/{new}")]) {
            anyhow::bail!("Branch `{new}' already exists");
        }
        let config = |k: &str| self.git.git_string(&["config", k]).ok().filter(|v| !v.is_empty());
        let push_remote = config(&format!("branch.{old}.pushRemote"));
        let remote = push_remote.clone().or_else(|| config("remote.pushDefault"));
        self.git.git_interactive(&["branch", if force { "-M" } else { "-m" }, old, new])?;
        if let Some(r) = &push_remote {
            let _ = self.git.git_ok(&["config", "--unset", &format!("branch.{old}.pushRemote")]);
            let _ = self.git.git_ok(&["config", &format!("branch.{new}.pushRemote"), r]);
        }
        if let Some(remote) = remote {
            let (o, n) = (format!("{remote}/{old}"), format!("{remote}/{new}"));
            if self.git.rev_verify(&o)
                && !self.git.rev_verify(&n)
                && self.magit_y_or_n_p(&format!("Also rename \"{old}\" to \"{new}\" on \"{remote}\"?"), None)?
            {
                let src = format!("refs/remotes/{o}:refs/heads/{new}");
                let del = format!(":refs/heads/{old}");
                let res = self.git.git_interactive(&["push", "-v", &remote, &src, &del]);
                self.done("renamed", res.map(|()| String::new()));
                return Ok(());
            }
        }
        self.set_msg("renamed");
        self.refresh();
        Ok(())
    }

    /// What the section at point is in the refs buffer.
    pub(super) fn ref_thing(&self) -> Option<RefThing> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Tag, .. }) => Some(RefThing::Tag),
            Some(LineKind::Ref { .. }) => Some(RefThing::Branch),
            Some(LineKind::Remote { .. }) => Some(RefThing::Remote),
            _ => None,
        }
    }

    /// The branch of the branch section at point (magit-section-value-if
    /// 'branch).
    pub(super) fn branch_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Local | RefKind::Remote, name }) => Some(name.clone()),
            _ => None,
        }
    }

    /// magit-local-branch-at-point: the local branch at point, or at the
    /// commit at point.
    pub(super) fn local_branch_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Local, name }) => Some(name.clone()),
            Some(LineKind::Commit { hash, .. }) => self.name_local_branch(hash),
            _ => None,
        }
    }

    /// magit-read-other-local-branch: a local branch but the current
    /// one, the default the one at point, else the previous branch.
    pub(super) fn read_other_local_branch(&mut self, prompt: &str) -> Result<Option<String>> {
        let current = self.git.current_branch();
        let default = self.local_branch_at_point().filter(|b| Some(b) != current.as_ref()).or_else(|| self.previous_branch());
        let mut candidates = self.git.branches();
        candidates.retain(|b| Some(b) != current.as_ref());
        let table = Completion { candidates, require_match: true, multiple: false };
        Ok(self.completing_read(prompt, default.as_deref(), Hist::Revision, Some(table))?.filter(|b| !b.is_empty()))
    }

    /// magit-read-shelved-branch: a shelved branch (the newest last).
    pub(super) fn read_shelved_branch(&mut self, prompt: &str) -> Result<Option<String>> {
        let mut candidates: Vec<String> = self
            .git
            .git_lines(&["for-each-ref", "--format=%(refname)", "refs/shelved"])
            .into_iter()
            .filter_map(|r| r.strip_prefix("refs/shelved/").map(str::to_string))
            .collect();
        candidates.reverse();
        let table = Completion { candidates, require_match: true, multiple: false };
        Ok(self.completing_read(prompt, None, Hist::Minibuffer, Some(table))?.filter(|b| !b.is_empty()))
    }

    /// magit--rename-reflog-file OLD NEW (full refnames).
    pub(super) fn rename_reflog_file(&self, old: &str, new: &str) {
        let old = self.git.gitdir.join("logs").join(old);
        let new = self.git.gitdir.join("logs").join(new);
        if old.exists() {
            if let Some(dir) = new.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::rename(old, new);
        }
    }

    /// magit-branch-shelve: refs/heads/BRANCH becomes
    /// refs/shelved/YYYY-MM-DD-BRANCH (its committer date), with its
    /// reflog; its push-remote is unset.
    pub(super) fn branch_shelve(&mut self) -> Result<()> {
        let Some(branch) = self.read_other_local_branch("Shelve branch")? else {
            return Ok(());
        };
        let old = format!("refs/heads/{branch}");
        let date = self.git.git_string(&["log", "-1", "--format=%cs", &old]).unwrap_or_default();
        let new = format!("refs/shelved/{}-{branch}", date.trim());
        if let Err(e) = self.git.git_interactive(&["update-ref", &new, &old, ""]) {
            self.set_msg(&format!("{e:#}"));
            return Ok(());
        }
        self.rename_reflog_file(&old, &new);
        let _ = self.git.git_interactive(&["config", "--unset", &format!("branch.{branch}.pushRemote")]);
        let res = self.git.git_interactive(&["branch", "-D", &branch]);
        self.done_keep_msg(res);
        Ok(())
    }

    /// magit-branch-unshelve: refs/shelved/BRANCH becomes
    /// refs/heads/BRANCH (a leading YYYY-MM-DD- dropped), with its
    /// reflog.
    pub(super) fn branch_unshelve(&mut self) -> Result<()> {
        let Some(branch) = self.read_shelved_branch("Unshelve branch")? else {
            return Ok(());
        };
        let old = format!("refs/shelved/{branch}");
        let b = branch.as_bytes();
        let dated = b.len() > 11
            && b[..10].iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
            && b[10] == b'-';
        let new = format!("refs/heads/{}", if dated { &branch[11..] } else { &branch[..] });
        if let Err(e) = self.git.git_interactive(&["update-ref", &new, &old, ""]) {
            self.set_msg(&format!("{e:#}"));
            return Ok(());
        }
        self.rename_reflog_file(&old, &new);
        let res = self.git.git_interactive(&["update-ref", "-d", &old]);
        self.done_keep_msg(res);
        Ok(())
    }

    /// magit-tag-at-point
    pub(super) fn tag_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Tag, name }) => Some(name.clone()),
            _ => None,
        }
    }

    /// The secret OpenPGP keys that can sign (`gpg --list-secret-keys`):
    /// the fingerprints of their primary keys.
    pub(super) fn gpg_signing_keys(&self) -> Vec<String> {
        let Ok(out) = std::process::Command::new("gpg").args(["--list-secret-keys", "--with-colons"]).output() else {
            return Vec::new();
        };
        let mut keys: Vec<(String, bool)> = Vec::new();
        let mut want_fpr = false;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let f: Vec<&str> = line.split(':').collect();
            match f[0] {
                "sec" => {
                    keys.push((String::new(), false));
                    want_fpr = true;
                }
                "ssb" => want_fpr = false,
                "fpr" if want_fpr => {
                    if let Some(k) = keys.last_mut() {
                        k.0 = f.get(9).unwrap_or(&"").to_string();
                    }
                    want_fpr = false;
                }
                _ => {}
            }
            if matches!(f[0], "sec" | "ssb") && f.get(11).is_some_and(|c| c.contains('s')) {
                if let Some(k) = keys.last_mut() {
                    k.1 = true;
                }
            }
        }
        keys.into_iter().filter(|(f, sign)| *sign && !f.is_empty()).map(|(f, _)| f).collect()
    }

    /// magit-read-gpg-signing-key: "" unless gpg.format is openpgp; the
    /// only key, or magit-openpgp-default-signing-key, without asking
    /// unless with a prefix argument; else one read (any input).
    pub(super) fn read_gpg_signing_key(&mut self, prompt: &str) -> Result<Option<String>> {
        if !matches!(self.config("gpg.format").as_deref(), None | Some("openpgp")) {
            return Ok(Some(String::new()));
        }
        let keys = self.gpg_signing_keys();
        if self.prefix_arg.is_none() {
            if keys.len() == 1 {
                return Ok(keys.into_iter().next());
            }
            if let Some(d) = self.options.openpgp_default_signing_key.clone().filter(|d| keys.contains(d)) {
                return Ok(Some(d));
            }
        }
        let table = Completion { candidates: keys, require_match: false, multiple: false };
        Ok(self.read_line_hist(prompt, None, Hist::Infix("--gpg-sign="), Some(table))?.map(|s| s.trim().to_string()))
    }

    /// magit-remote-at-point: the remote section at point, or the remote
    /// of the remote branch section at point.
    pub(super) fn remote_at_point(&self) -> Option<String> {
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Remote { name }) => Some(name.clone()),
            Some(LineKind::Ref { kind: RefKind::Remote, .. }) => self.sec(self.cur_sec()).parent.and_then(|parent| {
                match &self.buf.all[self.sec(parent).start].kind {
                    LineKind::Remote { name } => Some(name.clone()),
                    _ => None,
                }
            }),
            _ => None,
        }
    }

    /// The value of the git variable VAR, if set and not empty.
    pub(super) fn config(&self, var: &str) -> Option<String> {
        self.git.git_string(&["config", var]).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }

    /// magit-get-remote: the remote of the current branch, unless ".".
    pub(super) fn get_remote(&self) -> Option<String> {
        let branch = self.git.current_branch()?;
        self.config(&format!("branch.{branch}.remote")).filter(|r| r != ".")
    }

    /// magit-get-current-remote with ALLOW-UNNAMED: the remote of the
    /// upstream (an url too), else the only remote, else
    /// magit-primary-remote.
    pub(super) fn current_remote(&self) -> Option<String> {
        let remotes = self.git.remotes();
        let upstream = self.git.current_branch().and_then(|b| self.config(&format!("branch.{b}.remote"))).filter(|r| {
            r != "." && (remotes.contains(r) || unnamed_remote(r))
        });
        upstream.or_else(|| if remotes.len() == 1 { remotes.first().cloned() } else { self.git.primary_remote() })
    }

    /// magit-get-push-remote of BRANCH: branch.BRANCH.pushRemote, else
    /// remote.pushDefault.
    pub(super) fn get_push_remote(&self, branch: Option<&str>) -> Option<String> {
        branch.and_then(|b| self.config(&format!("branch.{b}.pushRemote"))).or_else(|| self.config("remote.pushDefault"))
    }

    /// magit--push-remote-variable: branch.BRANCH.pushRemote, without a
    /// branch remote.pushDefault (SHORT: pushRemote, pushDefault).
    pub(super) fn push_remote_variable(branch: Option<&str>, short: bool) -> String {
        match (branch, short) {
            (Some(_), true) => "pushRemote".into(),
            (Some(b), false) => format!("branch.{b}.pushRemote"),
            (None, true) => "pushDefault".into(),
            (None, false) => "remote.pushDefault".into(),
        }
    }

    /// magit-read-remote: one of the remotes, the default the remote at
    /// point, else that of the current branch.
    pub(super) fn read_remote(&mut self, prompt: &str) -> Result<Option<String>> {
        let default = self.remote_at_point().or_else(|| self.get_remote());
        let table = Completion { candidates: self.git.remotes(), require_match: true, multiple: false };
        Ok(self.completing_read(prompt, default.as_deref(), Hist::Minibuffer, Some(table))?.filter(|r| !r.is_empty()))
    }

    /// magit-read-remote-or-url: a remote or any url (the remotes and
    /// "https://", "git://", "git@" completed).
    pub(super) fn read_remote_or_url(&mut self, prompt: &str) -> Result<Option<String>> {
        let default = self.remote_at_point().or_else(|| self.get_remote());
        let mut candidates = self.git.remotes();
        candidates.extend(["https://", "git://", "git@"].map(String::from));
        let table = Completion { candidates, ..Default::default() };
        self.read_any(prompt, default.as_deref(), Hist::Minibuffer, table)
    }

    /// magit-read-refspec: one of the refs of magit-remote-list-refs of
    /// REMOTE (any input).
    pub(super) fn read_refspec(&mut self, prompt: &str, remote: &str) -> Result<Option<String>> {
        self.set_msg("Determining available refs...");
        let candidates = self
            .git
            .git_lines(&["ls-remote", remote])
            .into_iter()
            .filter_map(|l| l.get(41..).map(str::to_string))
            .filter(|r| !r.ends_with("^{}"))
            .collect();
        self.set_msg("Determining available refs...done");
        let table = Completion { candidates, ..Default::default() };
        self.read_any(prompt, None, Hist::Minibuffer, table)
    }

    /// magit-remote-branch-at-point: the remote branch at point, or one
    /// at the commit at point.
    pub(super) fn remote_branch_at_point(&self) -> Option<String> {
        let remote = self.git.remote_branches();
        match self.current_line().map(|l| &l.kind) {
            Some(LineKind::Ref { kind: RefKind::Remote, name }) => remote.contains(name).then(|| name.clone()),
            Some(LineKind::Commit { hash, .. }) => self
                .git
                .git_lines(&["for-each-ref", "--points-at", hash, "--format=%(refname:short)", "refs/remotes"])
                .into_iter()
                .find(|r| remote.contains(r)),
            _ => None,
        }
    }

    /// magit-read-upstream-branch: an existing branch but BRANCH, the
    /// default the branch at point (the local one first), else the main
    /// branch (or origin/MAIN), else the previous branch.
    pub(super) fn read_upstream_branch(&mut self, branch: &str, prompt: &str) -> Result<Option<String>> {
        let mut candidates = self.git.branches();
        candidates.extend(self.git.remote_branches());
        candidates.retain(|b| b != branch);
        let member = |b: Option<String>| b.filter(|b| candidates.contains(b));
        let default = member(self.local_branch_at_point())
            .or_else(|| member(self.remote_branch_at_point()))
            .or_else(|| {
                let main = self.git.main_branch()?;
                member(Some(main.clone())).or_else(|| member(Some(format!("origin/{main}"))))
            })
            .or_else(|| member(self.previous_branch()));
        let table = Completion { candidates, require_match: true, multiple: false };
        Ok(self.completing_read(prompt, default.as_deref(), Hist::Revision, Some(table))?.filter(|b| !b.is_empty()))
    }

    /// magit--select-push-remote: the current branch and its push-remote,
    /// which is read (and set) with a prefix argument or when it isn't
    /// an existing remote ("Set VAR and PROMPT-SUFFIX").
    /// CHANGED: whether it was read.
    pub(super) fn select_push_remote(&mut self, suffix: &str) -> Result<Option<(String, String, bool)>> {
        let Some(branch) = self.git.current_branch() else {
            self.set_msg("No branch is checked out");
            return Ok(None);
        };
        let remote = self.get_push_remote(Some(&branch)).filter(|r| self.git.remotes().contains(r));
        let (remote, changed) = match remote {
            Some(r) if self.prefix_arg.is_none() => (r, false),
            _ => {
                let var = Self::push_remote_variable(Some(&branch), false);
                let Some(r) = self.read_remote(&format!("Set {var} and {suffix}"))? else {
                    return Ok(None);
                };
                self.git.git_interactive(&["config", &var, &r])?;
                (r, true)
            }
        };
        Ok(Some((branch, remote, changed)))
    }

    /// magit-pull--upstreams: the upstream of the current branch if it is
    /// a local branch, and its upstream if that is a remote branch.
    pub(super) fn pull_upstreams(&self) -> Option<(String, String)> {
        let up1 = self.upstream_branch(&self.git.current_branch()?)?;
        let is = |prefix: &str, b: &str| self.git.git_ok(&["show-ref", "--verify", "--quiet", &format!("{prefix}{b}")]);
        if !is("refs/heads/", &up1) {
            return None;
        }
        let up2 = self.upstream_branch(&up1)?;
        is("refs/remotes/", &up2).then_some((up1, up2))
    }

    /// magit-pull--pushbranch-description of BRANCH.
    pub(super) fn pull_pushbranch_description(&self, b: &str, remotes: &[String]) -> String {
        let v = Self::push_remote_variable(Some(b), true);
        match self.get_push_remote(Some(b)) {
            Some(r) if self.git.rev_verify(&format!("{r}/{b}")) => format!("{r}/{b}"),
            Some(r) if remotes.contains(&r) => format!("{v}, replacing non-existent"),
            Some(_) => format!("{v}, replacing invalid"),
            None => format!("{v}, setting that"),
        }
    }

    /// The descriptions and `:if` of the suffixes of the menu NAME that
    /// depend on the repository, the git variable infixes included.
    pub(super) fn menu_states(&self, name: &str) -> Vec<(String, String, bool)> {
        let mut states: Vec<(String, String, bool)> =
            self.menu_suffix_states(name).into_iter().map(|(k, d, s)| (k.to_string(), d, s)).collect();
        states.extend(self.var_suffix_states(name));
        states
    }

    /// Update the descriptions of the menu NAME shown (transient
    /// :refresh-suffixes after an infix set a git variable).
    pub(super) fn refresh_menu_states(&mut self, name: &str) {
        let states = self.menu_states(name);
        if let Some(m) = self.menus.get_mut(name) {
            for e in m.groups.iter_mut().flatten() {
                if let Some((_, desc, _)) = states.iter().find(|(k, _, _)| *k == e.key) {
                    e.desc.clone_from(desc);
                }
            }
        }
    }

    /// The descriptions and `:if` of the suffixes of NAME that depend on
    /// the repository: (key, description, shown).
    pub(super) fn menu_suffix_states(&self, name: &str) -> Vec<(&'static str, String, bool)> {
        let branch = self.git.current_branch();
        let remotes = self.git.remotes();
        match name {
            // magit-gitignore-on-system's description
            "gitignore-menu" => {
                let file = self.config("core.excludesfile");
                let file = file.as_deref().unwrap_or("core.excludesfile is not set");
                vec![("g", format!("privately for all repositories ({file})"), true)]
            }
            "fetch-menu" => {
                // magit-fetch--pushremote-description
                let remote = self.get_push_remote(branch.as_deref());
                let v = Self::push_remote_variable(branch.as_deref(), true);
                let p = match remote {
                    Some(r) if remotes.contains(&r) => r,
                    Some(_) => format!("{v}, replacing invalid"),
                    None => format!("{v}, setting that"),
                };
                let u = self.current_remote();
                vec![("p", p, true), ("u", u.clone().unwrap_or_default(), u.is_some())]
            }
            "pull-menu" => {
                let Some(b) = branch else {
                    return ["p", "u", "U"].map(|k| (k, String::new(), false)).into();
                };
                let p = self.pull_pushbranch_description(&b, &remotes);
                // magit-pull--upstream-description
                let remote = self.config(&format!("branch.{b}.remote"));
                let merge = self.config(&format!("branch.{b}.merge"));
                let u = match self.upstream_branch(&b) {
                    Some(up) => up,
                    None if unnamed_upstream(remote.as_deref(), merge.as_deref()) => {
                        format!("{} of {}", merge.unwrap_or_default(), remote.unwrap_or_default())
                    }
                    None if valid_upstream(remote.as_deref(), merge.as_deref(), &remotes) => {
                        "@{upstream}, replacing non-existent".into()
                    }
                    None if remote.is_some() || merge.is_some() => "@{upstream}, replacing invalid".into(),
                    None => "@{upstream}, setting that".into(),
                };
                let ups = self.pull_upstreams();
                vec![
                    ("p", p, true),
                    ("u", u, true),
                    ("U", ups.clone().map(|(_, u)| u).unwrap_or_default(), ups.is_some()),
                ]
            }
            "rebase-menu" => {
                let Some(b) = branch else {
                    return ["p", "u"].map(|k| (k, String::new(), false)).into();
                };
                let p = self.pull_pushbranch_description(&b, &remotes);
                // magit-rebase--upstream-description
                let remote = self.config(&format!("branch.{b}.remote"));
                let merge = self.config(&format!("branch.{b}.merge"));
                let u = match self.upstream_branch(&b) {
                    Some(up) => up,
                    None if unnamed_upstream(remote.as_deref(), merge.as_deref()) => "@{upstream}, replacing unnamed".into(),
                    None if valid_upstream(remote.as_deref(), merge.as_deref(), &remotes) => {
                        "@{upstream}, replacing non-existent".into()
                    }
                    None if remote.is_some() || merge.is_some() => "@{upstream}, replacing invalid".into(),
                    None => "@{upstream}, setting that".into(),
                };
                vec![("p", p, true), ("u", u, true)]
            }
            "push-menu" => {
                let Some(b) = branch else {
                    return ["p", "u", "e"].map(|k| (k, String::new(), false)).into();
                };
                // magit-push--pushbranch-description
                let remote = self.get_push_remote(Some(&b));
                let v = Self::push_remote_variable(Some(&b), true);
                let p = match remote {
                    Some(r) if self.git.rev_verify(&format!("{r}/{b}")) => format!("{r}/{b}"),
                    Some(r) if remotes.contains(&r) => format!("{r}/{b}, creating it"),
                    Some(_) => format!("{v}, replacing invalid"),
                    None => format!("{v}, setting that"),
                };
                // magit-push--upstream-description
                let remote = self.config(&format!("branch.{b}.remote"));
                let merge = self.config(&format!("branch.{b}.merge"));
                let u = match self.upstream_branch(&b) {
                    Some(up) => up,
                    None if unnamed_upstream(remote.as_deref(), merge.as_deref()) => {
                        format!("{} as {}", remote.unwrap_or_default(), merge.unwrap_or_default())
                    }
                    None if valid_upstream(remote.as_deref(), merge.as_deref(), &remotes) => {
                        format!("{} creating {}", remote.unwrap_or_default(), merge.unwrap_or_default())
                    }
                    None if remote.is_some() || merge.is_some() => "@{upstream}, creating it and replacing invalid".into(),
                    None => "@{upstream}, creating it".into(),
                };
                vec![("p", p, true), ("u", u, true), ("e", "elsewhere".into(), true)]
            }
            _ => Vec::new(),
        }
    }

    /// magit-get-tracked: the remote and the ref on it of the
    /// remote-tracking branch REFNAME (per the remote.*.fetch refspecs).
    pub(super) fn get_tracked(&self, refname: &str) -> Option<(String, String)> {
        let full = self
            .git
            .git_string(&["rev-parse", "--verify", "-q", "--symbolic-full-name", refname])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        self.git.git_lines(&["config", "--local", "--list"]).iter().find_map(|line| {
            let rest = line.strip_prefix("remote.")?;
            let (remote, rest) = rest.split_once(".fetch=")?;
            if remote.contains('.') {
                return None;
            }
            let (src, dst) = rest.trim_start_matches('+').split_once(':')?;
            let star = match dst.split_once('*') {
                Some((pre, post)) => full.strip_prefix(pre)?.strip_suffix(post).filter(|m| !m.is_empty())?,
                None if dst == full => "",
                None => return None,
            };
            Some((remote.to_string(), src.replacen('*', star, 1)))
        })
    }

    /// magit-split-branch-name: (".", BRANCH) for a local branch, else
    /// the remote and the branch on it.
    pub(super) fn split_branch_name(&self, branch: &str) -> std::result::Result<(String, String), String> {
        if self.git.branches().iter().any(|b| b == branch) {
            return Ok((".".into(), branch.into()));
        }
        self.git
            .remotes()
            .into_iter()
            .find_map(|r| {
                let b = branch.strip_prefix(&format!("{r}/")).filter(|b| !b.is_empty())?.to_string();
                Some((r, b))
            })
            .ok_or_else(|| format!("Invalid branch name {branch}"))
    }

    /// magit-read-remote with USE-ONLY: the only remote without asking.
    pub(super) fn read_remote_use_only(&mut self, prompt: &str) -> Result<Option<String>> {
        let remotes = self.git.remotes();
        if remotes.len() == 1 {
            return Ok(remotes.into_iter().next());
        }
        self.read_remote(prompt)
    }

    /// magit-read-remote-branch without REMOTE, with LOCAL-BRANCH and
    /// REQUIRE-MATCH confirm: REMOTE/LOCAL-BRANCH for every remote and
    /// the remote branches offered; the choice must have the form
    /// REMOTE/BRANCH.
    pub(super) fn read_remote_branch(&mut self, prompt: &str, default: Option<&str>, local: &str) -> Result<Option<String>> {
        let mut candidates: Vec<String> = self.git.remotes().iter().map(|r| format!("{r}/{local}")).collect();
        for b in self.git.remote_branches() {
            if !candidates.contains(&b) {
                candidates.push(b);
            }
        }
        let table = Completion { candidates, ..Default::default() };
        let Some(choice) = self.read_any(prompt, default, Hist::Revision, table)? else {
            return Ok(None);
        };
        if !choice.split_once('/').is_some_and(|(r, b)| !r.is_empty() && !b.is_empty()) {
            self.set_msg(&format!("`{choice}' doesn't have the form REMOTE/BRANCH"));
            return Ok(None);
        }
        Ok(Some(choice))
    }

    /// magit-read-local-branch-or-commit: a local branch, a special ref
    /// (HEAD, ORIG_HEAD, ... that exist) or the commit at point (the
    /// default after the local branch at point).
    pub(super) fn read_local_branch_or_commit(&mut self, prompt: &str) -> Result<Option<String>> {
        let commit = self.commit_at_point();
        let mut candidates: Vec<String> = commit.iter().cloned().collect();
        candidates.extend(self.git.branches());
        candidates.extend(
            ["HEAD", "ORIG_HEAD", "FETCH_HEAD", "MERGE_HEAD", "CHERRY_PICK_HEAD"]
                .iter()
                .filter(|r| self.git.gitdir.join(r).exists())
                .map(|r| r.to_string()),
        );
        let default = self.local_branch_at_point().or(commit);
        let table = Completion { candidates, ..Default::default() };
        self.read_any(prompt, default.as_deref(), Hist::Revision, table)
    }

    /// magit-read-tag: a tag, the one at point the default.
    pub(super) fn read_tag(&mut self, prompt: &str) -> Result<Option<String>> {
        let default = self.tag_at_point();
        let table = Completion { candidates: self.git.tags(), require_match: true, multiple: false };
        Ok(self.completing_read(prompt, default.as_deref(), Hist::Revision, Some(table))?.filter(|t| !t.is_empty()))
    }

    /// magit-git-push: push BRANCH to the remote branch TARGET
    /// ("REMOTE/NAME"), qualified with refs/heads/ unless it exists.
    pub(super) fn git_push(&mut self, branch: &str, target: &str, args: &[String]) {
        let namespace = if self.get_tracked(target).is_some() { "" } else { "refs/heads/" };
        let (remote, name) = match self.split_branch_name(target) {
            Ok(x) => x,
            Err(e) => {
                self.set_msg(&e);
                return;
            }
        };
        let mut a: Vec<String> = vec!["push".into(), "-v".into()];
        a.extend(args.iter().cloned());
        a.extend([remote, format!("{branch}:{namespace}{name}")]);
        self.run_remote_git(&a, &format!("pushed {branch} to {target}"));
    }

    /// The suffixes of magit-push.
    pub(super) fn push_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let push = |head: &[&str], tail: &[String]| -> Vec<String> {
            head.iter().map(|s| s.to_string()).chain(tail.iter().cloned()).collect()
        };
        match cmd {
            // magit-push-current-to-pushremote
            "push-current-to-pushremote" => {
                let Some((branch, remote, changed)) = self.select_push_remote("push there")? else {
                    return Ok(());
                };
                let prompt = format!("Really use \"{remote}\" as push-remote and push \"{branch}\" there");
                if changed && !self.confirm("set-and-push", &prompt)? {
                    return Ok(());
                }
                let mut a = push(&["push", "-v"], args);
                a.extend([remote.clone(), format!("refs/heads/{branch}:refs/heads/{branch}")]);
                self.run_remote_git(&a, &format!("pushed {branch} to {remote}"));
            }
            // magit-push-current-to-upstream
            "push-current-to-upstream" => self.push_current_to_upstream(args)?,
            // magit-push-current
            "push-current" => {
                let Some(branch) = self.git.current_branch() else {
                    self.set_msg("No branch is checked out");
                    return Ok(());
                };
                let Some(target) = self.read_remote_branch(&format!("Push {branch} to"), None, &branch)? else {
                    return Ok(());
                };
                self.git_push(&branch, &target, args);
            }
            // magit-push-other
            "push-other" => {
                let Some(source) = self.read_local_branch_or_commit("Push")? else {
                    return Ok(());
                };
                let push_branch = |s: &Self, b: &str| s.get_push_remote(Some(b)).map(|r| format!("{r}/{b}"));
                let default = if self.git.branches().contains(&source) {
                    push_branch(self, &source).or_else(|| self.upstream_branch(&source))
                } else if self.git.is_ancestor(&source, "HEAD") {
                    self.git.current_branch().and_then(|c| push_branch(self, &c).or_else(|| self.upstream_branch(&c)))
                } else {
                    None
                };
                let prompt = format!("Push {source} to");
                let Some(target) = self.read_remote_branch(&prompt, default.as_deref(), &source)? else {
                    return Ok(());
                };
                self.git_push(&source, &target, args);
            }
            // magit-push-refspecs
            "push-refspecs" => {
                let Some(remote) = self.read_remote("Push to remote")? else {
                    return Ok(());
                };
                let mut candidates = vec!["HEAD".to_string()];
                candidates.extend(self.git.branches());
                let table = Completion { candidates, require_match: false, multiple: true };
                let Some(input) = self.completing_read("Push refspec,s", None, Hist::Minibuffer, Some(table))? else {
                    return Ok(());
                };
                let mut a = push(&["push", "-v"], args);
                a.push(remote.clone());
                a.extend(input.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
                self.run_remote_git(&a, &format!("pushed to {remote}"));
            }
            // magit-push-matching
            "push-matching" => {
                let Some(remote) = self.read_remote_use_only("Push matching branches to")? else {
                    return Ok(());
                };
                let mut a = push(&["push", "-v"], args);
                a.extend([remote.clone(), ":".into()]);
                self.run_remote_git(&a, &format!("pushed matching branches to {remote}"));
            }
            // magit-push-tags: git push REMOTE --tags ARGS
            "push-tags" => {
                let Some(remote) = self.read_remote_use_only("Push tags to remote")? else {
                    return Ok(());
                };
                let a = push(&["push", &remote, "--tags"], args);
                self.run_remote_git(&a, &format!("pushed tags to {remote}"));
            }
            // magit-push-tag / magit-push-notes-ref: git push REMOTE REF ARGS
            "push-tag" | "push-notes-ref" => {
                let r = if cmd == "push-tag" { self.read_tag("Push tag")? } else { self.notes_read_ref("Push notes", "push-notes-ref")? };
                let Some(r) = r else {
                    return Ok(());
                };
                let Some(remote) = self.read_remote_use_only(&format!("Push {r} to remote"))? else {
                    return Ok(());
                };
                let a = push(&["push", &remote, &r], args);
                self.run_remote_git(&a, &format!("pushed {r} to {remote}"));
            }
            _ => {}
        }
        Ok(())
    }

    /// magit-push-current-to-upstream: with a prefix argument, or when
    /// the upstream isn't usable, it is read ("Set upstream of B and push
    /// there", any REMOTE/BRANCH) and set by pushing with --set-upstream.
    pub(super) fn push_current_to_upstream(&mut self, args: &[String]) -> Result<()> {
        let Some(branch) = self.git.current_branch() else {
            self.set_msg("No branch is checked out");
            return Ok(());
        };
        let mut remote = self.config(&format!("branch.{branch}.remote"));
        let mut merge = self.config(&format!("branch.{branch}.merge"));
        let mut args = args.to_vec();
        let remotes = self.git.remotes();
        let usable = self.upstream_branch(&branch).is_some()
            || unnamed_upstream(remote.as_deref(), merge.as_deref())
            || valid_upstream(remote.as_deref(), merge.as_deref(), &remotes);
        if self.prefix_arg.is_some() || !usable {
            let mut candidates: Vec<String> = remotes.iter().map(|r| format!("{r}/{branch}")).collect();
            for b in self.git.remote_branches() {
                if !candidates.contains(&b) {
                    candidates.push(b);
                }
            }
            let default = self
                .remote_branch_at_point()
                .or_else(|| self.git.primary_remote().and_then(|r| self.git.remote_main_branch(&r)));
            let table = Completion { candidates, ..Default::default() };
            let prompt = format!("Set upstream of {branch} and push there");
            let Some(upstream) = self.read_any(&prompt, default.as_deref(), Hist::Revision, table)? else {
                return Ok(());
            };
            let (r, m) = match self.get_tracked(&upstream).map_or_else(|| self.split_branch_name(&upstream), Ok) {
                Ok(x) => x,
                Err(e) => {
                    self.set_msg(&e);
                    return Ok(());
                }
            };
            let m = if m.starts_with("refs/") { m } else { format!("refs/heads/{m}") };
            let prompt = format!("Really use \"{upstream}\" as upstream and push \"{branch}\" there");
            if !self.confirm("set-and-push", &prompt)? {
                return Ok(());
            }
            (remote, merge) = (Some(r), Some(m));
            if !args.iter().any(|a| a == "--set-upstream") {
                args.insert(0, "--set-upstream".into());
            }
        }
        let mut a: Vec<String> = vec!["push".into(), "-v".into()];
        a.extend(args);
        a.extend(remote.clone());
        a.push(format!("{branch}:{}", merge.unwrap_or_default()));
        self.run_remote_git(&a, &format!("pushed {branch} to {}", remote.unwrap_or_default()));
        Ok(())
    }

    /// Run the git command A (fetching, pulling, pushing) in the
    /// background, then refresh.
    pub(super) fn run_remote_git(&mut self, a: &[String], ok: &str) {
        self.run_git_async(a, ok);
    }

    /// The suffixes of magit-fetch and magit-pull.
    pub(super) fn fetch_pull_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let v = |head: &[&str], tail: &[String]| -> Vec<String> {
            head.iter().map(|s| s.to_string()).chain(tail.iter().cloned()).collect()
        };
        match cmd {
            // magit-fetch-from-pushremote
            "fetch-from-pushremote" => {
                let branch = self.git.current_branch();
                let mut remote = self.get_push_remote(branch.as_deref());
                if self.prefix_arg.is_some() || !remote.as_ref().is_some_and(|r| self.git.remotes().contains(r)) {
                    let var = Self::push_remote_variable(branch.as_deref(), false);
                    let Some(r) = self.read_remote(&format!("Set {var} and fetch from there"))? else {
                        return Ok(());
                    };
                    self.git.git_interactive(&["config", &var, &r])?;
                    remote = Some(r);
                }
                let r = remote.unwrap_or_default();
                self.run_remote_git(&v(&["fetch", &r], args), &format!("fetched {r}"));
            }
            // magit-fetch-from-upstream
            "fetch-from-upstream" => {
                let Some(r) = self.current_remote() else {
                    self.set_msg("The \"current\" remote could not be determined");
                    return Ok(());
                };
                self.run_remote_git(&v(&["fetch", &r], args), &format!("fetched {r}"));
            }
            "fetch-other" => {
                let Some(r) = self.read_remote("Fetch remote")? else {
                    return Ok(());
                };
                self.run_remote_git(&v(&["fetch", &r], args), &format!("fetched {r}"));
            }
            // magit-fetch-branch / magit-fetch-refspec
            "fetch-branch" | "fetch-refspec" => {
                let Some(r) = self.read_remote_or_url("Fetch from remote or url")? else {
                    return Ok(());
                };
                let what = if cmd == "fetch-branch" {
                    // magit-read-remote-branch with REMOTE: its branches
                    let prefix = format!("{r}/");
                    let candidates =
                        self.git.remote_branches().iter().filter_map(|b| b.strip_prefix(&prefix)).map(str::to_string).collect();
                    let table = Completion { candidates, ..Default::default() };
                    self.read_any("Fetch branch", None, Hist::Revision, table)?
                } else {
                    self.read_refspec("Fetch using refspec", &r)?
                };
                let Some(what) = what else {
                    return Ok(());
                };
                self.run_remote_git(&v(&["fetch", &r, &what], args), &format!("fetched {r} {what}"));
            }
            // magit-fetch-all
            "fetch-all" => self.run_remote_git(&v(&["fetch", "--all"], args), "fetched all remotes"),
            // magit-fetch-modules: with a prefix argument its transient,
            // else with its value
            "fetch-modules" | "fetch-modules-run" => {
                if cmd == "fetch-modules" && self.prefix_arg.is_some() {
                    self.open_menu("fetch-modules-menu");
                    return Ok(());
                }
                let args = if cmd == "fetch-modules" { self.prefix_args("fetch-modules-menu") } else { args.to_vec() };
                let res = self.git.fetch_modules(&args);
                if self.git.remotes().is_empty() {
                    self.set_msg("Cannot fetch modules if super-repository has no remotes");
                    self.refresh();
                } else {
                    self.done("fetched modules", res);
                }
            }
            // magit-pull-from-pushremote
            "pull-from-pushremote" => {
                let Some((branch, remote, _)) = self.select_push_remote("pull from there")? else {
                    return Ok(());
                };
                let mut a = v(&["pull"], args);
                a.extend([remote.clone(), branch.clone()]);
                self.run_remote_git(&a, &format!("pulled {remote}/{branch}"));
            }
            // magit-pull-from-upstream
            "pull-from-upstream" => {
                let Some(branch) = self.git.current_branch() else {
                    self.set_msg("No branch is checked out");
                    return Ok(());
                };
                let get = |s: &Self, k: &str| s.config(&format!("branch.{branch}.{k}"));
                let (mut remote, mut merge) = (get(self, "remote"), get(self, "merge"));
                if self.prefix_arg.is_some()
                    || !(self.upstream_branch(&branch).is_some() || unnamed_upstream(remote.as_deref(), merge.as_deref()))
                {
                    let prompt = format!("Set upstream of {branch} and pull from there");
                    let Some(up) = self.read_upstream_branch(&branch, &prompt)? else {
                        return Ok(());
                    };
                    self.git.git_interactive(&["branch", "--set-upstream-to", &up, &branch])?;
                    remote = get(self, "remote");
                    merge = get(self, "merge");
                }
                let mut a = v(&["pull"], args);
                a.extend(remote.into_iter().chain(merge));
                self.run_remote_git(&a, "pulled");
            }
            // magit-pull-into-upstream: fetch the upstream of the
            // upstream, then update the upstream to it
            "pull-into-upstream" => {
                let Some((up1, up2)) = self.pull_upstreams() else {
                    self.set_msg("Cannot perform background update of upstream branch");
                    return Ok(());
                };
                let (remote, branch) = self.git.tracked_of(&up2);
                let res = self
                    .git
                    .git_interactive(&["fetch", &remote, &branch])
                    .and_then(|()| self.git.git_interactive(&["update-ref", &format!("refs/heads/{up1}"), "FETCH_HEAD"]));
                match res {
                    Ok(()) => self.set_msg(&format!("updated {up1} from {up2}")),
                    Err(e) => self.report_error(&e),
                }
                self.refresh();
            }
            // magit-pull-branch: a remote branch
            "pull-branch" => {
                let table = Completion { candidates: self.git.remote_branches(), require_match: true, multiple: false };
                let Some(source) = self.completing_read("Pull", None, Hist::Revision, Some(table))? else {
                    return Ok(());
                };
                let Some((remote, branch)) = source.split_once('/').filter(|(r, b)| !r.is_empty() && !b.is_empty()) else {
                    self.set_msg(&format!("`{source}' doesn't have the form REMOTE/BRANCH"));
                    return Ok(());
                };
                let (remote, branch) = match self.git.tracked_of(&source) {
                    (r, b) if r != "." => (r, b),
                    _ => (remote.to_string(), branch.to_string()),
                };
                let mut a = v(&["pull"], args);
                a.extend([remote, branch]);
                self.run_remote_git(&a, &format!("pulled {source}"));
            }
            _ => {}
        }
        Ok(())
    }

    /// magit-remote--cleanup-push-variables: remote.pushDefault and the
    /// branch.*.pushRemote that are REMOTE are set to NEW, or unset.
    pub(super) fn remote_cleanup_push_variables(&self, remote: &str, new: Option<&str>) {
        let set = |var: &str| match new {
            Some(n) => self.git.git_ok(&["config", var, n]),
            None => self.git.git_ok(&["config", "--unset", var]),
        };
        if self.git.git_string(&["config", "remote.pushDefault"]).ok().as_deref() == Some(remote) {
            set("remote.pushDefault");
        }
        let pattern = format!("^{}$", regex_quote(remote));
        for var in self.git.git_lines_pub(&["config", "--name-only", "--get-regexp", "^branch\\.[^.]*\\.pushRemote", &pattern]) {
            set(&var);
        }
    }

}
