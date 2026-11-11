//! The suffixes of magit-run (!) and magit-ediff (E), and the
//! magit-git-mergetool transient (magit-extras.el).
//!
//! Emacs' Ediff sessions have no equivalent in a terminal: the "Show"
//! suffixes of `magit-ediff` display a text diff, and the stage/resolve
//! suffixes hand the terminal to an interactive git command instead.

use super::*;
use std::process::{Command, Stdio};

/// magit--read-mergetool: the tools of `git mergetool --tool-help'. Like
/// Magit, only the tab-indented tools after the first line are offered;
/// the description that git prints after the name is dropped, since it
/// isn't a valid `--tool=' or merge.tool value.
pub(super) fn mergetool_tools(g: &Git) -> Vec<String> {
    let mut tools = Vec::new();
    for line in g.git_lines(&["mergetool", "--tool-help"]).into_iter().skip(1) {
        let Some(rest) = line.strip_prefix("\t\t") else {
            break;
        };
        let Some(name) = rest.split_whitespace().next() else {
            continue;
        };
        tools.push(name.to_string());
    }
    tools
}

impl App {
    /// The suffixes of `!`, `E` and `magit-git-mergetool`.
    pub(super) fn run_menu_command(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        match cmd {
            "git-command" | "git-command-topdir" | "shell-command" | "shell-command-topdir" => self.run_shell_command(cmd)?,
            "run-gitk" | "run-gitk-all" | "run-gitk-branches" | "run-git-gui" => self.launch_gui(cmd),
            // magit-git-mergetool with a prefix argument shows its transient
            "git-mergetool" | "ediff-mergetool" if self.prefix_arg.is_some() => self.open_menu("mergetool-menu"),
            "git-mergetool" | "git-mergetool-invoke" | "ediff-mergetool" => self.resolve_with_mergetool(args, true, "Resolve")?,
            // magit-ediff-resolve-rest / -all: the merge session is xagit's
            // resolution view of the unmerged file read (E t: git mergetool)
            "ediff-resolve-rest" | "ediff-resolve-all" => {
                let _ = args;
                if let Some(file) = self.read_unmerged_file("Resolve file")? {
                    self.resolve_open_file(&file)?;
                }
            }
            "ediff-stage" => self.ediff_stage()?,
            _ => {}
        }
        Ok(())
    }

