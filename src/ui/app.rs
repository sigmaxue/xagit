//! ui/app.rs — application state machine, event loop, prompts and command dispatch.

use super::keys::Key;
use super::transient::{registry, resolve_menu, Action, Lookup, Menu, MenuEntry, RepoState};
use super::view::{render_status, BufferLine, LineKind};
use crate::git::exec::Editor;
use crate::git::ops::Untracked;
use crate::git::status::Status;
use crate::git::Git;
use anyhow::Result;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use std::collections::BTreeMap;
use std::io::{stdout, Write};

pub struct App {
    pub git: Git,
    menus: BTreeMap<String, Menu>,
    /// stack of open transient menus + their active switches
    stack: Vec<(String, BTreeMap<String, bool>)>,
    /// keys typed so far of a multi-key transient binding ("-" of "-a")
    pending: Vec<Key>,
    lines: Vec<BufferLine>,
    cursor: usize,
    message: String,
    quit: bool,
    /// pending text-mode buffer shown instead of status (diff/log/show output)
    view_text: Option<(String, String)>, // (title, content)
    /// directory ngit was started in (magit-git-command's default-directory)
    pub cwd: std::path::PathBuf,
    /// arguments for diff views, changed with `D` (magit-diff-refresh)
    pub diff_args: Vec<String>,
    /// arguments for log views, changed with `L` (magit-log-refresh);
    /// Magit's default is "--graph --decorate"
    pub log_args: Vec<String>,
    /// command that produced the current text view, re-run by `D g` / `L g`
    last_view: Option<String>,
}

