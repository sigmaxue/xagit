//! The suffixes of magit-tag (magit-tag.el): creating tags and release
//! tags, deleting tags, and pruning the tags that exist only locally or
//! only on a remote.

use super::*;

/// magit-release-tag-regexp: the prefix ("v", "version", "r", "release",
/// optionally followed by "-", "_" or "/") and the version of the
/// release tag TAG.
pub(super) fn parse_release_tag(tag: &str) -> Option<(String, String)> {
    for word in ["version", "v", "release", "r"] {
        for sep in ["-", "_", "/", ""] {
            let prefix = format!("{word}{sep}");
            if let Some(rest) = tag.strip_prefix(&prefix) {
                if is_release_version(rest) {
                    return Some((prefix, rest.to_string()));
                }
            }
        }
    }
    is_release_version(tag).then(|| (String::new(), tag.to_string()))
}

/// "[0-9]+(\.[0-9]+)*(-[a-zA-Z0-9-]+(\.[a-zA-Z0-9-]+)*)?" matching all of V.
fn is_release_version(v: &str) -> bool {
    let (numbers, suffix) = match v.split_once('-') {
        Some((n, s)) => (n, Some(s)),
        None => (v, None),
    };
    let numeric = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let ident = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    numbers.split('.').all(numeric) && suffix.is_none_or(|s| s.split('.').all(ident))
}

/// The separators of magit-tag-version-regexp-alist (case-insensitive),
/// the "[-._+ ]?" before them already removed.
fn version_separator_value(s: &str) -> Option<i64> {
    let lower = s.to_lowercase();
    if matches!(lower.as_str(), "-" | "." | "_" | "+") {
        return Some(-4);
    }
    let word = lower.strip_prefix(['-', '.', '_', '+', ' ']).unwrap_or(&lower);
    let word = word.strip_suffix('.').unwrap_or(word);
    match word {
        "snapshot" | "cvs" | "git" | "bzr" | "svn" | "hg" | "darcs" | "unknown" => Some(-4),
        "alpha" => Some(-3),
        "beta" => Some(-2),
        "pre" | "rc" => Some(-1),
        _ => None,
    }
}

/// version-to-list with magit-tag-version-regexp-alist: None for an
/// invalid version (an error in Emacs).
pub(super) fn version_to_list(ver: &str) -> Option<Vec<i64>> {
    let ver = if ver.starts_with('.') { format!("0{ver}") } else { ver.to_string() };
    let bytes = ver.as_bytes();
    let mut list = Vec::new();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        list.push(ver[start..i].parse::<i64>().ok()?);
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_digit() {
            i += 1;
        }
        let s = &ver[start..i];
        if s.is_empty() || s == "." {
            continue;
        }
        if let Some(v) = version_separator_value(s) {
            list.push(v);
        } else {
            // 22.3a is 22.3.1, but only at the end
            let letter = s.strip_prefix(['-', '.', '_', '+', ' ']).unwrap_or(s);
            match letter.as_bytes() {
                [c] if c.is_ascii_alphabetic() && i == bytes.len() => {
                    list.push(i64::from(c.to_ascii_lowercase() - b'a') + 1);
                }
                _ => return None,
            }
        }
    }
    (!list.is_empty()).then_some(list)
}

/// version-list-<: the lists compared as if padded with zeros.
fn version_list_cmp(a: &[i64], b: &[i64]) -> std::cmp::Ordering {
    let n = a.len().max(b.len());
    (0..n).map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0))).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal)
}

/// capitalize: the first letter of each word upcased, the rest downcased.
fn capitalize(s: &str) -> String {
    let mut out = String::new();
    let mut in_word = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if in_word {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            in_word = true;
        } else {
            out.push(c);
            in_word = false;
        }
    }
    out
}

impl App {
    /// magit--list-releases: (VERSION TAG MESSAGE) of the release tags,
    /// the highest first.
    fn list_releases(&self) -> Vec<(String, String, String)> {
        let mut releases: Vec<(Vec<i64>, (String, String, String))> = self
            .git
            .git_lines(&["tag", "-n"])
            .into_iter()
            .filter_map(|line| {
                let (tag, msg) = line.split_once(' ')?;
                let msg = msg.trim_start_matches(' ').to_string();
                let (_, ver) = parse_release_tag(tag)?;
                Some((version_to_list(&ver)?, (ver, tag.to_string(), msg)))
            })
            .collect();
        releases.sort_by(|a, b| version_list_cmp(&b.0, &a.0));
        releases.into_iter().map(|(_, r)| r).collect()
    }

