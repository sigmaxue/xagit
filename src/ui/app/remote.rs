//! The suffixes of magit-remote (magit-remote.el): adding, renaming,
//! removing, pruning and unshallowing remotes.

use super::*;
use crate::ui::options::SetPushDefault;

/// The regexp of a refspec "THEIRS" (any "*" of it ".*") matching NAME
/// anywhere (string-match-p).
fn refspec_matches(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        Some((pre, post)) => {
            let re = format!("{}.*{}", regex_quote(pre), regex_quote(post));
            crate::ui::regexp::Regexp::new(&re).is_ok_and(|r| r.is_match(name))
        }
        None => false,
    }
}

/// The url URL with the owner (the match of "\\([^:/]+\\)/[^/]+\\(\\.git\\)?\\'")
/// replaced by OWNER.
fn replace_url_owner(url: &str, owner: &str) -> Option<String> {
    let slash = url.rfind('/')?;
    if slash + 1 >= url.len() {
        return None;
    }
    let start = url[..slash].rfind([':', '/']).map_or(0, |i| i + 1);
    (start < slash).then(|| format!("{}{owner}{}", &url[..start], &url[slash..]))
}

impl App {
    /// magit-read-url: a string without whitespace, "~" expanded.
    fn read_url(&mut self, prompt: &str, initial: Option<String>) -> Result<Option<String>> {
        *self.initial_input.borrow_mut() = initial;
        let Some(url) = self.read_string_ns(prompt)? else {
            return Ok(None);
        };
        Ok(Some(match url.strip_prefix('~') {
            Some(rest) => format!("{}{rest}", std::env::var("HOME").unwrap_or_default()),
            None => url,
        }))
    }

