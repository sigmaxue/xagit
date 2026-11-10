//! The suffixes of magit-submodule (magit-submodule.el) and
//! magit-list-submodules.

use super::*;

/// The predicates of magit-module-confirm.
#[derive(Clone, Copy, PartialEq)]
enum ModulePred {
    /// magit-module-worktree-p
    Worktree,
    /// magit-module-no-worktree-p
    NoWorktree,
}

impl ModulePred {
    fn name(self) -> &'static str {
        match self {
            ModulePred::Worktree => "magit-module-worktree-p",
            ModulePred::NoWorktree => "magit-module-no-worktree-p",
        }
    }
}

/// magit-submodule-read-path's initial input: the last component of
/// URL without ".git".
fn url_basename(url: &str) -> String {
    // "\\([^./]+\\)\\(\\.git\\)?$": the last run of characters other
    // than "." and "/", before a trailing ".git" when there is one
    let last = |s: &str| s[s.rfind(['.', '/']).map_or(0, |i| i + 1)..].to_string();
    match url.strip_suffix(".git").map(last) {
        Some(name) if !name.is_empty() => name,
        _ => last(url),
    }
}

/// The names of submodules whose path is PATH in `git config --list -f
/// .gitmodules` output.
fn names_for_path(config: &[String], path: &str) -> Vec<String> {
    config
        .iter()
        .filter_map(|l| {
            let (var, val) = l.split_once('=')?;
            (val == path).then(|| var.split('.').nth(1).map(str::to_string))?
        })
        .collect()
}

/// A tabulated-list cell: TEXT padded (or right-aligned) to WIDTH,
/// truncated with "…" when longer.
fn cell(text: &str, width: usize, right: bool) -> String {
    let n = text.chars().count();
    if n > width {
        let mut t: String = text.chars().take(width.saturating_sub(1)).collect();
        t.push('…');
        return t;
    }
    if right {
        format!("{text:>width$}")
    } else {
        format!("{text:<width$}")
    }
}

/// magit-submodule-list-columns: (HEADER WIDTH RIGHT-ALIGN).
const COLUMNS: [(&str, usize, bool); 9] = [
    ("Path", 25, false),
    ("Version", 25, false),
    ("Branch", 20, false),
    ("B<P", 3, true),
    ("B<U", 3, true),
    ("B>P", 3, true),
    ("B>U", 3, true),
    ("S", 3, true),
    ("B", 3, true),
];

impl App {
    /// magit-list-module-paths: the gitlinks in the index.
    fn list_module_paths(&self) -> Vec<String> {
        self.git
            .git_lines(&["ls-files", "-z", "--stage"])
            .iter()
            .filter_map(|l| {
                let (meta, path) = l.split_once('\t')?;
                let mut f = meta.split(' ');
                (f.next() == Some("160000") && f.next().is_some_and(|h| h.len() >= 40) && f.next() == Some("0"))
                    .then(|| path.to_string())
            })
            .collect()
    }

    fn module_dir(&self, module: &str) -> std::path::PathBuf {
        self.git.toplevel.join(module)
    }

    fn module_pred(&self, pred: ModulePred, module: &str) -> bool {
        let worktree = self.module_dir(module).join(".git").exists();
        (pred == ModulePred::Worktree) == worktree
    }

    /// magit-read-module-path: one of the modules satisfying PRED
    /// (require-match; there is no module at point).
    fn read_module_path(&mut self, prompt: &str, pred: Option<ModulePred>) -> Result<Option<String>> {
        let candidates: Vec<String> =
            self.list_module_paths().into_iter().filter(|m| pred.is_none_or(|p| self.module_pred(p, m))).collect();
        let table = Completion { candidates: candidates.clone(), require_match: true, multiple: false };
        let Some(v) = self.completing_read(prompt, None, Hist::Minibuffer, Some(table))? else {
            return Ok(None);
        };
        if v.is_empty() {
            self.set_msg("Nothing selected");
            return Ok(None);
        }
        if !candidates.contains(&v) {
            self.set_msg("[No match]");
            return Ok(None);
        }
        Ok(Some(v))
    }