    /// The suffixes of magit-tag.
    pub(super) fn tag_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        match cmd {
            // magit-tag-create: annotated with a prefix argument
            "tag-create" => {
                let table = Completion { candidates: self.git.tags(), require_match: false, multiple: false };
                let Some(name) =
                    self.completing_read("Create tag", None, Hist::Minibuffer, Some(table))?.filter(|s| !s.is_empty())
                else {
                    return Ok(());
                };
                let Some(rev) = self.read_branch_or_commit("Place tag on")? else {
                    return Ok(());
                };
                let mut args = args.to_vec();
                if self.prefix_arg.is_some() && !args.iter().any(|a| a == "--annotate") {
                    args.insert(0, "--annotate".into());
                }
                let mut a: Vec<&str> = vec!["tag"];
                a.extend(args.iter().map(String::as_str));
                a.extend([name.as_str(), rev.as_str()]);
                let res = self.with_terminal(|g| g.git_interactive(&a));
                self.done("", res.map(|()| String::new()));
            }
            "tag-release" => self.tag_release(args)?,
            "tag-prune" => self.tag_prune()?,
            _ => {}
        }
        Ok(())
    }

    /// magit-tag-release: a release tag for HEAD, its version from the
    /// message of HEAD ("Release version VERSION") or read with the
    /// highest release tag as initial input; the message (when
    /// annotating) that of the highest release with the version replaced.
    fn tag_release(&mut self, args: &[String]) -> Result<()> {
        let mut args = args.to_vec();
        let previous = self.list_releases().into_iter().next();
        let subject = self.git.git_string(&["show", "-s", "--format=%s", "HEAD"]).unwrap_or_default();
        let subject = subject.trim_end_matches('\n');
        // magit-release-commit-regexp
        let commit_ver = subject.strip_prefix("Release version ").filter(|v| !v.is_empty()).map(str::to_string);
        let tag = match (&previous, &commit_ver) {
            (None, _) => {
                // force a review of the message of the first release tag
                if !args.iter().any(|a| a == "--edit") {
                    args.insert(0, "--edit".into());
                }
                let initial = commit_ver.as_ref().map(|v| {
                    if v.starts_with(|c: char| c.is_ascii_digit()) {
                        format!("v{v}")
                    } else {
                        v.clone()
                    }
                });
                *self.initial_input.borrow_mut() = initial;
                self.read_line_hist("Create first release tag: ", None, Hist::None, None)?
            }
            (Some((_, ptag, _)), Some(v)) => {
                let prefix = parse_release_tag(ptag).map(|(p, _)| p).unwrap_or_default();
                Some(format!("{prefix}{v}"))
            }
            (Some((_, ptag, _)), None) => {
                *self.initial_input.borrow_mut() = Some(ptag.clone());
                self.read_line_hist(&format!("Create release tag (previous was {ptag}): "), None, Hist::None, None)?
            }
        };
        let Some(tag) = tag.filter(|t| !t.is_empty()) else {
            return Ok(());
        };
        let ver = parse_release_tag(&tag).map(|(_, v)| v);
        let annotate = args.iter().any(|a| ["--annotate", "--local-user", "--sign"].iter().any(|p| a.starts_with(p)));
        let msg = annotate.then(|| match (&previous, &ver) {
            (Some((pver, _, pmsg)), Some(v)) if pmsg.contains(pver.as_str()) => pmsg.replacen(pver.as_str(), v, 1),
            (Some((_, ptag, pmsg)), _) if pmsg.contains(ptag.as_str()) => pmsg.replacen(ptag.as_str(), &tag, 1),
            _ => {
                let name = self.git.toplevel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                format!("{} {}", capitalize(&name), ver.clone().unwrap_or_default())
            }
        });
        let mut a: Vec<&str> = vec!["tag"];
        a.extend(args.iter().map(String::as_str));
        if let Some(m) = &msg {
            a.extend(["-m", m.as_str()]);
        }
        a.push(&tag);
        let res = self.with_terminal(|g| g.git_interactive(&a));
        let ok = res.is_ok();
        self.done("", res.map(|()| String::new()));
        if ok {
            let args = self.prefix_args("show-refs-menu");
            self.setup_buffer(BufferMode::Refs { upstream: "HEAD".into() }, args);
        }
        Ok(())
    }

    /// magit-tag-prune: offer to delete the tags missing on REMOTE
    /// locally, and those missing locally from REMOTE.
    fn tag_prune(&mut self) -> Result<()> {
        let Some(remote) = self.read_remote("Prune tags using remote")? else {
            return Ok(());
        };
        let tags = self.git.tags();
        // magit-remote-list-tags
        let rtags: Vec<String> = self
            .git
            .git_lines(&["ls-remote", "--tags", &remote])
            .into_iter()
            .filter(|l| !l.ends_with("^{}"))
            .filter_map(|l| l.split_once('\t').and_then(|(_, r)| r.strip_prefix("refs/tags/")).map(str::to_string))
            .collect();
        let ltags: Vec<String> = tags.iter().filter(|t| !rtags.contains(t)).cloned().collect();
        let rtags: Vec<String> = rtags.into_iter().filter(|t| !tags.contains(t)).collect();
        if ltags.is_empty() && rtags.is_empty() {
            self.set_msg("Same tags exist locally and remotely");
        }
        let ltags = if self.confirm_items_noabort("t", "Delete %s locally", "Delete %d tags locally", &ltags)? {
            ltags
        } else {
            Vec::new()
        };
        let rtags = if self.confirm_items_noabort("t", "Delete %s from remote", "Delete %d tags from remote", &rtags)? {
            rtags
        } else {
            Vec::new()
        };
        if !ltags.is_empty() {
            let mut a = vec!["tag", "-d"];
            a.extend(ltags.iter().map(String::as_str));
            if let Err(e) = self.git.git_interactive(&a) {
                self.set_msg(&format!("{e:#}"));
            }
        }
        if !rtags.is_empty() {
            let mut a = vec!["push".to_string(), remote.clone()];
            a.extend(rtags.iter().map(|t| format!(":{t}")));
            self.run_remote_git(&a, "pruned remote tags");
        }
        self.refresh();
        Ok(())
    }

    /// magit-confirm with ITEMS and NOABORT: none confirmed when there
    /// are none.
    pub(super) fn confirm_items_noabort(&mut self, action: &str, one: &str, many: &str, items: &[String]) -> Result<bool> {
        match items {
            [] => Ok(false),
            [i] => self.confirm_noabort(action, &one.replacen("%s", i, 1)),
            is => self.confirm_noabort(action, &format!("{}\n\n{}", is.join("\n"), many.replacen("%d", &is.len().to_string(), 1))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_tags() {
        assert_eq!(parse_release_tag("v1.2.3"), Some(("v".into(), "1.2.3".into())));
        assert_eq!(parse_release_tag("version-2.0-rc.1"), Some(("version-".into(), "2.0-rc.1".into())));
        assert_eq!(parse_release_tag("release/3"), Some(("release/".into(), "3".into())));
        assert_eq!(parse_release_tag("1.0"), Some((String::new(), "1.0".into())));
        assert_eq!(parse_release_tag("x1.0"), None);
        assert_eq!(parse_release_tag("v1."), None);
        assert_eq!(version_to_list("1.0-rc1"), Some(vec![1, 0, -1, 1]));
        assert_eq!(version_to_list("2.3a"), Some(vec![2, 3, 1]));
        assert_eq!(version_to_list("1.0-beta.2"), Some(vec![1, 0, -2, 2]));
        assert_eq!(version_to_list("1.0-foo"), None);
        assert!(version_list_cmp(&[1, 0], &[1, 0, -1, 1]).is_gt());
        assert!(version_list_cmp(&[1, 10], &[1, 9]).is_gt());
        assert_eq!(capitalize("my-project"), "My-Project");
    }
}