/// What a key does in the status buffer when no transient is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// q / ESC: leave the text view, or quit (magit-mode-bury-buffer)
    Bury,
    Refresh,
    Down,
    Up,
    Toggle,
    Visit,
    Menu(&'static str),
    Command(&'static str),
}

/// Choices cycled by `b C r` / `b C R` (magit-branch.<branch>.rebase, magit-pull.rebase).
pub const REBASE_CHOICES: &[&str] = &["true", "merges", "interactive", "false"];
/// Choices cycled by `b C a m` (magit-branch.autoSetupMerge).
pub const AUTOSETUP_MERGE_CHOICES: &[&str] = &["always", "true", "false"];
/// Choices cycled by `b C a r` (magit-branch.autoSetupRebase).
pub const AUTOSETUP_REBASE_CHOICES: &[&str] = &["always", "local", "remote", "never"];
/// magit-published-branches (default)
const PUBLISHED_BRANCHES: &[&str] = &["origin/master"];

/// Status buffer key map (magit-status-mode-map + magit-mode-map).
/// `k` is magit-discard and `j` is magit-status-jump (Magit has no vi-style
/// movement); move with n / p, arrows, or C-n / C-p.
pub fn status_binding(key: &Key) -> Option<Binding> {
    use Binding::*;
    if key.ctrl {
        return match key.code.as_str() {
            "n" => Some(Down),
            "p" => Some(Up),
            _ => None,
        };
    }
    Some(match key.code.as_str() {
        "q" | "esc" => Bury,
        "h" | "?" => Menu("dispatch"),
        "g" | "G" => Refresh,
        "n" | "down" => Down,
        "p" | "up" => Up,
        "j" => Command("jump"),
        "^" => Command("section-up"),
        "tab" => Toggle,
        "enter" => Visit,
        "!" => Menu("run-menu"),
        "$" => Command("process-buffer"),
        "H" => Command("section-info"),
        "J" => Command("display-buffer"),
        "Y" => Command("cherries"),
        "%" => Menu("worktree-menu"),
        "s" => Command("stage"),
        "S" => Command("stage-all"),
        "u" => Command("unstage"),
        "U" => Command("unstage-all"),
        "k" => Command("discard"),
        "K" => Command("untrack"),
        "x" => Command("reset-quickly"),
        "a" => Command("apply"),
        "v" => Command("reverse"),
        "y" => Command("show-refs"),
        ":" | "Q" => Command("git-command"),
        "I" => Command("init"),
        "C" => Command("clone"),
        "e" => Command("ediff-dwim"),
        "D" => Menu("diff-refresh-menu"),
        "L" => Menu("log-refresh-menu"),
        "V" => Menu("revert-menu"),
        "c" => Menu("commit-menu"),
        "b" => Menu("branch-menu"),
        "B" => Menu("bisect-menu"),
        "E" => Menu("ediff-menu"),
        "f" => Menu("fetch-menu"),
        "F" => Menu("pull-menu"),
        "P" => Menu("push-menu"),
        "m" => Menu("merge-menu"),
        "M" => Menu("remote-menu"),
        "o" => Menu("submodule-menu"),
        "O" => Menu("subtree-menu"),
        "r" => Menu("rebase-menu"),
        "t" => Menu("tag-menu"),
        "T" => Menu("notes-menu"),
        "w" => Menu("patches-menu"),
        "W" => Menu("patch-menu"),
        "z" => Menu("stash-menu"),
        "Z" => Menu("worktree-menu"),
        "X" => Menu("reset-menu"),
        "i" => Menu("gitignore-menu"),
        "l" => Menu("log-menu"),
        "d" => Menu("diff-menu"),
        "A" => Menu("cherry-pick-menu"),
        _ => return None,
    })
}

impl App {
    pub fn new(git: Git) -> App {
        let cwd = git.toplevel.clone();
        let mut app = App {
            git,
            menus: registry(),
            stack: Vec::new(),
            pending: Vec::new(),
            lines: Vec::new(),
            cursor: 0,
            message: String::new(),
            quit: false,
            view_text: None,
            cwd,
            diff_args: Vec::new(),
            log_args: vec!["--graph".into(), "--decorate".into()],
            last_view: None,
        };
        app.refresh();
        app
    }

    pub fn refresh(&mut self) {
        let st = Status::load(&self.git);
        self.lines = render_status(&self.git, &st);
        if self.cursor >= self.lines.len() {
            self.cursor = self.lines.len().saturating_sub(1);
        }
        if !self.stack.is_empty() {
            // transient stays on top after refresh (Magit refreshes but keeps popup)
        }
    }

    // ------------------------------------------------------------ rendering

    fn draw(&self) {
        let mut out = stdout();
        let _ = execute!(out, Clear(ClearType::All));
        let mut y = 0;
        let mut shown = self.lines.iter().enumerate();

        // status buffer
        for (i, line) in &mut shown {
            let marker = if i == self.cursor { "> " } else { "  " };
            let _ = execute!(
                out,
                crossterm::cursor::MoveTo(0, y as u16),
                crossterm::style::Print(format!("{marker}{}\r\n", line.text))
            );
            y += 1;
        }

        // transient popup
        if let Some((name, active)) = self.stack.last() {
            if let Some(menu) = self.menus.get(name) {
                let _ = execute!(
                    out,
                    crossterm::cursor::MoveTo(0, y as u16),
                    crossterm::style::Print(format!("\r\n{}\r\n", menu.title))
                );
                y += 2;
                for l in menu.render(active).lines() {
                    let _ = execute!(
                        out,
                        crossterm::cursor::MoveTo(0, y as u16),
                        crossterm::style::Print(format!("  {l}\r\n"))
                    );
                    y += 1;
                }
                let _ = execute!(
                    out,
                    crossterm::cursor::MoveTo(0, y as u16),
                    crossterm::style::Print(format!("  q/C-g quit popup\r\n"))
                );
                y += 1;
            }
        }

        // text view (diff/log/show)
        if let Some((title, content)) = &self.view_text {
            let _ = execute!(
                out,
                crossterm::cursor::MoveTo(0, y as u16),
                crossterm::style::Print(format!("\r\n--- {title} ---\r\n"))
            );
            y += 2;
            for l in content.lines().take(200) {
                let _ = execute!(
                    out,
                    crossterm::cursor::MoveTo(0, y as u16),
                    crossterm::style::Print(format!("  {l}\r\n"))
                );
                y += 1;
            }
        }

        // footer
        let branch = self.git.head_desc();
        let _ = execute!(
            out,
            crossterm::cursor::MoveTo(0, y as u16),
            crossterm::style::Print(format!(
                "\r\n[ngit:{branch}] {}\r\n(h/? dispatch, g refresh, q quit, TAB toggle, RET visit, n/p move)",
                self.message
            ))
        );
        let _ = out.flush();
    }

    // ------------------------------------------------------------ input helpers

    fn read_line_prompt(&self, prompt: &str, default: Option<&str>) -> Result<Option<String>> {
        // Leave raw mode temporarily to read a normal line.
        let _ = disable_raw_mode();
        let mut out = stdout();
        let _ = execute!(
            out,
            crossterm::cursor::MoveTo(0, 500),
            crossterm::style::Print(format!("\r\n{prompt}")),
            crossterm::style::Print(
                default
                    .map(|d| format!(" (default: {d}): "))
                    .unwrap_or_else(|| ": ".into())
            )
        );
        let _ = out.flush();
        let mut buf = String::new();
        let n = std::io::stdin().read_line(&mut buf)?;
        let _ = enable_raw_mode();
        if n == 0 {
            return Ok(None); // EOF / C-d
        }
        let mut s = buf.trim().to_string();
        if s.is_empty() {
            if let Some(d) = default {
                s = d.to_string();
            }
        }
        if s.is_empty() {
            return Ok(None);
        }
        Ok(Some(s))
    }

    fn yes_no(&self, prompt: &str) -> Result<bool> {
        loop {
            let Some(ans) = self.read_line_prompt(&format!("{prompt} (y/n)"), Some("n"))? else {
                return Ok(false);
            };
            match ans.to_lowercase().as_str() {
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                _ => continue,
            }
        }
    }

    fn pick<'a>(&mut self, prompt: &str, choices: &'a [String], default: Option<&str>) -> Result<Option<&'a String>> {
        if choices.is_empty() {
            self.message = "no candidates".into();
            return Ok(None);
        }
        // simple completion: substring match or exact
        let Some(inp) = self.read_line_prompt(
            prompt,
            default.or(Some(&choices[0])),
        )? else {
            return Ok(None);
        };
        if let Some(exact) = choices.iter().find(|c| c.as_str() == inp) {
            return Ok(Some(exact));
        }
        let hits: Vec<&String> = choices.iter().filter(|c| c.contains(inp.as_str())).collect();
        if hits.len() == 1 {
            Ok(Some(hits[0]))
        } else if hits.is_empty() {
            self.message = "no match".into();
            Ok(None)
        } else {
            Ok(Some(hits[0]))
        }
    }

    fn set_msg(&mut self, m: &str) {
        self.message = m.to_string();
    }

    // ------------------------------------------------------------ selection helpers

    fn current_line(&self) -> Option<&BufferLine> {
        self.lines.get(self.cursor)
    }

    /// Pick one of the last 30 commits; returns the abbreviated hash.
    fn pick_recent_commit(&mut self, prompt: &str) -> Result<Option<String>> {
        let revs: Vec<String> = self
            .git
            .log(None, 30)?
            .iter()
            .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
            .filter(|r| !r.is_empty())
            .collect();
        Ok(self.pick(prompt, &revs, None)?.cloned())
    }

    fn current_file(&self) -> Option<(String, String)> {
        match self.current_line() {
            Some(BufferLine { kind: LineKind::File { section, path }, .. }) => {
                Some((section.clone(), path.clone()))
            }
            _ => None,
        }
    }

    fn files_in_current_section(&self) -> Vec<String> {
        let Some(cur) = self.current_line() else { return vec![] };
        let section = match &cur.kind {
            LineKind::File { section, .. } => section.clone(),
            LineKind::SectionHeader { section, .. } => section.clone(),
            _ => return vec![],
        };
        self.lines
            .iter()
            .filter_map(|l| match &l.kind {
                LineKind::File { section: s, path } if *s == section => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    // ------------------------------------------------------------ event loop

    pub fn run(&mut self) -> Result<()> {
        let _ = enable_raw_mode();
        let _ = execute!(stdout(), EnterAlternateScreen);
        self.draw();
        loop {
            if self.quit {
                break;
            }
            match event::read()? {
                Event::Key(ev) => {
                    if ev.kind == crossterm::event::KeyEventKind::Release {
                        continue;
                    }
                    let key = Key::from_event(&ev);
                    self.handle_key(key)?;
                }
                _ => {}
            }
            self.draw();
        }
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        Ok(())
    }

    fn handle_key(&mut self, key: Key) -> Result<()> {
        // transient popup takes precedence
        if let Some((name, active)) = self.stack.last().cloned() {
            if let Some(menu) = self.menus.get(&name).cloned() {
                return self.handle_transient_key(key, menu, active);
            }
        }
        // status buffer bindings (magit-status-mode-map)
        let Some(binding) = status_binding(&key) else {
            return Ok(());
        };
        match binding {
            Binding::Bury => {
                if self.view_text.is_some() {
                    self.view_text = None;
                    self.set_msg("");
                } else {
                    self.quit = true;
                }
            }
            Binding::Refresh => {
                self.refresh();
                self.set_msg("refreshed");
            }
            Binding::Down => {
                if self.cursor + 1 < self.lines.len() {
                    self.cursor += 1;
                }
            }
            Binding::Up => self.cursor = self.cursor.saturating_sub(1),
            Binding::Toggle => self.toggle_section(),
            Binding::Visit => self.visit_thing()?,
            Binding::Menu(m) => self.open_menu(m),
            Binding::Command(c) => self.run_command(c, &BTreeMap::new())?,
        }
        Ok(())
    }

    fn open_menu(&mut self, name: &str) {
        let name = resolve_menu(name, self.repo_state());
        if !self.menus.contains_key(&name) {
            return;
        }
        // diff/log menus start with the arguments currently in use, like
        // Magit's buffer-local magit-diff/log arguments
        let preset: &[String] = match name.as_str() {
            "diff-menu" | "diff-refresh-menu" => &self.diff_args,
            "log-menu" | "log-refresh-menu" => &self.log_args,
            _ => &[],
        };
        let active: BTreeMap<String, bool> = preset.iter().map(|a| (a.clone(), true)).collect();
        self.pending.clear();
        self.stack.push((name, active));
    }

    fn repo_state(&self) -> RepoState {
        RepoState {
            merge: self.git.merge_in_progress(),
            rebase: self.git.rebase_in_progress(),
            sequencer: self.git.pick_or_revert_in_progress(),
            am: self.git.am_in_progress(),
            bisect: self.git.bisect_in_progress(),
        }
    }

    /// Name of the transient currently shown, if any.
    pub fn current_menu(&self) -> Option<&str> {
        self.stack.last().map(|(n, _)| n.as_str())
    }

    /// Feed one key (Emacs notation, e.g. "h", "C-g", "<tab>") as if typed.
    /// Only keys that do not prompt should be used outside the terminal.
    pub fn press(&mut self, label: &str) -> Result<()> {
        self.handle_key(super::keys::key_from_label(label))
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Switches enabled in the transient currently shown.
    pub fn active_switches(&self) -> Vec<String> {
        self.stack
            .last()
            .map(|(_, a)| a.iter().filter(|(_, on)| **on).map(|(k, _)| k.clone()).collect())
            .unwrap_or_default()
    }

    /// Give the terminal to a program that needs it (the user's editor for a
    /// rebase todo list or commit message), then take it back.
    fn with_terminal<T>(&mut self, f: impl FnOnce(&Git) -> T) -> T {
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        let r = f(&self.git);
        let _ = enable_raw_mode();
        let _ = execute!(stdout(), EnterAlternateScreen);
        r
    }

    fn handle_transient_key(
        &mut self,
        key: Key,
        menu: Menu,
        mut active: BTreeMap<String, bool>,
    ) -> Result<()> {
        if key.code == "q" || key.code == "esc" || (key.ctrl && key.code == "g") {
            self.pending.clear();
            self.stack.pop();
            return Ok(());
        }
        let mut typed = std::mem::take(&mut self.pending);
        typed.push(key);
        let entry: MenuEntry = match menu.lookup(&typed) {
            Lookup::Found(e) => e.clone(),
            Lookup::Prefix => {
                self.pending = typed;
                return Ok(());
            }
            Lookup::None => {
                let keys: Vec<String> = typed.iter().map(Key::label).collect();
                self.set_msg(&format!("{} is undefined", keys.join(" ")));
                return Ok(());
            }
        };
        match entry.action {
            Action::Switch(sw) => {
                let on = *active.get(&sw).unwrap_or(&false);
                active.insert(sw, !on);
                if let Some(top) = self.stack.last_mut() {
                    top.1 = active;
                }
            }
            Action::Submenu(name) => {
                self.stack.pop();
                self.open_menu(&name);
            }
            Action::Command(cmd) => {
                self.stack.pop();
                self.run_command(&cmd, &active)?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ commands

    fn args_from(&self, active: &BTreeMap<String, bool>) -> Vec<String> {
        active
            .iter()
            .filter(|(_, on)| **on)
            .map(|(k, _)| k.clone())
            .collect()
    }

    fn untracked_from(&self, active: &BTreeMap<String, bool>) -> Untracked {
        if *active.get("--all").unwrap_or(&false) {
            Untracked::All
        } else if *active.get("--include-untracked").unwrap_or(&false) {
            Untracked::Include
        } else {
            Untracked::No
        }
    }

    #[allow(clippy::too_many_lines)]
    fn run_command(&mut self, cmd: &str, active: &BTreeMap<String, bool>) -> Result<()> {
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
            "quit" => self.quit = true,
            "toggle-section" => self.toggle_section(),
            "visit" => self.visit_thing()?,
            "jump" => self.jump_section(),
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
            "stage" => {
                if let Some((_, path)) = self.pick_current_file_or_section() {
                    g!().stage(&path)?;
                    self.set_msg("staged");
                    self.refresh();
                } else {
                    self.set_msg("nothing to stage at point");
                }
            }
            "stage-all" => {
                g!().stage_all()?;
                self.set_msg("staged all modified");
                self.refresh();
            }
            "unstage" => {
                if let Some((_, path)) = self.pick_current_file_or_section_staged() {
                    g!().unstage(&path)?;
                    self.set_msg("unstaged");
                    self.refresh();
                } else {
                    self.set_msg("nothing staged at point");
                }
            }
            "unstage-all" => {
                g!().unstage_all()?;
                self.set_msg("unstaged all");
                self.refresh();
            }
            "discard" => self.discard_flow()?,
            "untrack" => {
                if let Some((_, path)) = self.pick_current_file_or_section() {
                    g!().untrack(&path)?;
                    self.set_msg("untracked");
                    self.refresh();
                }
            }

            // -------- commit menu
            "commit-create" => {
                let msg = self.read_line_prompt("Commit message", None)?;
                g!().commit(msg.as_deref(), &args)?;
                self.set_msg("committed");
                self.refresh();
            }
            "commit-extend" => {
                g!().commit_extend(&args)?;
                self.set_msg("extended HEAD commit");
                self.refresh();
            }
            "commit-amend" => {
                let msg = self.read_line_prompt("Amend message (empty keeps)", Some(""))?;
                let m = msg.filter(|s| !s.is_empty());
                g!().commit_amend(m.as_deref(), &args)?;
                self.set_msg("amended HEAD commit");
                self.refresh();
            }
            "commit-reword" => {
                let msg = self.read_line_prompt("New message", None)?;
                if let Some(m) = msg {
                    g!().commit_reword(&m)?;
                    self.set_msg("reworded HEAD commit");
                    self.refresh();
                }
            }
            "commit-fixup" => {
                let branches = g!().branches();
                if let Some(rev) = self.pick("Fixup commit (branch or hash)", &branches, None)? {
                    g!().commit_fixup(rev)?;
                    self.set_msg("created fixup commit");
                    self.refresh();
                }
            }
            "commit-squash" => {
                let branches = g!().branches();
                if let Some(rev) = self.pick("Squash into (branch or hash)", &branches, None)? {
                    g!().commit_squash(rev)?;
                    self.set_msg("created squash commit");
                    self.refresh();
                }
            }
            "commit-instant-fixup" => {
                let branches = g!().branches();
                if let Some(rev) = self.pick("Instant fixup into", &branches, None)? {
                    g!().commit_instant_fixup(rev)?;
                    self.set_msg("instant fixup done");
                    self.refresh();
                }
            }
            "commit-instant-squash" => {
                let branches = g!().branches();
                if let Some(rev) = self.pick("Instant squash into", &branches, None)? {
                    g!().commit_instant_squash(rev)?;
                    self.set_msg("instant squash done");
                    self.refresh();
                }
            }

            // -------- branch menu
            "checkout" | "branch-checkout" => {
                let bs = g!().branches();
                if let Some(b) = self.pick("Checkout branch/revision", &bs, None)? {
                    g!().checkout(b)?;
                    self.set_msg(&format!("checked out {b}"));
                    self.refresh();
                }
            }
            "branch-and-checkout" => {
                let Some(name) = self.read_line_prompt("New branch name", None)? else {
                    return Ok(());
                };
                let bs = g!().branches();
                let start = self.pick("Start from (empty = HEAD)", &bs, Some(""))?;
                g!().branch_and_checkout(&name, start.map(|s| s.as_str()))?;
                self.set_msg(&format!("created and checked out {name}"));
                self.refresh();
            }
            "branch-create" => {
                let Some(name) = self.read_line_prompt("New branch name", None)? else {
                    return Ok(());
                };
                let bs = g!().branches();
                let start = self.pick("Start from (empty = HEAD)", &bs, Some(""))?;
                g!().branch_create(&name, start.map(|s| s.as_str()))?;
                self.set_msg(&format!("created branch {name}"));
                self.refresh();
            }
            "branch-spinoff" => {
                let Some(name) = self.read_line_prompt("Spin off to branch", None)? else {
                    return Ok(());
                };
                g!().branch_spinoff(&name)?;
                self.set_msg(&format!("spin-off {name}"));
                self.refresh();
            }
            "branch-spinout" => {
                let Some(name) = self.read_line_prompt("Spin out branch", None)? else {
                    return Ok(());
                };
                g!().branch_spinout(&name)?;
                self.set_msg(&format!("spin-out {name}"));
                self.refresh();
            }
            "branch-rename" => {
                let bs = g!().branches();
                if let Some(old) = self.pick("Rename branch", &bs, None)? {
                    if let Some(new) = self.read_line_prompt("New name", Some(old))? {
                        g!().branch_rename(old, &new)?;
                        self.set_msg("renamed");
                        self.refresh();
                    }
                }
            }
            "branch-reset" => {
                let bs = g!().branches();
                if let Some(b) = self.pick("Reset branch", &bs, None)? {
                    if let Some(rev) = self.read_line_prompt("Reset to", Some("HEAD"))? {
                        g!().git_interactive(&["branch", "-f", b, &rev])?;
                        if g!().current_branch().as_deref() == Some(b.as_str()) {
                            let _ = g!().git_interactive(&["reset", "--keep", &rev]);
                        }
                        self.set_msg("branch reset");
                        self.refresh();
                    }
                }
            }
            "branch-delete" => {
                let bs = g!().branches();
                if let Some(b) = self.pick("Delete branch", &bs, None)? {
                    g!().branch_delete(b)?;
                    self.set_msg(&format!("deleted {b}"));
                    self.refresh();
                }
            }

            // -------- remote menu
            "remote-add" => {
                let Some(name) = self.read_line_prompt("Remote name", None)? else {
                    return Ok(());
                };
                let Some(url) = self.read_line_prompt("Remote url", None)? else {
                    return Ok(());
                };
                let fetch = *active.get("-f").unwrap_or(&false);
                g!().remote_add(&name, &url, fetch)?;
                self.set_msg(&format!("added remote {name}"));
                self.refresh();
            }
            "remote-rename" => {
                let rs = g!().remotes();
                if let Some(old) = self.pick("Rename remote", &rs, None)? {
                    if let Some(new) = self.read_line_prompt("New name", Some(old))? {
                        g!().remote_rename(old, &new)?;
                        self.set_msg("renamed remote");
                        self.refresh();
                    }
                }
            }
            "remote-remove" => {
                let rs = g!().remotes();
                if let Some(r) = self.pick("Remove remote", &rs, None)? {
                    g!().remote_remove(r)?;
                    self.set_msg("removed remote");
                    self.refresh();
                }
            }
            "remote-prune" => {
                let rs = g!().remotes();
                if let Some(r) = self.pick("Prune remote", &rs, None)? {
                    g!().remote_prune(r)?;
                    self.set_msg("pruned");
                    self.refresh();
                }
            }

            // -------- fetch/pull/push (upstream = branch.<b>.remote / branch.<b>.merge)
            "fetch-upstream" => {
                match g!().fetch_upstream(&args) {
                    Ok(r) => self.set_msg(&format!("fetched {r}")),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "fetch-all" => {
                let prune = args.iter().any(|a| a == "--prune");
                g!().fetch_all(prune)?;
                self.set_msg("fetched all remotes");
                self.refresh();
            }
            "fetch-other" | "fetch-branch" => {
                let remotes = g!().remotes();
                let Some(r) = self.pick("Fetch from remote", &remotes, None)?.cloned() else {
                    return Ok(());
                };
                if cmd == "fetch-other" {
                    g!().fetch(&r, &args)?;
                    self.set_msg(&format!("fetched {r}"));
                } else {
                    let Some(b) = self.read_line_prompt(&format!("Fetch branch from {r}"), None)? else {
                        return Ok(());
                    };
                    g!().fetch_branch(&r, &b, &args)?;
                    self.set_msg(&format!("fetched {r} {b}"));
                }
                self.refresh();
            }
            "pull-upstream" => {
                match g!().pull_upstream(&args) {
                    Ok(()) => self.set_msg("pulled"),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "pull-other" => {
                let mut choices = g!().remote_branches();
                choices.extend(g!().branches());
                let Some(src) = self.pick("Pull from", &choices, None)?.cloned() else {
                    return Ok(());
                };
                let (remote, branch) = g!().tracked_of(&src);
                g!().pull(&remote, &branch, &args)?;
                self.set_msg(&format!("pulled {src}"));
                self.refresh();
            }
            "push-upstream" => {
                match g!().push_upstream(&args) {
                    Ok(()) => self.set_msg("pushed"),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "push-other" | "push-other-branch" => {
                let src = if cmd == "push-other" {
                    match g!().current_branch() {
                        Some(b) => b,
                        None => {
                            self.set_msg("cannot push: detached HEAD");
                            return Ok(());
                        }
                    }
                } else {
                    let bs = g!().branches();
                    let Some(b) = self.pick("Push branch", &bs, None)?.cloned() else {
                        return Ok(());
                    };
                    b
                };
                let default = g!().remotes().first().map(|r| format!("{r}/{src}"));
                let Some(target) =
                    self.read_line_prompt(&format!("Push {src} to (remote/branch)"), default.as_deref())?
                else {
                    return Ok(());
                };
                let (remote, dst) = g!().tracked_of(&target);
                if remote == "." {
                    self.set_msg(&format!("not a remote branch: {target}"));
                    return Ok(());
                }
                g!().push_refspec(&remote, &src, &dst, &args)?;
                self.set_msg(&format!("pushed {src} to {target}"));
                self.refresh();
            }
            "push-tag" => {
                let remotes = g!().remotes();
                let tags = g!().tags();
                if let (Some(r), Some(t)) = (
                    self.pick("Push tag to remote", &remotes, None)?,
                    self.pick("Tag", &tags, None)?,
                ) {
                    g!().push_tag(r, t)?;
                    self.set_msg("tag pushed");
                }
                self.refresh();
            }
            "push-pushremote" => {
                match g!().push_pushremote(&args) {
                    Ok(r) => self.set_msg(&format!("pushed to {r}")),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "push-tags" | "push-matching" | "push-refspecs" => {
                let remotes = g!().remotes();
                let default = g!()
                    .current_branch()
                    .and_then(|b| g!().upstream_of(&b))
                    .map(|(r, _)| r)
                    .filter(|r| r != ".");
                let Some(remote) = self.pick("Push to remote", &remotes, default.as_deref())?.cloned() else {
                    return Ok(());
                };
                let res = match cmd {
                    "push-tags" => g!().push_tags(&remote, &args),
                    "push-matching" => g!().push_matching(&remote, &args),
                    _ => {
                        let Some(specs) = self.read_line_prompt("Refspecs (space separated)", None)? else {
                            return Ok(());
                        };
                        let specs: Vec<String> = specs.split_whitespace().map(str::to_string).collect();
                        g!().push_refspecs(&remote, &specs, &args)
                    }
                };
                match res {
                    Ok(()) => self.set_msg(&format!("pushed to {remote}")),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }

            // -------- merge / rebase
            "merge-plain" | "merge-nocommit" | "merge-squash" | "merge-editmsg" | "merge-preview" => {
                let mut bs = g!().branches();
                bs.extend(g!().remote_branches());
                let Some(rev) = self.pick("Merge revision", &bs, None)?.cloned() else {
                    return Ok(());
                };
                match cmd {
                    "merge-plain" => g!().merge(&rev, &args)?,
                    "merge-nocommit" => g!().merge_nocommit(&rev)?,
                    "merge-squash" => g!().merge_squash(&rev)?,
                    "merge-editmsg" => {
                        let into = g!().current_branch().unwrap_or_else(|| "HEAD".into());
                        let default = format!("Merge branch '{rev}' into {into}");
                        let Some(msg) = self.read_line_prompt("Merge message", Some(&default))? else {
                            return Ok(());
                        };
                        g!().merge_with_message(&rev, &msg, &args)?;
                    }
                    _ => {
                        match g!().merge_preview(&rev) {
                            Ok(p) if p.trim().is_empty() => self.set_msg("merge would not change anything"),
                            Ok(p) => self.view_text = Some((format!("preview merge of {rev}"), p)),
                            Err(e) => self.set_msg(&format!("{e}")),
                        }
                        return Ok(());
                    }
                }
                self.set_msg("merged");
                self.refresh();
            }
            "merge-commit" => {
                match g!().merge_commit() {
                    Ok(()) => self.set_msg("merge committed"),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "merge-abort" => {
                g!().merge_abort()?;
                self.set_msg("merge aborted");
                self.refresh();
            }
            "rebase-pushremote" => {
                match g!().rebase_pushremote(&args) {
                    Ok(t) => self.set_msg(&format!("rebased onto {t}")),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "rebase-autosquash" => {
                let default = g!().autosquash_base();
                let Some(base) =
                    self.read_line_prompt("Autosquash commits after", default.as_deref().or(Some("HEAD~10")))?
                else {
                    return Ok(());
                };
                match g!().rebase_autosquash_with(&base, &args) {
                    Ok(()) => self.set_msg("autosquashed"),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "rebase-upstream" | "rebase-other" => {
                let bs = g!().branches();
                if cmd == "rebase-upstream" {
                    let branch = g!().current_branch().unwrap_or_else(|| "HEAD".into());
                    let target = format!("{branch}@{{u}}");
                    if g!().rev_parse(&target).is_ok() {
                        g!().rebase(&target, &args)?;
                        self.set_msg("rebased onto upstream");
                    } else {
                        self.set_msg("no upstream configured");
                    }
                } else if let Some(onto) = self.pick("Rebase onto", &bs, None)? {
                    g!().rebase(onto, &args)?;
                    self.set_msg("rebased");
                }
                self.refresh();
            }
            "rebase-continue" => {
                g!().rebase_continue()?;
                self.set_msg("rebase continued");
                self.refresh();
            }
            "rebase-skip" => {
                g!().rebase_skip()?;
                self.set_msg("rebase skipped");
                self.refresh();
            }
            "rebase-abort" => {
                g!().rebase_abort()?;
                self.set_msg("rebase aborted");
                self.refresh();
            }

            // -------- reset
            "reset-mixed" | "reset-soft" | "reset-hard" | "reset-keep" | "reset-index"
            | "reset-worktree" | "reset-quickly" => {
                let bs = g!().branches();
                let rev = self
                    .pick("Reset to (branch/hash)", &bs, Some("HEAD"))?
                    .cloned()
                    .unwrap_or_else(|| "HEAD".into());
                let hard_confirm = cmd == "reset-hard" || cmd == "reset-quickly";
                if hard_confirm && !self.yes_no(&format!("Discard uncommitted changes and reset to {rev}?"))? {
                    self.set_msg("aborted");
                    return Ok(());
                }
                match cmd {
                    "reset-mixed" => g!().reset_mixed(&rev)?,
                    "reset-soft" => g!().reset_soft(&rev)?,
                    "reset-hard" => g!().reset_hard(&rev)?,
                    "reset-keep" => g!().reset_keep(&rev)?,
                    "reset-index" => g!().reset_index(&rev)?,
                    "reset-worktree" => g!().reset_worktree(&rev)?,
                    "reset-quickly" => g!().reset_mixed(&rev)?, // Magit default without prefix arg
                    _ => unreachable!(),
                }
                self.set_msg("reset");
                self.refresh();
            }

            // -------- tag
            "tag-create" => {
                let annotate = args.iter().any(|a| a == "--annotate");
                let Some(name) = self.read_line_prompt("Tag name", None)? else {
                    return Ok(());
                };
                let rev = self
                    .read_line_prompt("Place tag on", Some("HEAD"))?
                    .unwrap_or_else(|| "HEAD".into());
                let msg = if annotate {
                    self.read_line_prompt("Tag message", Some(&name))?
                } else {
                    None
                };
                g!().tag_create(&name, &rev, annotate, msg.as_deref())?;
                self.set_msg(&format!("tagged {name}"));
                self.refresh();
            }
            "tag-delete" => {
                let tags = g!().tags();
                if let Some(t) = self.pick("Delete tag", &tags, None)? {
                    g!().tag_delete(t)?;
                    self.set_msg("tag deleted");
                    self.refresh();
                }
            }

            // -------- revert / cherry-pick
            "revert" | "revert-no-commit" => {
                let Some(r) = self.pick_recent_commit("Revert commit")? else {
                    return Ok(());
                };
                let no_commit = cmd == "revert-no-commit";
                g!().revert(&[r.clone()], &args, no_commit)?;
                self.set_msg(if no_commit {
                    "reverted changes in the working tree"
                } else {
                    "reverted"
                });
                self.refresh();
            }
            "sequencer-continue" | "sequencer-skip" | "sequencer-abort" => {
                let action = cmd.trim_start_matches("sequencer-");
                match g!().sequencer(action) {
                    Ok(()) => self.set_msg(&format!("sequencer {action}")),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            // magit-apply: apply the thing at point to the working tree
            "apply" => {
                let line = self.current_line().map(|l| l.kind.clone());
                match line {
                    Some(LineKind::Stash { name, .. }) => {
                        g!().stash_apply(&name, true)?;
                        self.set_msg(&format!("applied {name}"));
                        self.refresh();
                    }
                    Some(LineKind::File { section, .. })
                    | Some(LineKind::SectionHeader { section, .. })
                        if section == "staged" || section == "unstaged" =>
                    {
                        self.set_msg("Change is already in the working tree");
                    }
                    Some(LineKind::File { section, .. })
                    | Some(LineKind::SectionHeader { section, .. })
                        if section == "untracked" =>
                    {
                        self.set_msg("Cannot apply untracked files (use w w for patch files)");
                    }
                    _ => {
                        // no commit sections in the status buffer: prompt like magit-cherry-apply
                        let Some(r) = self.pick_recent_commit("Apply changes from commit")? else {
                            return Ok(());
                        };
                        g!().cherry_apply(&[r.clone()], &args)?;
                        self.set_msg(&format!("applied {r} to the working tree"));
                        self.refresh();
                    }
                }
            }
            // magit-reverse: reverse the change at point in the working tree
            "reverse" => {
                let line = self.current_line().map(|l| l.kind.clone());
                match line {
                    Some(LineKind::File { section, path }) if section == "staged" => {
                        g!().reverse_staged(&[path.clone()])?;
                        self.set_msg(&format!("reversed {path} in the working tree"));
                        self.refresh();
                    }
                    Some(LineKind::SectionHeader { section, .. }) if section == "staged" => {
                        g!().reverse_staged(&[])?;
                        self.set_msg("reversed staged changes in the working tree");
                        self.refresh();
                    }
                    Some(LineKind::File { section, .. })
                    | Some(LineKind::SectionHeader { section, .. })
                        if section == "unstaged" || section == "untracked" =>
                    {
                        self.set_msg(&format!("Cannot reverse {section} changes"));
                    }
                    _ => {
                        // magit-revert-no-commit for a commit
                        let Some(r) = self.pick_recent_commit("Reverse changes from commit")? else {
                            return Ok(());
                        };
                        g!().revert(&[r.clone()], &[], true)?;
                        self.set_msg(&format!("reversed {r} in the working tree"));
                        self.refresh();
                    }
                }
            }
            "cherry-pick" | "cherry-apply" => {
                let log = g!().log(None, 30)?;
                let revs: Vec<String> = log
                    .iter()
                    .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
                    .collect();
                if let Some(r) = self.pick("Cherry (hash)", &revs, None)? {
                    match cmd {
                        "cherry-pick" => g!().cherry_pick(&[r.clone()], &args)?,
                        _ => g!().cherry_apply(&[r.clone()], &args)?,
                    }
                    self.set_msg("done");
                    self.refresh();
                }
            }

            // -------- stash
            "stash-both" | "stash-index" | "stash-worktree" | "stash-keep-index" => {
                let untracked = self.untracked_from(active);
                let msg = self.read_line_prompt("Stash message", Some("ngit stash"))?;
                match cmd {
                    "stash-both" => g!().stash_both(msg.as_deref(), untracked)?,
                    "stash-index" => g!().stash_index(msg.as_deref())?,
                    "stash-worktree" => g!().stash_worktree(msg.as_deref(), untracked)?,
                    "stash-keep-index" => g!().stash_keep_index(msg.as_deref(), untracked)?,
                    _ => unreachable!(),
                }
                self.set_msg("stashed");
                self.refresh();
            }
            "snapshot-both" | "snapshot-index" | "snapshot-worktree" => {
                let untracked = self.untracked_from(active);
                match cmd {
                    "snapshot-both" => g!().snapshot(crate::git::ops::SnapshotScope::Both, untracked)?,
                    "snapshot-index" => {
                        g!().snapshot(crate::git::ops::SnapshotScope::Index, untracked)?
                    }
                    _ => g!().snapshot(crate::git::ops::SnapshotScope::Worktree, untracked)?,
                }
                self.set_msg("snapshot saved");
                self.refresh();
            }
            "stash-apply" | "stash-pop" | "stash-drop" | "stash-show" | "stash-branch" => {
                let stashes = g!().stashes();
                if stashes.is_empty() {
                    self.set_msg("no stashes");
                    return Ok(());
                }
                let names: Vec<String> = stashes.iter().map(|(n, _)| n.clone()).collect();
                if let Some(s) = self.pick("Stash", &names, None)? {
                    match cmd {
                        "stash-apply" => g!().stash_apply(s, true)?,
                        "stash-pop" => g!().stash_pop(s, true)?,
                        "stash-drop" => {
                            if self.yes_no(&format!("Drop {s}?"))? {
                                g!().stash_drop(s)?;
                            } else {
                                return Ok(());
                            }
                        }
                        "stash-show" => {
                            let d = g!().stash_show(s)?;
                            self.view_text = Some((format!("stash {s}"), d));
                        }
                        "stash-branch" => {
                            if let Some(name) = self.read_line_prompt("Branch name", None)? {
                                g!().stash_branch(&name, s)?;
                            } else {
                                return Ok(());
                            }
                        }
                        _ => unreachable!(),
                    }
                    self.set_msg("done");
                    self.refresh();
                }
            }
            "stash-list" => {
                let l = g!().stash_list();
                if l.is_empty() {
                    self.set_msg("no stashes");
                } else {
                    self.view_text = Some(("stash list".into(), l.join("\n")));
                }
            }

            // -------- ignore / misc
            "ignore" => {
                let Some(pattern) = self.read_line_prompt("Pattern to ignore (top dir)", None)? else {
                    return Ok(());
                };
                g!().ignore(&pattern)?;
                self.set_msg("added to .gitignore");
                self.refresh();
            }
            "ignore-subdir" => {
                let default = self
                    .current_file()
                    .and_then(|(_, p)| std::path::Path::new(&p).parent().map(|d| d.display().to_string()))
                    .filter(|d| !d.is_empty());
                let Some(dir) = self.read_line_prompt("Subdirectory", default.as_deref())? else {
                    return Ok(());
                };
                let Some(pattern) = self.read_line_prompt(&format!("Pattern to ignore in {dir}"), None)? else {
                    return Ok(());
                };
                g!().ignore_in_subdir(&dir, &pattern)?;
                self.set_msg(&format!("added to {}/.gitignore", dir.trim_end_matches('/')));
                self.refresh();
            }
            "git-command" | "git-command-cwd" => {
                let Some(line) = self.read_line_prompt("git command (args after 'git')", Some("status"))? else {
                    return Ok(());
                };
                let parts: Vec<String> = line.split_whitespace().map(|s| s.to_string()).collect();
                let strs: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
                let out = if cmd == "git-command-cwd" {
                    g!().git_string_in(&self.cwd, &strs)
                } else {
                    g!().git_string(&strs)
                };
                match out {
                    Ok(o) => self.view_text = Some((format!("git {}", line), o)),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
                self.refresh();
            }
            "show-refs" => {
                let mut out = String::new();
                out.push_str(&format!("Branches:\n"));
                for b in g!().branches() {
                    let marker = if Some(b.clone()) == g!().current_branch() { "*" } else { " " };
                    out.push_str(&format!("  {marker} {b}\n"));
                }
                out.push_str("Tags:\n");
                for t in g!().tags() {
                    out.push_str(&format!("  {t}\n"));
                }
                self.view_text = Some(("refs".into(), out));
            }

            // -------- log / diff / show (arguments come from the l/d menus)
            "log-current" | "log-other" | "log-branches" | "log-all-branches" | "log-all" => {
                self.log_args = args.clone();
                self.log_view(cmd)?;
            }
            "log-refresh" => {
                self.log_args = args.clone();
                match self.last_view.clone() {
                    Some(v) if v.starts_with("log:") => self.log_view(&v[4..])?,
                    _ => self.set_msg(&format!("log arguments: {}", self.log_args.join(" "))),
                }
            }
            "reflog-other" | "reflog-head" => {
                let rev = if cmd == "reflog-head" {
                    "HEAD".to_string()
                } else {
                    let bs = g!().branches();
                    let Some(b) = self.pick("Reflog of branch", &bs, None)?.cloned() else {
                        return Ok(());
                    };
                    b
                };
                match g!().reflog(Some(&rev), 50) {
                    Ok(o) => self.view_text = Some((format!("reflog {rev}"), o)),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
            }
            "shortlog" => match g!().git_string(&["shortlog", "-s", "-n", "HEAD"]) {
                Ok(o) => self.view_text = Some(("shortlog".into(), o)),
                Err(e) => self.set_msg(&format!("{e}")),
            },
            "diff-unstaged" | "diff-staged" | "diff-worktree" | "diff-dwim" | "diff-range"
            | "show-commit" => {
                self.diff_args = args.clone();
                let kind = if cmd == "show-commit" { "commit" } else { &cmd[5..] };
                self.diff_view(kind)?;
            }
            "diff-refresh" => {
                self.diff_args = args.clone();
                match self.last_view.clone() {
                    Some(v) if v.starts_with("diff:") => self.diff_view(&v[5..])?,
                    _ => self.set_msg(&format!("diff arguments: {}", self.diff_args.join(" "))),
                }
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

            // -------- repo-level
            "init" => {
                let Some(path) = self.read_line_prompt("Initialize repository at", Some("."))? else {
                    return Ok(());
                };
                let abs = std::path::Path::new(&path)
                    .canonicalize()
                    .unwrap_or_else(|_| std::path::PathBuf::from(&path));
                g!().git_interactive_at(&abs, &["init"])?;
                self.set_msg(&format!("initialized {path}"));
            }
            "clone" => {
                let Some(url) = self.read_line_prompt("Repository url", None)? else {
                    return Ok(());
                };
                let target = self.read_line_prompt("Target directory", Some("."))?;
                let t = target.unwrap_or_else(|| ".".into());
                g!().git_interactive_at(&std::env::current_dir()?, &["clone", &url, &t])?;
                self.set_msg("cloned");
            }

            // All remaining commands are Magit extras
            _ => self.run_extra(cmd, &args)?,
        }
        Ok(())
    }

    // ------------------------------------------------------------ diff / log views

    /// Show a diff buffer using `self.diff_args` (magit-diff-arguments).
    fn diff_view(&mut self, kind: &str) -> Result<()> {
        let a = self.diff_args.clone();
        let (title, res, rerun): (String, Result<String>, bool) = match kind {
            "unstaged" => ("diff unstaged".into(), self.git.diff_with(&a, &[]), true),
            "staged" => ("diff staged".into(), self.git.diff_with(&a, &["--cached"]), true),
            "worktree" => ("diff worktree".into(), self.git.diff_with(&a, &["HEAD"]), true),
            "dwim" => match self.current_line().map(|l| l.kind.clone()) {
                Some(LineKind::File { section, path }) if section == "untracked" => (
                    format!("new file {path}"),
                    Ok(std::fs::read_to_string(self.git.toplevel.join(&path)).unwrap_or_default()),
                    false,
                ),
                Some(LineKind::File { section, path }) => {
                    let mut rest = vec![];
                    if section == "staged" {
                        rest.push("--cached");
                    }
                    rest.extend(["--", path.as_str()]);
                    (format!("diff {path}"), self.git.diff_with(&a, &rest), false)
                }
                Some(LineKind::SectionHeader { section, .. }) if section == "staged" => {
                    ("diff staged".into(), self.git.diff_with(&a, &["--cached"]), false)
                }
                Some(LineKind::Stash { name, .. }) => {
                    (format!("stash {name}"), self.git.stash_show(&name), false)
                }
                _ => ("diff unstaged".into(), self.git.diff_with(&a, &[]), false),
            },
            "range" => {
                let Some(from) = self.read_line_prompt("Diff from revision", Some("HEAD~1"))? else {
                    return Ok(());
                };
                let Some(to) = self.read_line_prompt("Diff to revision", Some("HEAD"))? else {
                    return Ok(());
                };
                (format!("{from}..{to}"), self.git.diff_with(&a, &[&from, &to]), false)
            }
            "commit" => {
                let Some(rev) = self.pick_recent_commit("Show commit")? else {
                    return Ok(());
                };
                (format!("commit {rev}"), self.git.show_with(&rev, &a), false)
            }
            "stash" => {
                let names: Vec<String> = self.git.stashes().into_iter().map(|(n, _)| n).collect();
                let Some(name) = self.pick("Show stash", &names, None)?.cloned() else {
                    return Ok(());
                };
                (format!("stash {name}"), self.git.stash_show(&name), false)
            }
            _ => return Ok(()),
        };
        match res {
            Ok(out) if out.trim().is_empty() => self.set_msg("(no differences)"),
            Ok(out) => {
                self.view_text = Some((title, out));
                self.last_view = rerun.then(|| format!("diff:{kind}"));
            }
            Err(e) => self.set_msg(&format!("{e}")),
        }
        Ok(())
    }

    /// Show a log buffer using `self.log_args` (magit-log-arguments, -n256).
    fn log_view(&mut self, cmd: &str) -> Result<()> {
        let mut a = self.log_args.clone();
        let title = match cmd {
            "log-current" => "log".to_string(),
            "log-other" => {
                let mut refs = self.git.branches();
                refs.extend(self.git.remote_branches());
                refs.extend(self.git.tags());
                let Some(r) = self.pick("Log rev", &refs, None)?.cloned() else {
                    return Ok(());
                };
                a.push(r.clone());
                format!("log {r}")
            }
            "log-branches" => {
                a.push("--branches".into());
                "log local branches".into()
            }
            "log-all-branches" => {
                a.extend(["--branches".into(), "--remotes".into()]);
                "log all branches".into()
            }
            "log-all" => {
                a.push("--all".into());
                "log all references".into()
            }
            _ => return Ok(()),
        };
        match self.git.log_with(None, 256, &a) {
            Ok(l) if l.is_empty() => self.set_msg("no commits"),
            Ok(l) => {
                self.view_text = Some((title, l.join("\n")));
                self.last_view = (cmd != "log-other").then(|| format!("log:{cmd}"));
            }
            Err(e) => self.set_msg(&format!("{e}")),
        }
        Ok(())
    }

    /// magit-section-up: move to the header of the section containing point.
    fn section_up(&mut self) {
        if let Some(i) = (0..self.cursor)
            .rev()
            .find(|&i| matches!(self.lines[i].kind, LineKind::SectionHeader { .. }))
        {
            self.cursor = i;
        }
    }

    // ------------------------------------------------------------ point helpers

    fn pick_current_file_or_section(&self) -> Option<(String, String)> {
        if let Some((section, path)) = self.current_file() {
            return Some((section, path));
        }
        let files = self.files_in_current_section();
        files.first().map(|f| (String::new(), f.clone()))
    }

    fn pick_current_file_or_section_staged(&self) -> Option<(String, String)> {
        if let Some((_, path)) = self.current_file() {
            return Some((String::new(), path));
        }
        let files = self.files_in_current_section();
        files.first().cloned().map(|f| (String::new(), f))
    }

    fn toggle_section(&mut self) {
        // collapse/expand: toggle between showing files and hiding them
        let Some(cur) = self.current_line() else { return };
        let section = match &cur.kind {
            LineKind::SectionHeader { section, .. } => section.clone(),
            LineKind::File { section, .. } => section.clone(),
            _ => return,
        };
        let files_visible = self
            .lines
            .iter()
            .any(|l| matches!(&l.kind, LineKind::File { section: s, .. } if *s == section));
        if files_visible {
            self.lines.retain(|l| {
                !matches!(&l.kind, LineKind::File { section: s, .. } if *s == section)
            });
        } else {
            self.refresh();
        }
        if self.cursor >= self.lines.len() {
            self.cursor = self.lines.len().saturating_sub(1);
        }
    }

    fn visit_thing(&mut self) -> Result<()> {
        let Some(cur) = self.current_line() else { return Ok(()) };
        match &cur.kind {
            LineKind::File { section, path } => match section.as_str() {
                "untracked" => {
                    let content = std::fs::read_to_string(self.git.toplevel.join(path))
                        .unwrap_or_else(|_| "(binary or unreadable)".into());
                    self.view_text = Some((path.clone(), content));
                }
                "unstaged" => {
                    let d = self.git.git_string(&["diff", "--no-color", "--", path])?;
                    self.view_text = Some((format!("diff {path}"), d));
                }
                "staged" => {
                    let d = self
                        .git
                        .git_string(&["diff", "--cached", "--no-color", "--", path])?;
                    self.view_text = Some((format!("diff --cached {path}"), d));
                }
                _ => {}
            },
            LineKind::SectionHeader { section, .. } => match section.as_str() {
                "unstaged" => {
                    let d = self.git.diff_unstaged()?;
                    self.view_text = Some(("diff unstaged".into(), d));
                }
                "staged" => {
                    let d = self.git.diff_staged()?;
                    self.view_text = Some(("diff staged".into(), d));
                }
                "untracked" => {}
                _ => {}
            },
            LineKind::Stash { name, .. } => {
                let d = self.git.stash_show(name)?;
                self.view_text = Some((format!("stash {name}"), d));
            }
            _ => {}
        }
        Ok(())
    }

    fn jump_section(&mut self) {
        // cycle through section headers
        let headers: Vec<usize> = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| match &l.kind {
                LineKind::SectionHeader { .. } | LineKind::Stash { .. } => Some(i),
                _ => None,
            })
            .collect();
        if headers.is_empty() {
            return;
        }
        let next = headers.iter().find(|&&i| i > self.cursor).unwrap_or(&headers[0]);
        self.cursor = *next;
    }

    fn discard_flow(&mut self) -> Result<()> {
        let Some(cur_kind) = self.current_line().map(|l| l.kind.clone()) else {
            return Ok(());
        };
        match cur_kind {
            LineKind::File { section, path } => {
                if !self.yes_no(&format!("Discard changes in {path}?"))? {
                    self.set_msg("aborted");
                    return Ok(());
                }
                match section.as_str() {
                    "untracked" => self.git.discard_untracked(&path)?,
                    "unstaged" => self.git.discard_unstaged(&path)?,
                    "staged" => self.git.discard_staged(&path)?,
                    _ => {}
                }
                self.set_msg("discarded");
                self.refresh();
            }
            LineKind::SectionHeader { section, n } => {
                if !self.yes_no(&format!("Discard all {n} changes ({section})?"))? {
                    self.set_msg("aborted");
                    return Ok(());
                }
                let files = self.files_in_current_section();
                for f in files {
                    match section.as_str() {
                        "untracked" => self.git.discard_untracked(&f)?,
                        "unstaged" => self.git.discard_unstaged(&f)?,
                        "staged" => self.git.discard_staged(&f)?,
                        _ => {}
                    }
                }
                self.set_msg("discarded all");
                self.refresh();
            }
            LineKind::Stash { name, .. } => {
                if self.yes_no(&format!("Drop {name}?"))? {
                    self.git.stash_drop(&name)?;
                    self.set_msg("stash dropped");
                    self.refresh();
                }
            }
            _ => self.set_msg("nothing to discard at point"),
        }
        Ok(())
    }
}

/// extra helper on Git for running outside repo (init/clone)
impl Git {
    fn git_interactive_at(&self, dir: &std::path::Path, args: &[&str]) -> Result<()> {
        let st = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()?;
        if !st.success() {
            anyhow::bail!("git {} failed: {}", args.first().unwrap_or(&""), st);
        }
        Ok(())
    }
}

/// Magit extras: bisect / worktree / submodule / notes / ediff.
/// Kept out of `run_command`'s main dispatch so each feature stays together.
impl App {
    fn show(&mut self, title: &str, res: Result<String>) {
        match res {
            Ok(out) if out.trim().is_empty() => self.set_msg("(no output)"),
            Ok(out) => self.view_text = Some((title.into(), out)),
            Err(e) => self.set_msg(&format!("{e}")),
        }
    }

    fn done(&mut self, ok_msg: &str, res: Result<String>) {
        match res {
            Ok(out) => {
                let first = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                self.set_msg(if first.is_empty() { ok_msg } else { first });
                self.refresh();
            }
            Err(e) => self.set_msg(&format!("{e}")),
        }
    }

    fn run_extra(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        match cmd {
            // ---- bisect
            "bisect-start" => {
                let Some(bad) = self.read_line_prompt("Bisect bad revision", Some("HEAD"))? else {
                    return Ok(());
                };
                let good = self.read_line_prompt("Bisect good revisions (space separated)", None)?;
                let goods: Vec<String> = good
                    .as_deref()
                    .unwrap_or("")
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                let res = self.git.bisect_start(Some(&bad), &goods);
                self.done("bisect started", res);
            }
            "bisect-bad" | "bisect-good" | "bisect-skip" => {
                let state = match cmd {
                    "bisect-bad" => "bad",
                    "bisect-good" => "good",
                    _ => "skip",
                };
                let rev = self.read_line_prompt(&format!("Bisect {state} revision"), Some("HEAD"))?;
                let res = self.git.bisect_mark(state, rev.as_deref());
                self.done(&format!("bisect {state}"), res);
            }
            "bisect-reset" => {
                let rev = self.read_line_prompt("Reset to revision (empty = pre-bisect HEAD)", None)?;
                let res = self.git.bisect_reset(rev.as_deref());
                self.done("bisect reset", res);
            }
            "bisect-log" => {
                let res = self.git.bisect_log();
                self.show("bisect log", res);
            }
            "bisect-run" | "bisect-start-run" => {
                // magit-bisect-run: when not bisecting yet, start first
                if cmd == "bisect-start-run" || !self.git.bisect_in_progress() {
                    let Some(bad) = self.read_line_prompt("Bisect bad revision", Some("HEAD"))? else {
                        return Ok(());
                    };
                    let Some(good) = self.read_line_prompt("Bisect good revision", None)? else {
                        return Ok(());
                    };
                    if let Err(e) = self.git.bisect_start(Some(&bad), &[good]) {
                        self.set_msg(&format!("{e}"));
                        return Ok(());
                    }
                }
                let Some(command) = self.read_line_prompt("Bisect run shell command", None)? else {
                    return Ok(());
                };
                let res = self.git.bisect_run(&command);
                self.show("bisect run", res);
                self.refresh();
            }

            // ---- worktree
            "worktree-list" => {
                let res = self.git.worktree_list();
                self.show("worktrees", res);
            }
            "worktree-checkout" | "worktree-branch" => {
                let Some(path) = self.read_line_prompt("Worktree path", None)? else {
                    return Ok(());
                };
                let is_new = cmd == "worktree-branch";
                let branch = self.read_line_prompt(
                    if is_new { "New branch name" } else { "Checkout branch/revision (empty = HEAD)" },
                    None,
                )?;
                let res = self.git.worktree_add(&path, branch.as_deref(), is_new, None);
                self.done("worktree added", res);
            }
            "worktree-move" => {
                let paths = self.git.worktree_paths();
                let Some(from) = self.pick("Move worktree", &paths, None)?.cloned() else {
                    return Ok(());
                };
                let Some(to) = self.read_line_prompt("New path", None)? else {
                    return Ok(());
                };
                let res = self.git.worktree_move(&from, &to);
                self.done("worktree moved", res);
            }
            "worktree-delete" => {
                let paths = self.git.worktree_paths();
                let Some(path) = self.pick("Delete worktree", &paths, None)?.cloned() else {
                    return Ok(());
                };
                if !self.yes_no(&format!("Delete worktree {path}?"))? {
                    return Ok(());
                }
                let force = self.yes_no("Force delete (dirty worktree)?")?;
                let res = self.git.worktree_remove(&path, force);
                self.done("worktree removed", res);
            }
            "worktree-prune" => {
                let res = self.git.worktree_prune();
                self.done("worktrees pruned", res);
            }

            // ---- submodule
            "submodule-list" => {
                let res = self.git.submodule_list();
                self.show("submodules", res);
            }
            "submodule-add" => {
                let Some(url) = self.read_line_prompt("Submodule url", None)? else {
                    return Ok(());
                };
                let Some(path) = self.read_line_prompt("Submodule path", None)? else {
                    return Ok(());
                };
                let res = self.git.submodule_add(&url, &path);
                self.done("submodule added", res);
            }
            // magit-submodule-register / populate / update / unpopulate:
            // act on one module or on all of them ("(all)")
            "submodule-register" | "submodule-populate" | "submodule-update" | "submodule-unpopulate" => {
                let Some(modules) = self.pick_submodules(cmd != "submodule-unpopulate")? else {
                    return Ok(());
                };
                let res = match cmd {
                    "submodule-register" => self.git.submodule_register(&modules),
                    "submodule-populate" => self.git.submodule_populate(args, &modules),
                    "submodule-update" => self.git.submodule_update_with(args, &modules),
                    _ => self.git.submodule_unpopulate(args, &modules),
                };
                self.done(&cmd.replace('-', " "), res);
            }
            "submodule-sync" => {
                let res = self.git.submodule_sync();
                self.done("submodules synced", res);
            }
            "submodule-fetch" => {
                let res = self.git.fetch_modules(args);
                self.done("fetched modules", res);
            }
            "submodule-remove" => {
                let paths = self.git.submodule_paths();
                let Some(path) = self.pick("Remove submodule", &paths, None)?.cloned() else {
                    return Ok(());
                };
                if !self.yes_no(&format!("Remove submodule {path}?"))? {
                    return Ok(());
                }
                let res = self.git.submodule_remove(&path);
                self.done("submodule removed", res);
            }

            // ---- notes
            "notes-list" => {
                let res = self.git.notes_list();
                self.show("notes", res);
            }
            "notes-edit" => {
                let Some(rev) = self.read_line_prompt("Note for revision", Some("HEAD"))? else {
                    return Ok(());
                };
                let Some(msg) = self.read_line_prompt("Note text (one line)", None)? else {
                    return Ok(());
                };
                let res = self.git.notes_add(&rev, &msg);
                self.done("note saved", res);
            }
            "notes-show" => {
                let Some(rev) = self.read_line_prompt("Show note for revision", Some("HEAD"))? else {
                    return Ok(());
                };
                let res = self.git.notes_show(&rev);
                self.show(&format!("notes {rev}"), res);
            }
            "notes-remove" => {
                let Some(rev) = self.read_line_prompt("Remove note from revision", Some("HEAD"))? else {
                    return Ok(());
                };
                if !self.yes_no(&format!("Remove note from {rev}?"))? {
                    return Ok(());
                }
                let res = self.git.notes_remove(&rev);
                self.done("note removed", res);
            }
            "notes-prune" => {
                let res = self.git.notes_prune();
                self.done("notes pruned", res);
            }

            // ---- ediff (text diff buffers instead of Emacs ediff sessions)
            "ediff-dwim" | "ediff-unstaged" | "ediff-staged" | "ediff-worktree" | "ediff-range"
            | "ediff-commit" | "ediff-stash" => {
                self.diff_view(&cmd["ediff-".len()..])?;
            }

            // ---- section info / buffer display
            "section-info" => {
                let line = self.current_line().cloned();
                let text = match line {
                    Some(l) => {
                        let section = match &l.kind {
                            LineKind::SectionHeader { section, .. }
                            | LineKind::File { section, .. } => section.clone(),
                            _ => "-".into(),
                        };
                        format!(
                            "Line: {}\nKind: {:?}\nSection: {}\nPosition: {}/{}",
                            l.text,
                            l.kind,
                            section,
                            self.cursor + 1,
                            self.lines.len(),
                        )
                    }
                    None => "No line at point".into(),
                };
                self.view_text = Some(("section info".into(), text));
            }
            "display-buffer" => {
                let mut out = format!("Status buffer ({} lines)\n\n", self.lines.len());
                for (i, l) in self.lines.iter().enumerate() {
                    out.push_str(&format!("{:>4}  {}\n", i + 1, l.text));
                }
                self.view_text = Some(("status buffer".into(), out));
            }
            "log-change" => {
                let path = match self.current_file() {
                    Some((_, p)) => p,
                    None => {
                        let Some(p) = self.read_line_prompt("Log file path", None)? else {
                            return Ok(());
                        };
                        p
                    }
                };
                let res = self.git.log_file(&path, 50);
                self.show(&format!("log {path}"), res);
            }
            "cherries" => {
                let default = self.git.upstream_shortname();
                let up = self.read_line_prompt("Cherry against upstream", default.as_deref())?;
                let res = self.git.cherries(up.as_deref(), None);
                self.show("cherries", res);
            }
            "reflog-current" => {
                let res = self.git.reflog(None, 50);
                self.show("reflog", res);
            }

            // ---- subtree
            "subtree-prefixes" => {
                let prefixes = self.git.subtree_prefixes();
                let text = if prefixes.is_empty() {
                    "No subtree prefixes found".into()
                } else {
                    prefixes.join("\n")
                };
                self.view_text = Some(("subtree prefixes".into(), text));
            }
            "subtree-add" | "subtree-pull" | "subtree-push" => {
                let prefix = self.subtree_prefix_prompt()?;
                let Some(prefix) = prefix else { return Ok(()) };
                let Some(repo) = self.read_line_prompt("Subtree repository", None)? else {
                    return Ok(());
                };
                let Some(rev) = self.read_line_prompt("Subtree revision", Some("HEAD"))? else {
                    return Ok(());
                };
                let res = match cmd {
                    "subtree-add" => self.git.subtree_add(&prefix, &repo, &rev),
                    "subtree-pull" => self.git.subtree_pull(&prefix, &repo, &rev),
                    _ => self.git.subtree_push(&prefix, &repo, &rev),
                };
                self.done("subtree updated", res);
            }
            "subtree-split" => {
                let Some(prefix) = self.subtree_prefix_prompt()? else { return Ok(()) };
                let Some(branch) = self.read_line_prompt("Split into branch", None)? else {
                    return Ok(());
                };
                let res = self.git.subtree_split(&prefix, &branch);
                self.done("subtree split", res);
            }

            // ---- patches
            "patches-format" => {
                let Some(range) =
                    self.read_line_prompt("Format patches for range", Some("HEAD~3..HEAD"))?
                else {
                    return Ok(());
                };
                let Some(dir) = self.read_line_prompt("Output directory", Some("patches"))? else {
                    return Ok(());
                };
                let res = self.git.patch_format(&range, &dir);
                self.show("format-patch", res);
            }
            "patches-apply" | "patches-apply-worktree" => {
                let Some(path) = self.read_line_prompt("Patch file(s), space separated", None)? else {
                    return Ok(());
                };
                let files: Vec<String> = path.split_whitespace().map(str::to_string).collect();
                let res = if cmd == "patches-apply" {
                    self.git.am_apply(&files, args)
                } else {
                    match files.as_slice() {
                        [one] => self.git.patch_apply_worktree(one),
                        _ => Err(anyhow::anyhow!("git apply takes exactly one patch file")),
                    }
                };
                self.done("patch applied", res);
            }
            "patches-abort" | "patches-continue" | "patches-skip" | "patches-show" => {
                let res = match cmd {
                    "patches-abort" => self.git.patch_am_abort(),
                    "patches-continue" => self.git.patch_am_continue(),
                    "patches-skip" => self.git.am_skip(),
                    _ => self.git.patch_am_show(),
                };
                if cmd == "patches-show" {
                    self.show("current patch", res);
                } else {
                    self.done("git am", res);
                }
            }
            // magit-patch-save: write the diff being viewed (or all changes) to a file
            "patch-save" => {
                let diff = match &self.view_text {
                    Some((title, body))
                        if title.starts_with("diff") || title.starts_with("commit") || title.starts_with("stash") =>
                    {
                        body.clone()
                    }
                    _ => self.git.diff_with(&self.diff_args, &["HEAD"])?,
                };
                if diff.trim().is_empty() {
                    self.set_msg("no changes to save");
                    return Ok(());
                }
                let Some(file) = self.read_line_prompt("Save patch as", Some("changes.patch"))? else {
                    return Ok(());
                };
                let path = self.git.toplevel.join(&file);
                std::fs::write(&path, format!("{}\n", diff.trim_end_matches('\n')))?;
                self.set_msg(&format!("saved {}", path.display()));
            }
            "request-pull" => {
                let default_start = self.git.upstream_shortname();
                let Some(start) = self.read_line_prompt("Start (first commit not to include)", default_start.as_deref())? else {
                    return Ok(());
                };
                let default_url = self
                    .git
                    .remotes()
                    .first()
                    .and_then(|r| self.git.git_string(&["remote", "get-url", r]).ok());
                let Some(url) = self.read_line_prompt("Public repository url", default_url.as_deref())? else {
                    return Ok(());
                };
                let end = self.read_line_prompt("End (default HEAD)", None)?;
                let res = self.git.request_pull(&start, &url, end.as_deref());
                self.show("request-pull", res);
            }

            // ---- branch configuration
            "branch-config-show" => {
                let res = self.git.branch_config();
                self.show("branch configuration", res);
            }
            "branch-config-upstream" => {
                let default = self.git.upstream_shortname();
                let Some(up) = self.read_line_prompt("Upstream branch", default.as_deref())? else {
                    return Ok(());
                };
                let res = self.git.branch_set_upstream(&up);
                self.done("upstream set", res);
            }
            "branch-config-unset-upstream" => {
                let res = self.git.branch_unset_upstream();
                self.done("upstream unset", res);
            }
            // magit-branch.<branch>.rebase / magit-pull.rebase:
            // cycle true -> merges -> interactive -> false -> unset
            "branch-config-rebase" | "pull-rebase" => {
                let key = if cmd == "pull-rebase" {
                    "pull.rebase".to_string()
                } else {
                    match self.git.current_branch() {
                        Some(b) => format!("branch.{b}.rebase"),
                        None => {
                            self.set_msg("cannot configure a branch on detached HEAD");
                            return Ok(());
                        }
                    }
                };
                match self.git.cycle_config(&key, REBASE_CHOICES) {
                    Ok(v) => self.set_msg(&format!("{key} = {}", v.as_deref().unwrap_or("(unset)"))),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
            }
            // magit-branch.autoSetupMerge / magit-branch.autoSetupRebase ("b C a m" / "a r")
            "branch-autosetup-merge" | "branch-autosetup-rebase" => {
                let (key, choices): (&str, &[&str]) = if cmd == "branch-autosetup-merge" {
                    ("branch.autoSetupMerge", AUTOSETUP_MERGE_CHOICES)
                } else {
                    ("branch.autoSetupRebase", AUTOSETUP_REBASE_CHOICES)
                };
                match self.git.cycle_config(key, choices) {
                    Ok(v) => self.set_msg(&format!("{key} = {}", v.as_deref().unwrap_or("(unset)"))),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
            }
            "update-default-branch" => self.update_default_branch()?,
            "patches-apply-maildir" => {
                let Some(p) = self.read_line_prompt("Apply mbox or Maildir", None)? else {
                    return Ok(());
                };
                let path = self.cwd.join(&p);
                let res = self.git.am_apply(&[path.display().to_string()], args);
                self.done("patches applied", res);
            }
            "merge-absorb" | "merge-dissolve" => self.merge_absorb_cmd(cmd, args)?,
            "cherry-harvest" | "cherry-donate" | "cherry-spinout" | "cherry-spinoff" => {
                self.cherry_move_cmd(cmd, args)?
            }
            "rebase-interactive" | "rebase-edit-commit" | "rebase-reword-commit" | "rebase-remove-commit" => {
                let prompt = match cmd {
                    "rebase-interactive" => "Rebase this commit and all commits above it",
                    "rebase-edit-commit" => "Edit commit",
                    "rebase-reword-commit" => "Reword commit",
                    _ => "Remove commit",
                };
                let Some(commit) = self.pick_recent_commit(prompt)? else {
                    return Ok(());
                };
                // magit-rebase-remove-commit passes NOASSERT
                if cmd != "rebase-remove-commit"
                    && !self.rebase_assert(&commit, cmd != "rebase-reword-commit", args)?
                {
                    return Ok(());
                }
                let res = match cmd {
                    "rebase-interactive" => {
                        self.with_terminal(|g| g.rebase_interactive(&commit, args, Editor::User, Editor::User))
                    }
                    "rebase-edit-commit" => self.git.rebase_edit_commit(&commit, args),
                    "rebase-reword-commit" => {
                        self.with_terminal(|g| g.rebase_reword_commit(&commit, args, Editor::User))
                    }
                    _ => self.git.rebase_remove_commit(&commit, args),
                };
                self.after_rebase(res);
            }
            "rebase-subset" => {
                let default = self.git.upstream_shortname();
                let Some(newbase) = self.read_line_prompt("Rebase subset onto", default.as_deref())? else {
                    return Ok(());
                };
                let Some(start) =
                    self.pick_recent_commit(&format!("Rebase this commit and the commits above it onto {newbase}"))?
                else {
                    return Ok(());
                };
                let res = self.git.rebase_subset(&newbase, &format!("{start}^"), args);
                self.after_rebase(res);
            }
            "rebase-edit-todo" => {
                let res = self.with_terminal(|g| g.rebase_edit_todo(Editor::User));
                self.after_rebase(res);
            }
            "remote-pushdefault" => {
                let mut choices = self.git.remotes();
                choices.push("(unset)".into());
                let Some(r) = self.pick("remote.pushDefault", &choices, None)?.cloned() else {
                    return Ok(());
                };
                let value = (r != "(unset)").then_some(r.as_str());
                match self.git.set_config("remote.pushDefault", value) {
                    Ok(()) => self.set_msg(&format!("remote.pushDefault = {}", value.unwrap_or("(unset)"))),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
            }
            "branch-config-description" | "branch-config-pushremote" => {
                let (key, prompt) = if cmd == "branch-config-description" {
                    ("description", "Branch description")
                } else {
                    ("pushRemote", "pushRemote")
                };
                let Some(value) = self.read_line_prompt(prompt, None)? else {
                    return Ok(());
                };
                let res = self.git.branch_set_config(key, &value);
                self.done("branch configured", res);
            }

            _ => self.set_msg(&format!("unknown command: {cmd}")),
        }
        Ok(())
    }

    /// Pick a submodule, or "(all)" (empty list = every module) when allowed.
    fn pick_submodules(&mut self, allow_all: bool) -> Result<Option<Vec<String>>> {
        let mut choices = Vec::new();
        if allow_all {
            choices.push("(all)".to_string());
        }
        choices.extend(self.git.submodule_paths());
        if choices.is_empty() || (allow_all && choices.len() == 1) {
            self.set_msg("no submodules");
            return Ok(None);
        }
        Ok(self
            .pick("Submodule", &choices, None)?
            .cloned()
            .map(|c| if c == "(all)" { vec![] } else { vec![c] }))
    }

    /// Report the outcome of a rebase that may stop half-way.
    fn after_rebase(&mut self, res: Result<()>) {
        match res {
            Ok(()) if self.git.rebase_in_progress() => {
                self.set_msg("rebase stopped (r r continue, r s skip, r a abort)")
            }
            Ok(()) => self.set_msg("rebased"),
            Err(e) => self.set_msg(&format!("{e}")),
        }
        self.refresh();
    }

    /// magit-rebase-interactive-assert: confirm rewriting published commits
    /// and rebasing a range that contains merges. With `delay_edit_confirm`,
    /// do not ask when the published branches point exactly at COMMIT.
    fn rebase_assert(&mut self, commit: &str, delay_edit_confirm: bool, args: &[String]) -> Result<bool> {
        let since = match self.git.rebase_since(commit) {
            Ok(s) => s,
            Err(e) => {
                self.set_msg(&format!("{e}"));
                return Ok(false);
            }
        };
        let target = self.git.rev_parse(commit)?;
        let published: Vec<&str> = PUBLISHED_BRANCHES
            .iter()
            .copied()
            .filter(|b| self.git.rev_verify(b) && self.git.is_ancestor(commit, b))
            .collect();
        let all_at_commit = published
            .iter()
            .all(|b| self.git.rev_parse(b).ok().as_deref() == Some(target.as_str()));
        if !published.is_empty()
            && !(delay_edit_confirm && all_at_commit)
            && !self.yes_no(&format!(
                "Some of these commits have already been published to {}. Do you really want to modify them?",
                published.join(", ")
            ))?
        {
            self.set_msg("Abort");
            return Ok(false);
        }
        if !args.iter().any(|a| a.starts_with("--rebase-merges")) {
            let range = format!("{}..HEAD", since.as_deref().unwrap_or(commit));
            let merges = self.git.git_string(&["rev-list", "--merges", &range]).unwrap_or_default();
            if !merges.trim().is_empty() && !self.yes_no("Proceed despite merge in rebase range?")? {
                self.set_msg("Abort");
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// magit-merge-absorb ("m a") / magit-merge-dissolve ("m d")
    fn merge_absorb_cmd(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let current = self.git.current_branch();
        let others: Vec<String> = self
            .git
            .branches()
            .into_iter()
            .filter(|b| Some(b) != current.as_ref())
            .collect();
        let (absorbed, res) = if cmd == "merge-absorb" {
            let Some(branch) = self.pick("Absorb branch", &others, None)?.cloned() else {
                return Ok(());
            };
            if !self.confirm_absorb(&branch)? {
                return Ok(());
            }
            let res = self.git.merge_absorb(&branch, args);
            (branch, res)
        } else {
            let label = match &current {
                Some(c) => c.clone(),
                None => self.git.rev_parse("HEAD")?,
            };
            // default: the current branch's upstream when that is a local branch
            let default = current
                .as_ref()
                .and_then(|b| self.git.upstream_of(b))
                .filter(|(r, _)| r == ".")
                .map(|(_, b)| b);
            let Some(into) = self.pick(&format!("Merge `{label}' into"), &others, default.as_deref())?.cloned()
            else {
                return Ok(());
            };
            if current.is_some() && !self.confirm_absorb(&label)? {
                return Ok(());
            }
            let res = self.git.merge_dissolve(&into, args);
            (label, res)
        };
        match res {
            Ok(()) => self.set_msg(&format!("merged and removed {absorbed}")),
            Err(e) => self.set_msg(&format!("{e}")),
        }
        self.refresh();
        Ok(())
    }

    /// magit--merge-absorb asks before merging the main branch into another one.
    fn confirm_absorb(&mut self, branch: &str) -> Result<bool> {
        if self.git.main_branch().as_deref() == Some(branch)
            && !self.yes_no(&format!("Do you really want to merge `{branch}' into another branch?"))?
        {
            self.set_msg("Abort");
            return Ok(false);
        }
        Ok(true)
    }

    /// magit-cherry-harvest / -donate / -spinout / -spinoff ("A h/d/n/s")
    fn cherry_move_cmd(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        let verb = &cmd["cherry-".len()..];
        let away = cmd != "cherry-harvest";
        let current = match self.git.current_branch() {
            Some(c) => c,
            None if cmd == "cherry-donate" => self.git.rev_parse("HEAD")?,
            None => {
                self.set_msg(&format!("Cannot {verb} cherries while HEAD is detached"));
                return Ok(());
            }
        };
        let Some(spec) = self.read_line_prompt(&format!("{verb} cherry (commit or A..B)"), None)? else {
            return Ok(());
        };
        let commits = match self.git.resolve_cherries(&spec) {
            Ok(c) => c,
            Err(e) => {
                self.set_msg(&format!("{e}"));
                return Ok(());
            }
        };
        let reachable = self.git.is_ancestor(&commits[0], &current);
        if away != reachable {
            let are = if reachable { "are" } else { "are not" };
            self.set_msg(&format!("Cannot {verb} cherries that {are} reachable from HEAD"));
            return Ok(());
        }
        let n = commits.len();
        let cherries = if n == 1 { "1 cherry".to_string() } else { format!("{n} cherries") };
        let res = match cmd {
            "cherry-harvest" => {
                let branches = self.git.branches_containing(&commits[0]);
                let src = match branches.len() {
                    0 => None,
                    1 => Some(branches[0].clone()),
                    _ => match self.pick(&format!("Remove {cherries} from branch"), &branches, None)? {
                        Some(b) => Some(b.clone()),
                        None => return Ok(()),
                    },
                };
                self.git.cherry_move(&commits, src.as_deref(), &current, args, None, true)
            }
            "cherry-donate" => {
                let others: Vec<String> =
                    self.git.branches().into_iter().filter(|b| *b != current).collect();
                let Some(dst) = self.pick(&format!("Move {cherries} to branch"), &others, None)?.cloned() else {
                    return Ok(());
                };
                self.git.cherry_move(&commits, Some(&current), &dst, args, None, false)
            }
            _ => {
                let Some(name) =
                    self.read_line_prompt(&format!("Create branch from {cherries} named"), None)?
                else {
                    return Ok(());
                };
                if self.git.local_branch_exists(&name) {
                    self.set_msg(&format!("branch {name} already exists"));
                    return Ok(());
                }
                let default_start = self.git.upstream_shortname().unwrap_or_else(|| current.clone());
                let Some(start) = self.read_line_prompt(
                    &format!("Create branch from {cherries} starting at"),
                    Some(&default_start),
                )?
                else {
                    return Ok(());
                };
                let checkout = cmd == "cherry-spinoff";
                self.git.cherry_move(&commits, Some(&current), &name, args, Some(&start), checkout)
            }
        };
        match res {
            Ok(()) => self.set_msg(&format!("{verb}: moved {cherries}")),
            Err(e) => self.set_msg(&format!("{e} (resolve, then A A to continue)")),
        }
        self.refresh();
        Ok(())
    }

    /// magit-update-default-branch ("b C B")
    fn update_default_branch(&mut self) -> Result<()> {
        let old = match self.git.default_branch(false) {
            Ok((_, o)) => o,
            Err(e) => {
                self.set_msg(&format!("{e}"));
                return Ok(());
            }
        };
        let (remote, new) = match self.git.default_branch(true) {
            Ok((r, Some(n))) => (r, n),
            Ok((_, None)) => {
                self.set_msg("Cannot determine new default branch");
                return Ok(());
            }
            Err(e) => {
                self.set_msg(&format!("{e}"));
                return Ok(());
            }
        };
        let old = if old.as_deref() == Some(new.as_str()) {
            let Some(o) = self.read_line_prompt(
                &format!(
                    "Name of default branch is still `{new}', but the upstreams of some local branches \
                     might need updating. Name of upstream branches to replace with `{new}'"
                ),
                None,
            )?
            else {
                return Ok(());
            };
            o
        } else {
            let old = match old {
                Some(o) => o,
                None => {
                    let others: Vec<String> = self.git.branches().into_iter().filter(|b| *b != new).collect();
                    let Some(o) = self
                        .pick(&format!("Name of old default branch to be renamed to `{new}'"), &others, Some("master"))?
                        .cloned()
                    else {
                        return Ok(());
                    };
                    o
                }
            };
            if !self.yes_no(&format!(
                "Default branch changed from `{old}' to `{new}' on {remote}.  Do the same locally?"
            ))? {
                self.set_msg("Abort");
                return Ok(());
            }
            old
        };
        match self.git.set_default_branch(&new, &old) {
            Ok(()) => self.set_msg(&format!("default branch is now {new}")),
            Err(e) => self.set_msg(&format!("{e}")),
        }
        self.refresh();
        Ok(())
    }

    /// Pick a known subtree prefix, or ask for one when none is recorded.
    fn subtree_prefix_prompt(&mut self) -> Result<Option<String>> {
        let prefixes = self.git.subtree_prefixes();
        if prefixes.is_empty() {
            self.read_line_prompt("Subtree prefix", None)
        } else {
            Ok(self.pick("Subtree prefix", &prefixes, None)?.cloned())
        }
    }
}

/// Program entry helper: launch the TUI for cwd (or first arg path).
pub fn main_run(path: Option<&str>) -> Result<()> {
    let dir = std::path::PathBuf::from(path.unwrap_or("."));
    let abs = dir.canonicalize().unwrap_or(dir);
    let git = Git::discover(&abs)?;
    let mut app = App::new(git);
    if abs.is_dir() {
        app.cwd = abs;
    }
    app.run()
}
