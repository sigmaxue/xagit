//! git/wip.rs — magit-wip.el: committing the index and the worktree to
//! the work-in-progress refs `<namespace>index/<ref>` and
//! `<namespace>wtree/<ref>` (magit-wip-commit and its hooks), and the
//! refs shown by the wip logs.

use super::Git;
use anyhow::{bail, Result};

/// The options of magit-wip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wip {
    /// magit-wip-namespace
    pub namespace: String,
    /// magit-wip-merge-branch
    pub merge_branch: bool,
}

impl Default for Wip {
    fn default() -> Wip {
        Wip { namespace: "refs/wip/".into(), merge_branch: false }
    }
}

impl Wip {
    /// magit--wip-ref: KIND (`index/` or `wtree/`) of REF (a full ref,
    /// a branch or HEAD; the current branch when None).
    pub fn wip_ref(&self, g: &Git, kind: &str, r: Option<&str>) -> String {
        let tail = match r {
            Some(r) if r.starts_with("refs/") => r.to_string(),
            Some("HEAD") => "HEAD".into(),
            Some(b) => format!("refs/heads/{b}"),
            None => g.current_branch().map_or_else(|| "HEAD".into(), |b| format!("refs/heads/{b}")),
        };
        format!("{}{kind}{tail}", self.namespace)
    }

    /// magit-wip-commit: commit the index and the worktree (all tracked
    /// files, or FILES) to the wip refs of the current branch. MSG
    /// starting with a space is appended to a generated message.
    pub fn commit(&self, g: &Git, files: &[String], msg: Option<&str>) -> Result<Option<String>> {
        let Some(r) = get_ref(g) else { return Ok(None) };
        self.commit_index(g, &r, files, msg)?;
        self.commit_worktree(g, &r, files, msg)
    }

    /// magit-wip-commit-index
    fn commit_index(&self, g: &Git, r: &str, files: &[String], msg: Option<&str>) -> Result<()> {
        let wipref = self.wip_ref(g, "index/", Some(r));
        let parent = get_parent(g, r, &wipref);
        let tree = g.git_string(&["write-tree"])?;
        self.update_wipref(g, r, &wipref, &tree, parent, files, msg, "index")
    }

    /// magit-wip-commit-worktree: FILES (directories left out; nothing
    /// when only directories are given) or all tracked files added to a
    /// temporary index read from the parent. A message when the index
    /// cannot be read.
    pub fn commit_worktree(&self, g: &Git, r: &str, files: &[String], msg: Option<&str>) -> Result<Option<String>> {
        let given = !files.is_empty();
        let files: Vec<String> = files.iter().filter(|f| !g.toplevel.join(f).is_dir()).cloned().collect();
        if given && files.is_empty() {
            return Ok(None);
        }
        let wipref = self.wip_ref(g, "wtree/", Some(r));
        let parent = get_parent(g, r, &wipref);
        let Some(tree) = temp_index_tree(g, &parent, &files) else {
            return Ok(Some("Index locked; no worktree wip commit created".into()));
        };
        self.update_wipref(g, r, &wipref, &tree, parent, &files, msg, "worktree")?;
        Ok(None)
    }

    /// magit-wip-update-wipref: start the wip ref at REF (or merge REF
    /// into it), then commit TREE when it differs from the parent.
    #[allow(clippy::too_many_arguments)]
    fn update_wipref(
        &self,
        g: &Git,
        r: &str,
        wipref: &str,
        tree: &str,
        mut parent: String,
        files: &[String],
        msg: Option<&str>,
        start_msg: &str,
    ) -> Result<()> {
        if parent != wipref && (!self.merge_branch || rev_verify(g, wipref).is_none()) {
            let m = format!("start autosaving {start_msg}");
            let c = g.git_string(&["commit-tree", "--no-gpg-sign", "-p", &parent, "-m", &m, &format!("{parent}^{{tree}}")])?;
            update_ref(g, wipref, &m, &c)?;
            parent = wipref.to_string();
        } else if self.merge_branch && {
            let merge = g.git_string(&["log", "--format=%H", "-1", "--merges", wipref]).unwrap_or_default();
            !ancestor(g, r, wipref) || !ancestor(g, &format!("{merge}^2"), r)
        } {
            let m = format!("merge {r} into {start_msg}");
            let c = g.git_string(&["commit-tree", "--no-gpg-sign", "-p", wipref, "-p", r, "-m", &m, &format!("{r}^{{tree}}")])?;
            update_ref(g, wipref, &m, &c)?;
            parent = wipref.to_string();
        }
        let mut a = vec!["diff-tree", "--quiet", parent.as_str(), tree, "--"];
        a.extend(files.iter().map(String::as_str));
        if g.git_output(&a)?.status.code() != Some(1) {
            return Ok(());
        }
        let msg = match msg {
            Some(m) if !m.starts_with(' ') => m.to_string(),
            m => {
                let head = match files {
                    [] => "autosave tracked files".to_string(),
                    [f] => format!("autosave {f}"),
                    fs => format!("autosave {} files", fs.len()),
                };
                format!("{head}{}", m.unwrap_or_default())
            }
        };
        let c = g.git_string(&["commit-tree", "--no-gpg-sign", "-p", &parent, "-m", &msg, tree])?;
        update_ref(g, wipref, &msg, &c)
    }