    /// magit-module-confirm: with a prefix argument all modules
    /// satisfying PRED (confirmed), else one read with "VERB module".
    fn module_confirm(&mut self, verb: &str, pred: Option<ModulePred>) -> Result<Option<Vec<String>>> {
        if self.prefix_arg.is_none() {
            return Ok(self.read_module_path(&format!("{verb} module"), pred)?.map(|m| vec![m]));
        }
        let modules: Vec<String> =
            self.list_module_paths().into_iter().filter(|m| pred.is_none_or(|p| self.module_pred(p, m))).collect();
        if modules.is_empty() {
            self.set_msg(&match pred {
                Some(p) => format!("No modules satisfying {} available", p.name()),
                None => "No modules available".to_string(),
            });
            return Ok(None);
        }
        // (magit-confirm t nil "VERB %d modules" nil modules): always
        // asked, the prompt of a single module made from the action t
        let ok = self.confirm_items("t", "T", &format!("{verb} %d modules"), &modules)?;
        Ok(ok.then_some(modules))
    }

    /// Run `git ARGS -- MODULES` in the toplevel (local URLs allowed).
    fn run_submodule(&mut self, sub: &[&str], args: &[String], modules: &[String]) {
        let mut a: Vec<String> = vec!["-c".into(), "protocol.file.allow=always".into(), "submodule".into()];
        a.extend(sub.iter().map(|s| s.to_string()));
        a.extend(args.iter().cloned());
        a.push("--".into());
        a.extend(modules.iter().cloned());
        let res = self.git.git_fmt(&a);
        self.done("", res);
    }