    /// The suffixes of magit-remote.
    pub(super) fn remote_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        match cmd {
            // magit-remote-add: the url offered is that of origin with
            // its owner replaced by the name of the remote
            "remote-add" => {
                let Some(remote) = self.read_string_ns("Remote name")? else {
                    return Ok(());
                };
                let initial = self.config("remote.origin.url").and_then(|o| replace_url_owner(&o, &remote));
                let Some(url) = self.read_url("Remote url", initial)? else {
                    return Ok(());
                };
                let unset = self.config("remote.pushDefault").is_none();
                let set = match &self.options.remote_add_set_push_default {
                    SetPushDefault::Named(n) => *n == remote && unset,
                    SetPushDefault::Ask => self.yes_no(&format!("Set `remote.pushDefault' to \"{remote}\"?"))?,
                    SetPushDefault::AskIfUnset if unset => {
                        self.yes_no(&format!("Set `remote.pushDefault' to \"{remote}\"?"))?
                    }
                    _ => false,
                };
                let mut a: Vec<&str> = vec!["remote", "add"];
                a.extend(args.iter().map(String::as_str));
                a.extend([remote.as_str(), url.as_str()]);
                match self.git.git_interactive(&a) {
                    Ok(()) => self.set_msg(&format!("added remote {remote}")),
                    Err(e) => self.set_msg(&format!("{e:#}")),
                }
                // set whether adding (and fetching) succeeded or not
                if set {
                    self.git.git_interactive(&["config", "remote.pushDefault", &remote])?;
                }
                self.refresh();
            }
            // magit-remote-rename
            "remote-rename" => {
                let Some(old) = self.read_remote("Rename remote")? else {
                    return Ok(());
                };
                let Some(new) = self.read_string_ns(&format!("Rename {old} to"))? else {
                    return Ok(());
                };
                if new != old {
                    self.git.git_interactive(&["remote", "rename", &old, &new])?;
                    self.remote_cleanup_push_variables(&old, Some(&new));
                    self.refresh();
                }
            }
            // magit-remote-remove
            "remote-remove" => {
                let Some(remote) = self.read_remote("Delete remote")? else {
                    return Ok(());
                };
                self.git.git_interactive(&["remote", "rm", &remote])?;
                self.remote_cleanup_push_variables(&remote, None);
                self.refresh();
            }
            // magit-remote-prune
            "remote-prune" => {
                let Some(remote) = self.read_remote("Prune stale branches of remote")? else {
                    return Ok(());
                };
                self.run_remote_git(&["remote".into(), "prune".into(), remote.clone()], &format!("pruned {remote}"));
            }
            "remote-prune-refspecs" => self.remote_prune_refspecs()?,
            // magit-remote-unshallow
            "remote-unshallow" => {
                let remote = match self.current_remote() {
                    Some(r) => r,
                    None => match self.read_remote("Delete remote")? {
                        Some(r) => r,
                        None => return Ok(()),
                    },
                };
                let var = format!("remote.{remote}.fetch");
                let refspecs = self.git.git_lines(&["config", "--get-all", &var]);
                let standard = format!("+refs/heads/*:refs/remotes/{remote}/*");
                if let [one] = refspecs.as_slice() {
                    if !one.contains('*') && self.yes_or_no_p(&format!("Also replace refspec {one} with {standard}? "))? {
                        self.git.git_interactive(&["config", &var, &standard])?;
                    }
                }
                self.run_remote_git(&["fetch".into(), "--unshallow".into(), remote.clone()], &format!("unshallowed {remote}"));
            }
            _ => {}
        }
        Ok(())
    }

    /// magit-remote-prune-refspecs: remove the refspecs of a remote that
    /// fetch none of its refs, and the remote-tracking branches created
    /// by them; when all are stale offer to replace them with the
    /// default refspec or to remove the remote.
    fn remote_prune_refspecs(&mut self) -> Result<()> {
        let Some(remote) = self.read_remote("Prune refspecs of remote")? else {
            return Ok(());
        };
        let tracking: Vec<String> = self
            .git
            .git_lines(&["for-each-ref", "--format=%(refname)", &format!("refs/remotes/{remote}/")])
            .into_iter()
            .collect();
        let remote_refs: Vec<String> = self
            .git
            .git_lines(&["ls-remote", &remote])
            .into_iter()
            .filter_map(|l| l.split_once('\t').map(|(_, r)| r.to_string()))
            .filter(|r| !r.ends_with("^{}"))
            .collect();
        let var = format!("remote.{remote}.fetch");
        let refspecs = self.git.git_lines(&["config", "--get-all", &var]);
        let mut stale: Vec<(String, Vec<String>)> = Vec::new();
        for refspec in &refspecs {
            let Some((theirs, ours)) = refspec.trim_start_matches('+').split_once(':') else {
                continue;
            };
            let live = if theirs.contains('*') {
                remote_refs.iter().any(|r| refspec_matches(theirs, r))
            } else {
                remote_refs.iter().any(|r| r == theirs)
            };
            if !live {
                let refs: Vec<String> = if ours.contains('*') {
                    tracking.iter().filter(|r| refspec_matches(ours, r)).cloned().collect()
                } else {
                    tracking.iter().filter(|r| *r == ours).cloned().collect()
                };
                stale.insert(0, (refspec.clone(), refs));
            }
        }
        if stale.is_empty() {
            self.set_msg(&format!("No stale refspecs for remote \"{remote}\""));
            return Ok(());
        }
        if stale.len() == refspecs.len() {
            let prompt = format!(
                "All of {remote}'s refspecs are stale.  replace with [d]efault refspec, [r]emove remote, or [a]abort "
            );
            match self.read_char_choice(&prompt, &['s', 'r', 'a'])? {
                Some('s') => {
                    let _ = self.git.git_ok(&["config", "--unset-all", &var]);
                    self.git.git_interactive(&["config", "--add", &var, &format!("+refs/heads/*:refs/remotes/{remote}/*")])?;
                }
                Some('r') => self.git.git_interactive(&["remote", "rm", &remote])?,
                _ => {
                    self.set_msg("Abort");
                    return Ok(());
                }
            }
        } else {
            let ok = match stale.as_slice() {
                [(refspec, refs)] => self.confirm_items(
                    "prune-stale-refspecs",
                    &format!("Prune stale refspec {refspec} and branch %s"),
                    &format!("Prune stale refspec {refspec} and %d branches"),
                    refs,
                )?,
                _ => {
                    let nrefs: usize = stale.iter().map(|(_, r)| r.len()).sum();
                    let items: Vec<String> = stale
                        .iter()
                        .map(|(spec, refs)| {
                            format!("{spec}\n{}", refs.iter().map(|r| format!("  {r}")).collect::<Vec<_>>().join("\n"))
                        })
                        .collect();
                    self.confirm_items(
                        "prune-stale-refspecs",
                        "%s",
                        &format!("Prune %d stale refspecs and {nrefs} branches"),
                        &items,
                    )?
                }
            };
            if !ok {
                return Ok(());
            }
            for (refspec, refs) in &stale {
                self.git.git_interactive(&["config", "--unset", &var, &regex_quote(refspec)])?;
                for r in refs {
                    self.git.git_interactive(&["update-ref", "-d", r])?;
                }
            }
        }
        self.refresh();
        Ok(())
    }
}