    /// magit-git-command(-topdir) and magit-shell-command(-topdir):
    /// magit--shell-command runs COMMAND through the shell asynchronously,
    /// with GIT_PAGER=cat, and shows the output in the process buffer.
    /// Here the output is captured and shown in the text view instead.
    fn run_shell_command(&mut self, cmd: &str) -> Result<()> {
        let topdir = cmd.ends_with("-topdir");
        let dir = if topdir { self.git.toplevel.clone() } else { self.cwd.clone() };
        if cmd.starts_with("git-command") {
            // read-shell-command's initial input, which can be deleted to
            // run another command
            *self.initial_input.borrow_mut() = Some("git ".into());
        }
        let shown = super::worktree::abbreviate(&dir.display().to_string());
        let prompt = format!("Async shell command in {shown}: ");
        let Some(line) = self.read_line_prompt(&prompt, None)? else {
            return Ok(());
        };
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
        let out = Command::new(&shell).arg("-c").arg(&line).current_dir(&dir).env("GIT_PAGER", "cat").output();
        match out {
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let label = format!("{shell} -c {line:?}");
                crate::git::exec::record_process(&dir, &label, o.status.success(), o.status.code(), &stderr);
                if !o.status.success() {
                    self.set_msg(&format!("{line} failed ({})", o.status));
                }
                let text = format!("$ {line}\n{}{}", String::from_utf8_lossy(&o.stdout), stderr);
                self.view_text = Some((format!("Async shell command in {shown}"), text));
                self.refresh();
            }
            Err(e) => self.set_msg(&format!("Cannot start {shell}: {}", crate::git::exec::sanitize(&e.to_string()))),
        }
        Ok(())
    }

    /// magit-run-gitk / -all / -branches and magit-run-git-gui: start a
    /// graphical program in the background (magit-process-file with 0 as
    /// the output buffer).
    fn launch_gui(&mut self, cmd: &str) {
        let (program, args) = match cmd {
            "run-gitk" => ("gitk", Vec::new()),
            "run-gitk-all" => ("gitk", vec!["--all"]),
            "run-gitk-branches" => ("gitk", vec!["--branches"]),
            _ => ("git", vec!["gui"]),
        };
        let label = std::iter::once(program).chain(args.iter().copied()).collect::<Vec<_>>().join(" ");
        let launched = Command::new(program)
            .args(&args)
            .current_dir(&self.git.toplevel)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match launched {
            Ok(_) => self.set_msg(&format!("Started {label}")),
            Err(e) => self.set_msg(&format!("Cannot start {label}: {}", crate::git::exec::sanitize(&e.to_string()))),
        }
    }

    /// magit-read-unmerged-file: the file at point if it is unmerged, else
    /// one of the unmerged files.
    fn read_unmerged_file(&mut self, prompt: &str) -> Result<Option<String>> {
        let files = self.git.unmerged_files();
        if files.is_empty() {
            self.set_msg("There are no unresolved conflicts");
            return Ok(None);
        }
        let default = self.current_file_name().filter(|f| files.contains(f));
        self.read_choice(prompt, &files, default.as_deref())
    }

    /// magit-git-mergetool (`E t`, `! m`) and the Ediff resolve suffixes:
    /// `git mergetool [--gui] ARGS -- FILE`. A GUI tool is run with its
    /// output captured; a terminal tool is given the terminal (with-editor
    /// style) instead of an Ediff session.
    fn resolve_with_mergetool(&mut self, args: &[String], gui: bool, prompt: &str) -> Result<()> {
        let Some(file) = self.read_unmerged_file(prompt)? else {
            return Ok(());
        };
        let mut a: Vec<String> = vec!["mergetool".into()];
        if gui {
            a.push("--gui".into());
        }
        a.extend(args.iter().cloned());
        a.extend(["--".into(), file]);
        let strs: Vec<&str> = a.iter().map(String::as_str).collect();
        if gui {
            match self.git.git_output(&strs) {
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    crate::git::exec::record(&self.git.toplevel, &strs, o.status.success(), o.status.code(), &stderr);
                    if !o.status.success() {
                        self.set_msg(&format!("git mergetool failed ({})", o.status));
                    }
                    let text = format!("$ git {}\n{}{}", strs.join(" "), String::from_utf8_lossy(&o.stdout), stderr);
                    self.view_text = Some(("git mergetool".into(), text));
                }
                Err(e) => self.set_msg(&format!("{e:#}")),
            }
        } else if let Err(e) = self.with_terminal(|g| g.git_interactive(&strs)) {
            self.set_msg(&format!("{e:#}"));
        }
        self.refresh();
        Ok(())
    }

    /// magit-ediff-stage: without Ediff, `git add -p FILE` is the
    /// interactive way to selectively stage changes of one file.
    fn ediff_stage(&mut self) -> Result<()> {
        let Some(file) = self.read_file_name("Selectively stage file", true)? else {
            return Ok(());
        };
        let a = ["add", "-p", "--", file.as_str()];
        if let Err(e) = self.with_terminal(|g| g.git_interactive(&a)) {
            self.set_msg(&format!("{e:#}"));
        }
        self.refresh();
        Ok(())
    }

    /// :inapt-if-not of magit-ediff-stage and the two resolve suffixes:
    /// magit-anything-modified-p / magit-anything-unmerged-p. An empty
    /// description leaves the menu's description unchanged.
    pub(super) fn ediff_inapt_states(&self) -> Vec<(String, String, bool)> {
        let modified = self.git.anything_modified();
        let unmerged = !self.git.unmerged_files().is_empty();
        [("s", !modified), ("m", !unmerged), ("M", !unmerged)]
            .into_iter()
            .map(|(k, inapt)| (k.to_string(), String::new(), inapt))
            .collect()
    }
}