    /// The suffixes of the magit-submodule transient.
    pub(super) fn submodule_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        // magit-submodule-arguments: the arguments each suffix understands
        let filter = |keep: &[&str]| -> Vec<String> { args.iter().filter(|a| keep.contains(&a.as_str())).cloned().collect() };
        match cmd {
            "submodule-add" => self.submodule_add(&filter(&["--force"]))?,
            "submodule-register" => {
                if let Some(m) = self.module_confirm("Register", Some(ModulePred::NoWorktree))? {
                    self.run_submodule(&["init"], &[], &m);
                }
            }
            "submodule-populate" => {
                if let Some(m) = self.module_confirm("Populate", Some(ModulePred::NoWorktree))? {
                    self.run_submodule(&["update", "--init"], &filter(&["--recursive"]), &m);
                }
            }
            "submodule-update" => {
                if let Some(m) = self.module_confirm("Update", Some(ModulePred::Worktree))? {
                    let keep = ["--force", "--remote", "--recursive", "--checkout", "--rebase", "--merge", "--no-fetch"];
                    self.run_submodule(&["update"], &filter(&keep), &m);
                }
            }
            "submodule-sync" => {
                if let Some(m) = self.module_confirm("Synchronize", Some(ModulePred::Worktree))? {
                    self.run_submodule(&["sync"], &filter(&["--recursive"]), &m);
                }
            }
            "submodule-unpopulate" => {
                if let Some(m) = self.module_confirm("Unpopulate", None)? {
                    self.run_submodule(&["deinit"], &filter(&["--force"]), &m);
                }
            }
            "submodule-remove" => self.submodule_remove(&filter(&["--force"]))?,
            _ => self.list_submodules(),
        }
        Ok(())
    }

    /// magit-submodule-add: URL (magit-read-string-ns), the path
    /// (read-directory-name, initially the URL's basename) and the name.
    fn submodule_add(&mut self, args: &[String]) -> Result<()> {
        let Some(url) = self.read_string_ns("Add submodule (remote url)")? else {
            return Ok(());
        };
        let top = self.git.toplevel.clone();
        let Some(dir) = self.read_directory_name("Add submodules at path: ", &top, &url_basename(&url))? else {
            return Ok(());
        };
        let path = match dir.strip_prefix(&top) {
            Ok(rel) => rel.to_string_lossy().into_owned(),
            Err(_) => dir.to_string_lossy().into_owned(),
        };
        if path.is_empty() {
            self.set_msg("Need non-empty input");
            return Ok(());
        }
        let Some(name) = self.submodule_read_name_for_path(&path)? else {
            return Ok(());
        };
        if !self.submodule_maybe_reuse_gitdir(&name, &path)? {
            return Ok(());
        }
        let mut a: Vec<String> = ["-c", "protocol.file.allow=always", "submodule", "add", "--name", &name]
            .iter()
            .map(|s| s.to_string())
            .collect();
        a.extend(args.iter().cloned());
        a.extend(["--".to_string(), url, path.clone()]);
        let res = self.git.git_fmt(&a);
        if res.is_ok() {
            let _ = self.git.git_fmt(&["submodule".into(), "absorbgitdirs".into(), path]);
        }
        self.done("", res);
        Ok(())
    }

    /// magit-submodule-read-name-for-path: the name (its basename pushed
    /// onto minibuffer-history), the default the name .gitmodules has
    /// for PATH, else PATH.
    fn submodule_read_name_for_path(&mut self, path: &str) -> Result<Option<String>> {
        let base = path.rsplit('/').next().unwrap_or(path).to_string();
        add_to_history(&mut self.minibuffer_history.borrow_mut(), &base, false);
        let config = self.git.git_lines(&["config", "--list", "-f", ".gitmodules"]);
        let default = names_for_path(&config, path).into_iter().next().unwrap_or_else(|| path.to_string());
        self.read_string_ns_default("Submodule name", Some(&default))
    }

    /// magit-submodule--maybe-reuse-gitdir: when $GIT_DIR/modules/NAME
    /// exists but PATH doesn't, use, rename or trash it (C-g: false).
    fn submodule_maybe_reuse_gitdir(&mut self, name: &str, path: &str) -> Result<bool> {
        let gitdir = self.git.gitdir.join("modules").join(name);
        let worktree = self.module_dir(path);
        if !gitdir.exists() || worktree.exists() {
            return Ok(true);
        }
        let prompt = format!(
            "{} already exists.\n\
             Type [u] to use the existing gitdir and create the working tree\n     \
             [r] to rename the existing gitdir and clone again\n     \
             [t] to trash the existing gitdir and clone again\n   \
             [C-g] to abort ",
            gitdir.display()
        );
        match self.read_char_choice(&prompt, &['u', 'r', 't'])? {
            Some('u') => {
                // magit-submodule--restore-worktree
                std::fs::create_dir_all(&worktree)?;
                let rel = relative_path(&gitdir, &worktree);
                std::fs::write(worktree.join(".git"), format!("gitdir: {rel}\n"))?;
                if let Err(e) = self.git.git_string_in(&worktree, &["reset", "--hard", "HEAD", "--"]) {
                    self.set_msg(&format!("{e:#}"));
                }
            }
            Some('r') => {
                let stamp = crate::ui::logbuf::format_time("%F-%T", crate::ui::logbuf::now().floor() as i64);
                let mut renamed = gitdir.clone().into_os_string();
                renamed.push(format!("-{stamp}"));
                std::fs::rename(&gitdir, renamed)?;
            }
            Some(_) => std::fs::remove_dir_all(&gitdir)?,
            None => return Ok(false),
        }
        Ok(true)
    }

    /// magit-anything-modified-p in MODULE.
    fn module_modified(&self, module: &str) -> bool {
        let dir = self.module_dir(module);
        let d = dir.to_string_lossy();
        let failed = |extra: &[&str]| {
            let mut a = vec!["-C", d.as_ref(), "diff", "--quiet"];
            a.extend(extra);
            a.extend(["--submodule=short", "--"]);
            !self.git.git_ok(&a)
        };
        let populated = std::fs::read_dir(&dir).map(|mut e| e.next().is_some()).unwrap_or(false);
        populated && (failed(&["--cached"]) || failed(&[]))
    }

    /// magit-submodule-remove: one module read; dirty ones omitted
    /// unless --force (then stashed, confirmed); gitdirs trashed with a
    /// prefix argument or magit-submodule-remove-trash-gitdirs.
    fn submodule_remove(&mut self, args: &[String]) -> Result<()> {
        let Some(module) = self.read_module_path("Remove module", None)? else {
            return Ok(());
        };
        let trash = self.prefix_arg.is_some() || self.options.submodule_remove_trash_gitdirs;
        let mut modules = vec![module];
        let modified: Vec<String> = modules.iter().filter(|m| self.module_modified(m)).cloned().collect();
        if !modified.is_empty() {
            let force = args.iter().any(|a| a == "--force");
            if force
                && self.confirm_items_noabort("remove-dirty-modules", "Remove dirty module %s", "Remove %d dirty modules", &modified)?
            {
                for m in &modified {
                    let dir = self.module_dir(m);
                    let _ = self.git.git_string_in(&dir, &["stash", "push", "-m", "backup before removal of this module"]);
                }
            } else {
                if !force {
                    self.set_msg(&if modified.len() > 1 {
                        format!("Omitting {} modules with uncommitted changes: {}", modified.len(), modified.join(", "))
                    } else {
                        format!("Omitting module {}, it has uncommitted changes", modified[0])
                    });
                }
                modules.retain(|m| !modified.contains(m));
            }
        }
        if modules.is_empty() {
            self.refresh();
            return Ok(());
        }
        let names: Vec<(String, String)> = if trash {
            self.git
                .git_lines(&["submodule", "foreach", "-q", "printf \"$sm_path\\0$name\\n\""])
                .chunks(2)
                .filter_map(|c| Some((c.first()?.clone(), c.get(1)?.clone())))
                .collect()
        } else {
            Vec::new()
        };
        let run = |app: &mut App, pre: &[&str], extra: &[String]| -> Result<String> {
            let mut a: Vec<String> = pre.iter().map(|s| s.to_string()).collect();
            a.extend(extra.iter().cloned());
            a.push("--".into());
            a.extend(modules.iter().cloned());
            app.git.git_fmt(&a)
        };
        let res = run(self, &["submodule", "absorbgitdirs"], &[])
            .and_then(|_| run(self, &["submodule", "deinit"], args))
            .and_then(|_| run(self, &["rm"], args));
        if let Err(e) = res {
            self.set_msg(&format!("{e:#}"));
            self.refresh();
            return Ok(());
        }
        if trash
            && self.confirm_items_noabort("trash-module-gitdirs", "Trash gitdir of module %s", "Trash gitdirs of %d modules", &modules)?
        {
            for m in &modules {
                match names.iter().find(|(p, _)| p == m) {
                    Some((_, name)) => {
                        let dir = self.git.gitdir.join("modules").join(name);
                        let _ = std::fs::remove_dir_all(dir);
                    }
                    None => {
                        self.set_msg(&format!("BUG: Weird module name and/or path for {m}"));
                        break;
                    }
                }
            }
        }
        self.refresh();
        Ok(())
    }

    /// magit-list-submodules: the columns of magit-submodule-list-columns
    /// for each module, sorted by path.
    fn list_submodules(&mut self) {
        let mut modules = self.list_module_paths();
        modules.sort();
        let header: Vec<String> = COLUMNS.iter().map(|(h, w, r)| cell(h, *w, *r)).collect();
        let mut out = vec![header.join(" ").trim_end().to_string()];
        for m in &modules {
            let row = self.module_row(m);
            let cells: Vec<String> = COLUMNS.iter().zip(&row).map(|((_, w, r), t)| cell(t, *w, *r)).collect();
            out.push(cells.join(" ").trim_end().to_string());
        }
        self.view_text = Some(("Modules".into(), out.join("\n")));
    }

    /// The cells of MODULE (only path and branch when unpopulated).
    fn module_row(&self, module: &str) -> Vec<String> {
        let dir = self.module_dir(module);
        if !dir.join(".git").exists() {
            let mut row = vec![module.to_string(), String::new(), "(unpopulated)".to_string()];
            row.resize(COLUMNS.len(), String::new());
            return row;
        }
        let git = |a: &[&str]| self.git.git_string_in(&dir, a).ok().filter(|s| !s.is_empty());
        let version = match git(&["describe", "--tags", "--dirty"]) {
            Some(v) if v.starts_with(|c: char| c.is_ascii_digit()) => format!(" {v}"),
            Some(v) => v,
            None => {
                let v = git(&["log", "-1", "--no-show-signature", "--format=#%cd-g%h", "--date=format:%Y%m%d"]).unwrap_or_default();
                let dirty = !self.git.git_ok(&["-C", &dir.to_string_lossy(), "diff", "--quiet", "--cached", "--submodule=short"])
                    || !self.git.git_ok(&["-C", &dir.to_string_lossy(), "diff", "--quiet", "--submodule=short"]);
                if dirty && !v.is_empty() {
                    format!("{v}-dirty")
                } else {
                    v
                }
            }
        };
        let branch = git(&["symbolic-ref", "--short", "-q", "HEAD"]);
        let count = |target: Option<String>, left: bool| -> String {
            let Some(t) = target else { return String::new() };
            let counts = git(&["rev-list", "--count", "--left-right", &format!("HEAD...{t}")]).unwrap_or_default();
            let mut it = counts.split_whitespace();
            let (a, b) = (it.next().unwrap_or("0"), it.next().unwrap_or("0"));
            if left { a } else { b }.to_string()
        };
        let upstream = git(&["rev-parse", "--abbrev-ref", "@{upstream}"]);
        let push = branch.as_ref().and_then(|b| {
            let remote = git(&["config", &format!("branch.{b}.pushRemote")]).or_else(|| git(&["config", "remote.pushDefault"]))?;
            let r = format!("{remote}/{b}");
            git(&["rev-parse", "--verify", "-q", &format!("refs/remotes/{r}")]).map(|_| r)
        });
        let stashes = git(&["stash", "list"]).map_or(0, |s| s.lines().count());
        let branches = git(&["for-each-ref", "--format=%(refname)", "refs/heads"]).map_or(0, |s| s.lines().count());
        vec![
            module.to_string(),
            version,
            branch.unwrap_or_default(),
            count(push.clone(), false),
            count(upstream.clone(), false),
            count(push, true),
            count(upstream, true),
            stashes.to_string(),
            branches.to_string(),
        ]
    }
}

