//! ui/app.rs — application state machine, event loop, prompts and command dispatch.

use super::keys::Key;
use super::transient::{registry, Action, Menu, MenuEntry};
use super::view::{render_status, BufferLine, LineKind};
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
    lines: Vec<BufferLine>,
    cursor: usize,
    message: String,
    quit: bool,
    /// pending text-mode buffer shown instead of status (diff/log/show output)
    view_text: Option<(String, String)>, // (title, content)
}

impl App {
    pub fn new(git: Git) -> App {
        let mut app = App {
            git,
            menus: registry(),
            stack: Vec::new(),
            lines: Vec::new(),
            cursor: 0,
            message: String::new(),
            quit: false,
            view_text: None,
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
                "\r\n[ngit:{branch}] {}\r\n(h/? dispatch, g refresh, q quit, TAB toggle, RET visit, j/k move)",
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
        // global keys (magit-mode-map)
        match key.code.as_str() {
            "q" | "esc" => {
                if self.view_text.is_some() {
                    self.view_text = None;
                    self.set_msg("");
                    return Ok(());
                }
                self.quit = true;
                return Ok(());
            }
            "h" | "?" => {
                self.open_menu("dispatch");
                return Ok(());
            }
            "g" => {
                self.refresh();
                self.set_msg("refreshed");
                return Ok(());
            }
            "j" | "down" => {
                if self.cursor + 1 < self.lines.len() {
                    self.cursor += 1;
                }
                return Ok(());
            }
            "k" | "up" => {
                self.cursor = self.cursor.saturating_sub(1);
                return Ok(());
            }
            "tab" => {
                self.toggle_section();
                return Ok(());
            }
            "enter" => {
                self.visit_thing()?;
                return Ok(());
            }
            "$" => {
                self.open_menu("run-menu");
                return Ok(());
            }
            _ => {}
        }
        // direct Magit single-key bindings (magit-mode-map)
        let command = match key.code.as_str() {
            "s" => "stage".to_string(),
            "S" => "stage-all".to_string(),
            "u" => "unstage".to_string(),
            "U" => "unstage-all".to_string(),
            "k" => "discard".to_string(),
            "K" => "untrack".to_string(),
            "x" => "reset-quickly".to_string(),
            "c" => "commit-menu".to_string(),
            "b" => "branch-menu".to_string(),
            "f" => "fetch-menu".to_string(),
            "F" => "pull-menu".to_string(),
            "P" => "push-menu".to_string(),
            "m" => "merge-menu".to_string(),
            "M" => "remote-menu".to_string(),
            "r" => "rebase-menu".to_string(),
            "t" => "tag-menu".to_string(),
            "z" => "stash-menu".to_string(),
            "X" => "reset-menu".to_string(),
            "i" => "gitignore-menu".to_string(),
            "l" => "log-menu".to_string(),
            "d" => "diff-menu".to_string(),
            "A" => "cherry-pick-menu".to_string(),
            "V" => "revert".to_string(),
            "y" => "show-refs".to_string(),
            ":" | "Q" => "git-command".to_string(),
            "!" => "run-menu".to_string(),
            "I" => "init".to_string(),
            "C" => "clone".to_string(),
            _ => return Ok(()),
        };
        if command.ends_with("-menu") || command == "dispatch" {
            self.open_menu(&command);
        } else {
            self.run_command(&command, &BTreeMap::new())?;
        }
        Ok(())
    }

    fn open_menu(&mut self, name: &str) {
        if self.menus.contains_key(name) {
            self.stack.push((name.to_string(), BTreeMap::new()));
        }
    }

    fn handle_transient_key(
        &mut self,
        key: Key,
        menu: Menu,
        mut active: BTreeMap<String, bool>,
    ) -> Result<()> {
        if key.code == "q" || key.code == "esc" || (key.ctrl && key.code == "g") {
            self.stack.pop();
            return Ok(());
        }
        if let Some(entry) = menu.find(&key) {
            let entry: MenuEntry = entry.clone();
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
                     s stage, u unstage, S stage all, U unstage all\n\
                     k discard, K untrack, x reset quickly\n\
                     c commit, b branch, z stash, P push, F pull, f fetch\n\
                     m merge, r rebase, X reset, t tag, M remote\n\
                     l log, d diff, A cherry-pick, V revert\n\
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

            // -------- fetch/pull/push
            "fetch-upstream" | "fetch-other" | "fetch-branch" | "fetch-all" => {
                let remotes = g!().remotes();
                match cmd {
                    "fetch-all" => {
                        let prune = args.iter().any(|a| a == "--prune");
                        g!().fetch_all(prune)?;
                        self.set_msg("fetched all remotes");
                    }
                    "fetch-upstream" => {
                        if let Some(r) = remotes.first() {
                            g!().fetch(r, &args)?;
                            self.set_msg(&format!("fetched {r}"));
                        } else {
                            self.set_msg("no remote configured");
                        }
                    }
                    _ => {
                        if let Some(r) = self.pick("Fetch from remote", &remotes, None)? {
                            g!().fetch(r, &args)?;
                            self.set_msg(&format!("fetched {r}"));
                        }
                    }
                }
                self.refresh();
            }
            "pull-upstream" | "pull-other" => {
                let remotes = g!().remotes();
                let branch = g!().current_branch().unwrap_or_else(|| "HEAD".into());
                if cmd == "pull-upstream" {
                    if let Some(r) = remotes.first() {
                        g!().pull(r, &branch, &args)?;
                        self.set_msg("pulled");
                    } else {
                        self.set_msg("no remote configured");
                    }
                } else if let Some(r) = self.pick("Pull from remote", &remotes, None)? {
                    g!().pull(r, &branch, &args)?;
                    self.set_msg("pulled");
                }
                self.refresh();
            }
            "push-upstream" | "push-other" | "push-other-branch" => {
                let remotes = g!().remotes();
                let branch = match g!().current_branch() {
                    Some(b) => b,
                    None => {
                        self.set_msg("cannot push: detached HEAD");
                        return Ok(());
                    }
                };
                match cmd {
                    "push-upstream" => {
                        if let Some(r) = remotes.first() {
                            g!().push(r, &branch, &args)?;
                            self.set_msg("pushed");
                        } else {
                            self.set_msg("no remote configured");
                        }
                    }
                    _ => {
                        if let Some(r) = self.pick("Push to remote", &remotes, None)? {
                            g!().push(r, &branch, &args)?;
                            self.set_msg("pushed");
                        }
                    }
                }
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

            // -------- merge / rebase
            "merge-plain" | "merge-nocommit" | "merge-squash" => {
                let bs = g!().branches();
                if let Some(rev) = self.pick("Merge revision", &bs, None)? {
                    match cmd {
                        "merge-plain" => g!().merge(rev, &args)?,
                        "merge-nocommit" => g!().merge_nocommit(rev)?,
                        "merge-squash" => g!().merge_squash(rev)?,
                        _ => unreachable!(),
                    }
                    self.set_msg("merged");
                    self.refresh();
                }
            }
            "merge-abort" => {
                g!().merge_abort()?;
                self.set_msg("merge aborted");
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
            "revert" | "reverse" | "apply" => {
                let log = g!().log(None, 30)?;
                let revs: Vec<String> = log
                    .iter()
                    .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
                    .collect();
                if let Some(r) = self.pick("Revision (hash)", &revs, None)? {
                    let no_commit = cmd != "revert";
                    g!().revert(&[r.clone()], &[], no_commit)?;
                    self.set_msg("done");
                    self.refresh();
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
                let Some(pattern) = self.read_line_prompt("Pattern to ignore", None)? else {
                    return Ok(());
                };
                g!().ignore(&pattern)?;
                self.set_msg("added to .gitignore");
                self.refresh();
            }
            "git-command" => {
                let Some(line) = self.read_line_prompt("git command (args after 'git')", Some("status"))? else {
                    return Ok(());
                };
                let parts: Vec<String> = line.split_whitespace().map(|s| s.to_string()).collect();
                let strs: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
                let out = g!().git_string(&strs);
                match out {
                    Ok(o) => self.view_text = Some((format!("git {}", line), o)),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
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

            // -------- log / diff / show
            "log-current" | "log-other" | "log-branches" | "log-all-branches" => {
                let (rev, limit) = match cmd {
                    "log-current" => (None, 50),
                    "log-other" => {
                        let bs = g!().branches();
                        (self.pick("Log branch", &bs, None)?.map(|s| s.clone()), 50)
                    }
                    "log-branches" => (Some("--branches".into()), 50),
                    _ => (Some("--all".into()), 50),
                };
                match g!().log(rev.as_deref(), limit) {
                    Ok(l) if l.is_empty() => self.set_msg("no commits"),
                    Ok(l) => self.view_text = Some(("log".into(), l.join("\n"))),
                    Err(e) => self.set_msg(&format!("{e}")),
                }
            }
            "diff-unstaged" => match g!().diff_unstaged() {
                Ok(d) => self.view_text = Some(("diff unstaged".into(), d)),
                Err(e) => self.set_msg(&format!("{e}")),
            },
            "diff-staged" => match g!().diff_staged() {
                Ok(d) => self.view_text = Some(("diff staged".into(), d)),
                Err(e) => self.set_msg(&format!("{e}")),
            },
            "show-commit" => {
                let log = g!().log(None, 30)?;
                let revs: Vec<String> = log
                    .iter()
                    .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
                    .collect();
                if let Some(r) = self.pick("Show commit", &revs, None)? {
                    match g!().show_commit(r) {
                        Ok(s) => self.view_text = Some((format!("commit {r}"), s)),
                        Err(e) => self.set_msg(&format!("{e}")),
                    }
                }
            }

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

            "unsupported" => self.set_msg("command not supported in this build"),

            // Magit extras (bisect/worktree/submodule/notes/ediff)
            _ => self.run_extra(cmd)?,
        }
        Ok(())
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

    fn run_extra(&mut self, cmd: &str) -> Result<()> {
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
            "bisect-run" => {
                let Some(command) = self.read_line_prompt("Bisect run shell command", None)? else {
                    return Ok(());
                };
                let res = self.git.bisect_run(&command);
                self.show("bisect run", res);
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
            "submodule-init" => {
                let res = self.git.submodule_init();
                self.done("submodules initialized", res);
            }
            "submodule-update" | "submodule-update-recursive" => {
                let recursive = cmd == "submodule-update-recursive";
                let res = self.git.submodule_update(true, recursive);
                self.done("submodules updated", res);
            }
            "submodule-sync" => {
                let res = self.git.submodule_sync();
                self.done("submodules synced", res);
            }
            "submodule-deinit" | "submodule-remove" => {
                let paths = self.git.submodule_paths();
                let Some(path) = self.pick("Submodule", &paths, None)?.cloned() else {
                    return Ok(());
                };
                if !self.yes_no(&format!("{cmd} {path}?"))? {
                    return Ok(());
                }
                let res = if cmd == "submodule-deinit" {
                    self.git.submodule_deinit(&path)
                } else {
                    self.git.submodule_remove(&path)
                };
                self.done("submodule updated", res);
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

            // ---- ediff / diff views (text buffers instead of Emacs sessions)
            "ediff-dwim" => {
                let cur = self.current_file();
                let res: Result<String> = match cur {
                    Some((section, path)) if section == "untracked" => Ok(format!(
                        "New file: {path}\n\n{}",
                        std::fs::read_to_string(self.git.toplevel.join(&path))
                            .unwrap_or_default()
                    )),
                    Some((section, path)) => self.git.diff_file(&path, section == "staged"),
                    None => self.git.diff_unstaged(),
                };
                self.show("ediff dwim", res);
            }
            "ediff-unstaged" | "ediff-staged" => {
                let res = if cmd == "ediff-staged" {
                    self.git.diff_staged()
                } else {
                    self.git.diff_unstaged()
                };
                self.show("ediff", res);
            }
            "ediff-range" => {
                let Some(from) = self.read_line_prompt("Diff from revision", Some("HEAD~1"))? else {
                    return Ok(());
                };
                let Some(to) = self.read_line_prompt("Diff to revision", Some("HEAD"))? else {
                    return Ok(());
                };
                let res = self.git.diff_range(&from, &to);
                self.show(&format!("{from}..{to}"), res);
            }
            "ediff-commit" | "diff-change" => {
                let log = self.git.log(None, 30)?;
                let revs: Vec<String> = log
                    .iter()
                    .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
                    .collect();
                let Some(rev) = self.pick("Commit", &revs, None)?.cloned() else {
                    return Ok(());
                };
                let res = if cmd == "diff-change" {
                    self.git.diff_change(&rev)
                } else {
                    self.git.show_commit(&rev)
                };
                self.show(&format!("commit {rev}"), res);
            }

            _ => self.set_msg(&format!("unknown command: {cmd}")),
        }
        Ok(())
    }
}

/// Program entry helper: launch the TUI for cwd (or first arg path).
pub fn main_run(path: Option<&str>) -> Result<()> {
    let dir = std::path::PathBuf::from(path.unwrap_or("."));
    let abs = dir.canonicalize().unwrap_or(dir);
    let git = Git::discover(&abs)?;
    let mut app = App::new(git);
    app.run()
}