    /// magit-wip-log-get-tips: WIPREF and the tips of its COUNT - 1
    /// previous "branches" (the commits before each "start autosaving"
    /// in its reflog).
    pub fn log_tips(g: &Git, wipref: &str, count: usize) -> Vec<String> {
        let reflog = g.git_lines(&["reflog", wipref]);
        if reflog.is_empty() {
            return Vec::new();
        }
        let is_start = |l: &str| {
            l.split_once(": ").is_some_and(|(h, m)| {
                h.contains(' ') && (m.starts_with("start autosaving") || m.starts_with("restart autosaving"))
            })
        };
        let mut tips = vec![wipref.to_string()];
        let mut rest: &[String] = &reflog;
        let mut count = count;
        while !rest.is_empty() && count > 1 {
            match rest.iter().position(|l| is_start(l)) {
                Some(i) => rest = &rest[i..],
                None => rest = &[],
            }
            if let Some(next) = rest.get(1) {
                if let Some(sel) = next.split_once(' ').and_then(|(_, s)| s.split(':').next()) {
                    tips.push(sel.to_string());
                }
            }
            rest = rest.get(2..).unwrap_or(&[]);
            count -= 1;
        }
        tips
    }
}

/// magit-rev-verify
fn rev_verify(g: &Git, rev: &str) -> Option<String> {
    g.git_string(&["rev-parse", "--verify", "-q", rev]).ok().filter(|s| !s.is_empty())
}

/// magit-rev-ancestor-p
fn ancestor(g: &Git, a: &str, b: &str) -> bool {
    g.git_ok(&["merge-base", "--is-ancestor", a, b])
}

/// magit-wip-get-ref: the current branch's full ref (HEAD if detached),
/// None without commits.
pub fn get_ref(g: &Git) -> Option<String> {
    let r = g.git_string(&["symbolic-ref", "HEAD"]).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "HEAD".into());
    rev_verify(g, &r).map(|_| r)
}

/// magit-wip-get-parent: WIPREF if it contains REF, else REF.
fn get_parent(g: &Git, r: &str, wipref: &str) -> String {
    let contains = rev_verify(g, wipref).is_some()
        && g.git_string(&["merge-base", wipref, r]).ok().filter(|s| !s.is_empty()) == rev_verify(g, r);
    if contains { wipref.to_string() } else { r.to_string() }
}

/// magit-wip--update-ref
fn update_ref(g: &Git, r: &str, message: &str, rev: &str) -> Result<()> {
    if !g.git_ok(&["update-ref", "--create-reflog", "-m", message, r, rev]) {
        bail!("Cannot update {r} with {rev}");
    }
    Ok(())
}

/// magit-with-temp-index PARENT ("--reset" "-i"): FILES (else all
/// tracked files) added to an index read from PARENT, its tree.
fn temp_index_tree(g: &Git, parent: &str, files: &[String]) -> Option<String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let file = g.gitdir.join(format!("index.magit.{}{nanos}", std::process::id()));
    let path = file.to_string_lossy().into_owned();
    let tree = (|| {
        if !g.git_ok(&["read-tree", "--reset", "-i", parent, &format!("--index-output={path}")]) {
            return None;
        }
        let env = [("GIT_INDEX_FILE", path.as_str())];
        if files.is_empty() {
            g.git_output_env(&env, &["add", "-u", "."]).ok()?;
        } else {
            let mut a = vec!["update-index", "--add", "--remove", "--ignore-skip-worktree-entries", "--"];
            a.extend(files.iter().map(String::as_str));
            g.git_output_env(&env, &a).ok()?;
        }
        let out = g.git_output_env(&env, &["write-tree"]).ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    })();
    let _ = std::fs::remove_file(&file);
    tree
}