/// file-relative-name of PATH relative to DIR.
fn relative_path(path: &std::path::Path, dir: &std::path::Path) -> String {
    let p: Vec<_> = path.components().collect();
    let d: Vec<_> = dir.components().collect();
    let common = p.iter().zip(&d).take_while(|(a, b)| a == b).count();
    let mut out: Vec<String> = vec!["..".to_string(); d.len() - common];
    out.extend(p[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    out.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_of_url() {
        assert_eq!(url_basename("https://example.com/foo/bar.git"), "bar");
        assert_eq!(url_basename("../lib"), "lib");
        assert_eq!(url_basename("git@host:x/my-lib"), "my-lib");
        assert_eq!(url_basename("foo/.git"), "git");
        assert_eq!(url_basename("https://h/x/"), "");
    }

    #[test]
    fn names() {
        let config = ["submodule.libx.path=lib/x".to_string(), "submodule.libx.url=../x".to_string()];
        assert_eq!(names_for_path(&config, "lib/x"), ["libx"]);
        assert!(names_for_path(&config, "lib/y").is_empty());
    }

    #[test]
    fn relative() {
        let p = std::path::Path::new("/r/.git/modules/a");
        assert_eq!(relative_path(p, std::path::Path::new("/r/lib/a")), "../../.git/modules/a");
    }

    #[test]
    fn cells() {
        assert_eq!(cell("ab", 4, false), "ab  ");
        assert_eq!(cell("2", 3, true), "  2");
        assert_eq!(cell("abcdef", 4, false), "abc…");
    }
}
