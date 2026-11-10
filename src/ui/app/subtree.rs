//! The suffixes of magit-subtree-import and magit-subtree-export
//! (magit-subtree.el).

use super::*;

/// magit-subtree-read-prefix's result for INPUT read in TOPDIR (with a
/// trailing slash): a relative name as is, an absolute one made relative
/// (an error when outside of TOPDIR); empty input is TOPDIR itself.
fn subtree_prefix(input: &str, topdir: &str) -> std::result::Result<String, String> {
    let input = match input.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("{}{rest}", std::env::var("HOME").unwrap_or_default()),
        _ => input.to_string(),
    };
    if input.is_empty() {
        return Ok("./".to_string());
    }
    if !input.starts_with('/') {
        return Ok(input);
    }
    match input.strip_prefix(topdir) {
        Some("") => Ok("./".to_string()),
        Some(rel) => Ok(rel.to_string()),
        None => Err(format!("{input} isn't inside the repository at {topdir}")),
    }
}

impl App {
    /// magit-subtree-read-prefix: read-directory-name in the toplevel
    /// (no initial input) with "PROMPT: ".
    pub(super) fn subtree_read_prefix(&mut self, prompt: &str) -> Result<Option<String>> {
        let table = Completion { candidates: self.git.subtree_prefixes(), ..Default::default() };
        let Some(input) = self.read_line_hist(&format!("{prompt}: "), None, Hist::Minibuffer, Some(table))? else {
            return Ok(None);
        };
        let topdir = format!("{}/", self.git.toplevel.to_string_lossy().trim_end_matches('/'));
        match subtree_prefix(input.trim(), &topdir) {
            Ok(p) => Ok(Some(p)),
            Err(e) => {
                self.set_msg(&e);
                Ok(None)
            }
        }
    }

    /// The suffixes of magit-subtree-import / -export: the prefix that
    /// of --prefix= (else read with PROMPT), the other arguments passed.
    pub(super) fn subtree_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let prompt = match cmd {
            "subtree-add" | "subtree-add-commit" => "Add subtree",
            "subtree-merge" => "Merge into subtree",
            "subtree-pull" => "Pull into subtree",
            "subtree-push" => "Push subtree",
            _ => "Split subtree",
        };
        // magit-subtree-prefix
        let prefix = match args.iter().find_map(|a| a.strip_prefix("--prefix=")) {
            Some(p) => p.to_string(),
            None => match self.subtree_read_prefix(prompt)? {
                Some(p) => p,
                None => return Ok(()),
            },
        };
        // magit-subtree-arguments
        let args: Vec<String> = args.iter().filter(|a| !a.starts_with("--prefix=")).cloned().collect();
        let rest: Vec<String> = match cmd {
            "subtree-add" | "subtree-pull" => {
                let Some(remote) = self.read_remote_or_url("From repository")? else {
                    return Ok(());
                };
                let Some(r) = self.read_refspec("Ref", &remote)? else {
                    return Ok(());
                };
                vec![remote, r]
            }
            "subtree-push" => {
                let Some(remote) = self.read_remote_or_url("To repository")? else {
                    return Ok(());
                };
                let Some(r) = self.read_string_ns("To reference")? else {
                    return Ok(());
                };
                vec![remote, r]
            }
            _ => {
                let Some(c) = self.read_string_ns("Commit")? else {
                    return Ok(());
                };
                vec![c]
            }
        };
        // magit-git-subtree
        let sub = match cmd {
            "subtree-add" | "subtree-add-commit" => "add",
            _ => cmd.trim_start_matches("subtree-"),
        };
        let mut a: Vec<String> = vec!["subtree".into(), sub.into(), format!("--prefix={prefix}")];
        a.extend(args);
        a.extend(rest);
        let res = self.git.git_fmt(&a);
        self.done("", res);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes() {
        assert_eq!(subtree_prefix("lib/x", "/r/"), Ok("lib/x".into()));
        assert_eq!(subtree_prefix("/r/lib", "/r/"), Ok("lib".into()));
        assert_eq!(subtree_prefix("", "/r/"), Ok("./".into()));
        assert_eq!(subtree_prefix("/s/lib", "/r/"), Err("/s/lib isn't inside the repository at /r/".into()));
    }
}
