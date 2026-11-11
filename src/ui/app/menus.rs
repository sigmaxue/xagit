//! ui/app/menus.rs — transient menus, their values/history and extended commands.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

impl App {
    pub(super) fn open_menu(&mut self, name: &str) {
        // opening a menu only reads (the branch, remotes and config of the
        // dynamic descriptions): answer those in-process, as in a refresh
        let _queries = crate::git::query::enter(&self.git.toplevel);
        let name = resolve_menu(name, self.repo_state());
        if !self.menus.contains_key(&name) {
            return;
        }
        if name == "diff-refresh-menu" {
            let menu = diff_refresh_menu(self.mode != BufferMode::Status);
            self.menus.insert(name.clone(), menu);
        }
        if name == "log-refresh-menu" {
            let kind = match self.mode {
                BufferMode::Reflog { .. } | BufferMode::Stashes { .. } => "reflog",
                BufferMode::Cherry { .. } => "cherry",
                _ => "",
            };
            if !kind.is_empty() {
                self.set_msg(&format!("Cannot change log arguments in {kind} buffers"));
                return;
            }
            // magit-log-infix-arguments only in magit-log-mode, the short
            // "Arguments" group elsewhere
            let full = matches!(self.mode, BufferMode::Log { .. });
            self.menus.insert(name.clone(), log_refresh_menu(full));
        }
        // suffixes with :if conditions on the visited file or blob
        let (in_file, blob, blaming, read_only) = match &self.mode {
            BufferMode::File { rev, blame, .. } => {
                (true, rev != "{worktree}", blame.is_some(), blame.as_ref().is_none_or(|b| b.read_only))
            }
            _ => (false, false, false, true),
        };
        if name == "show-refs-menu" {
            let refs = matches!(self.mode, BufferMode::Refs { .. });
            self.menus.insert(name.clone(), show_refs_menu(refs));
        }
        // magit--read-mergetool: the --tool= choices are read from git
        if name == "mergetool-menu" {
            let tools = run::mergetool_tools(&self.git);
            if let Some(m) = self.menus.get_mut(&name) {
                for e in m.groups.iter_mut().flatten() {
                    if matches!(&e.action, Action::Option { arg, .. } if arg == "--tool=") {
                        e.action = Action::Option { arg: "--tool=".into(), reader: Reader::Pick(tools.clone()) };
                    }
                }
            }
        }
        // the Wiplog group :if-non-nil magit-wip-mode
        if name == "log-menu" {
            if let Some(m) = self.menus.get_mut(&name) {
                let wip = |e: &crate::ui::transient::MenuEntry| matches!(&e.action, Action::Command(c) if c.starts_with("wip-log-"));
                m.groups.retain(|g| !g.iter().any(wip));
                if self.options.wip_mode {
                    m.groups.push(crate::ui::transient::wip_log_group());
                }
            }
        }
        if name == "blame-menu" {
            self.menus.insert(name.clone(), blame_menu(blob, blaming, read_only));
        }
        // magit-dispatch: "j" is magit-status-jump in the status buffer
        // (:if-mode), magit-status-quick elsewhere (:if-not-mode)
        if name == "dispatch" {
            let status = self.mode == BufferMode::Status;
            if let Some(e) = self.menus.get_mut(&name).and_then(|m| m.groups.iter_mut().flatten().find(|e| e.key == "j")) {
                e.desc = if status { "Jump to section" } else { "Display status" }.into();
                e.action = Action::Command(if status { "jump" } else { "status-quick" }.into());
            }
        }
        if name == "file-dispatch" {
            let status = self.mode == BufferMode::Status;
            self.menus.insert(name.clone(), file_dispatch_menu(in_file, blob, blaming, read_only, status));
        }
        // `d` starts with the arguments of the diff buffer shown, else the
        // current value of magit-diff-mode; `D` with the buffer's arguments.
        // Other prefixes: the set, else the saved, else the default value.
        let key = self.value_key(&name);
        let default: Vec<String> = match self.set_values.get(&key).or(self.saved_values.get(&key)) {
            Some(v) => v.clone(),
            None => prefix_value(&key).iter().map(|s| s.to_string()).collect(),
        };
        let preset: &[String] = match (name.as_str(), &self.mode) {
            ("diff-menu", BufferMode::Diff(_)) => &self.buf_args,
            ("diff-menu", _) => &self.diff_args,
            ("diff-refresh-menu", BufferMode::Status) => &self.status_args,
            ("diff-refresh-menu", _) => &self.buf_args,
            ("log-menu" | "log-refresh-menu", m) if m.is_log() => &self.buf_args,
            ("log-refresh-menu", BufferMode::Status) => &self.status_log_args,
            ("log-menu" | "log-refresh-menu", _) => &self.log_args,
            // magit-show-refs-arguments: the refs buffer's
            ("show-refs-menu", BufferMode::Refs { .. }) => &self.buf_args,
            _ => &default,
        };
        // the suffixes at the prefix's level (the values of the others
        // unused)
        let prefix = help::prefix_command(name.split('@').next().unwrap_or(&name));
        let level = self.options.prefix_level(&prefix);
        let no_levels = BTreeMap::new();
        let levels = self.options.suffix_levels.get(&prefix).unwrap_or(&no_levels);
        if matches!(name.as_str(), "branch-menu" | "pull-menu" | "remote-menu") {
            self.var_scopes.remove(&name);
        }
        let dynamic = self.menu_states(&name);
        let inapt = match name.as_str() {
            "status-jump-menu" => self.status_jump_states(),
            "ediff-menu" => self.ediff_inapt_states(),
            _ => Vec::new(),
        };
        let gpg_format = self.config("gpg.format");
        let preset: Vec<String> = match self.menus.get_mut(&name) {
            Some(m) => {
                m.set_level(level, levels);
                for e in m.groups.iter_mut().flatten() {
                    if let Some((_, desc, shown)) = dynamic.iter().find(|(k, _, _)| *k == e.key) {
                        e.desc.clone_from(desc);
                        e.hidden |= !shown;
                    }
                    // :inapt-if-not of the section jumpers and the ediff
                    // stage/resolve suffixes (an empty description keeps
                    // the one of the menu)
                    if let Some((_, desc, inapt)) = inapt.iter().find(|(k, _, _)| *k == e.key) {
                        if !desc.is_empty() {
                            e.desc.clone_from(desc);
                        }
                        e.inapt = *inapt;
                    }
                    // magit:--gpg-sign: "Sign using " gpg.format
                    if matches!(&e.action, Action::Option { arg, .. } if arg == "--gpg-sign=") {
                        e.desc = format!("Sign using {}", gpg_format.as_deref().unwrap_or("openpgp"));
                    }
                    // magit-push:--push-option offers magit-push-options
                    if let Action::Option { arg, reader: Reader::Multi(choices) } = &mut e.action {
                        if arg == "--push-option=" {
                            choices.clone_from(&self.options.push_options);
                        }
                    }
                }
                preset.iter().filter(|a| !m.hidden_arg(a)).cloned().collect()
            }
            None => preset.to_vec(),
        };
        let active: BTreeMap<String, bool> = preset.iter().map(|a| (a.clone(), true)).collect();
        // transient--history-init: the current value, then those used before
        let value = preset;
        let mut hist = vec![value.clone()];
        hist.extend(self.history.get(&key).into_iter().flatten().filter(|v| **v != value).cloned());
        self.menu_history = (hist, 0);
        self.pending.clear();
        self.stack.push((name, active));
    }

    /// Where the value of the prefix NAME is kept: the prefix itself
    /// (its sequencer-in-progress variant shares it), or for the diff and
    /// log prefixes the arguments of a major mode (magit-diff--set-value,
    /// magit-log--set-value).
    pub(super) fn value_key(&self, name: &str) -> String {
        let base = name.split('@').next().unwrap_or(name);
        match base {
            "diff-menu" => "magit-diff:magit-diff-mode",
            "diff-refresh-menu" if self.mode == BufferMode::Status => "magit-diff:magit-status-mode",
            "diff-refresh-menu" => "magit-diff:magit-diff-mode",
            "log-refresh-menu" if self.mode == BufferMode::Status => "magit-log:magit-status-mode",
            "log-refresh-menu" if matches!(self.mode, BufferMode::LogSelect { .. }) => "magit-log:magit-log-select-mode",
            "log-menu" | "log-refresh-menu" => "magit-log:magit-log-mode",
            b => b,
        }
        .to_string()
    }

    /// The default arguments of a major mode, if KEY is one.
    pub(super) fn mode_args(&mut self, key: &str) -> Option<&mut Vec<String>> {
        match key {
            "magit-diff:magit-status-mode" => Some(&mut self.status_args),
            "magit-diff:magit-diff-mode" => Some(&mut self.diff_args),
            "magit-log:magit-log-mode" => Some(&mut self.log_args),
            "magit-log:magit-status-mode" => Some(&mut self.status_log_args),
            "magit-log:magit-log-select-mode" => Some(&mut self.log_select_args),
            _ => None,
        }
    }

    /// The value of the prefix or major mode KEY before anything is set.
    pub(super) fn initial_value(key: &str) -> Vec<String> {
        let v: &[&str] = match key {
            "magit-diff:magit-status-mode" => &["--no-ext-diff", "--ignore-submodules=none"],
            "magit-diff:magit-diff-mode" => &["--stat", "--no-ext-diff"],
            "magit-log:magit-log-mode" | "magit-log:magit-log-select-mode" => &["--graph", "-n256", "--decorate"],
            "magit-log:magit-status-mode" => &["-n256", "--decorate"],
            k => prefix_value(k),
        };
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Use the user OPTIONS.
    pub fn set_options(&mut self, options: Options) {
        self.options = options;
        self.refresh();
    }

    /// The directories where the help of transients finds info manuals
    /// and the Lisp sources of the commands.
    pub fn set_help_dirs(&mut self, dirs: Vec<std::path::PathBuf>) {
        self.help_dirs = dirs;
    }

    /// Read the user options from FILE (if it exists), reporting the
    /// lines in error.
    pub fn load_options(&mut self, file: &std::path::Path) {
        let Ok(text) = std::fs::read_to_string(file) else { return };
        self.user_init_file = Some(file.to_path_buf());
        let (options, errors) = Options::parse(&text);
        self.set_options(options);
        if !errors.is_empty() {
            self.set_msg(&format!("{}:{}", file.display(), errors.join("; ")));
        }
    }

    /// transient-values-file: read the saved values, including the default
    /// arguments of the diff and log modes, and save to FILE from now on.
    pub fn load_values(&mut self, file: std::path::PathBuf) {
        self.saved_values = crate::ui::transient::read_values(&file);
        self.values_file = Some(file);
        let saved: Vec<(String, Vec<String>)> = self.saved_values.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (k, v) in saved {
            if let Some(slot) = self.mode_args(&k) {
                *slot = v;
            }
        }
        self.refresh();
    }

    pub(super) fn save_values(&mut self) {
        if let Some(f) = &self.values_file {
            if let Err(e) = crate::ui::transient::write_values(f, &self.saved_values) {
                self.set_msg(&format!("Cannot save {}: {e}", f.display()));
            }
        }
    }

    /// transient--history-push
    pub(super) fn history_push(&mut self, key: &str, value: Vec<String>) {
        let h = self.history.entry(key.to_string()).or_default();
        h.retain(|v| *v != value);
        h.insert(0, value);
    }

    /// transient-set (C-x s) / transient-save (C-x C-s): the value of the
    /// transient shown becomes the value of the prefix for this session,
    /// and with SAVE for future ones. For the diff and log prefixes it
    /// becomes the arguments of the mode and of the buffer, which is
    /// refreshed.
    pub(super) fn transient_set_value(&mut self, save: bool) {
        let Some((name, active)) = self.stack.last().cloned() else {
            return;
        };
        let args = self.args_from(&active);
        let key = self.value_key(&name);
        if let Some(slot) = self.mode_args(&key) {
            *slot = args.clone();
            let diff_buffer = matches!(self.mode, BufferMode::Diff(_) | BufferMode::Revision { .. } | BufferMode::Stash { .. });
            if key.starts_with("magit-diff:") {
                if self.mode == BufferMode::Status {
                    self.status_args = args.clone();
                } else if diff_buffer {
                    self.buf_args = args.clone();
                }
            } else if self.mode.is_log() {
                self.buf_args = args.clone();
            }
            self.refresh();
        } else {
            self.set_values.insert(key.clone(), args.clone());
        }
        if save {
            self.saved_values.insert(key.clone(), args.clone());
            self.save_values();
        }
        self.history_push(&key, args);
    }

    /// transient-reset (C-x C-k): forget the set and saved values; the
    /// transient shown gets the default value.
    pub(super) fn transient_reset(&mut self) {
        let Some((name, _)) = self.stack.last().cloned() else {
            return;
        };
        let key = self.value_key(&name);
        let value = Self::initial_value(&key);
        self.set_values.remove(&key);
        if let Some(slot) = self.mode_args(&key) {
            *slot = value.clone();
        }
        if self.saved_values.remove(&key).is_some() {
            self.save_values();
        }
        self.history_push(&key, value.clone());
        self.set_active(value);
    }

    /// transient-history-prev (C-x p, OLDER) / -next (C-x n).
    pub(super) fn transient_history(&mut self, older: bool) {
        let (hist, pos) = &self.menu_history;
        let pos = if older { pos + 1 } else { pos.wrapping_sub(1) };
        let Some(value) = hist.get(pos).cloned() else {
            self.set_msg("End of history");
            return;
        };
        self.menu_history.1 = pos;
        self.set_active(value);
    }

    pub(super) fn set_active(&mut self, value: Vec<String>) {
        if let Some(top) = self.stack.last_mut() {
            top.1 = value.into_iter().map(|a| (a, true)).collect();
        }
    }

    pub(super) fn repo_state(&self) -> RepoState {
        RepoState {
            merge: self.git.merge_in_progress(),
            rebase: self.git.rebase_in_progress(),
            sequencer: self.git.pick_or_revert_in_progress(),
            am: self.git.am_in_progress(),
            bisect: self.git.bisect_in_progress(),
            notes_merging: self.notes_merging(),
        }
    }

    /// Name of the transient currently shown, if any (none while it is
    /// suspended to show help).
    pub fn current_menu(&self) -> Option<&str> {
        if self.transient_help.is_some() {
            return None;
        }
        self.stack.last().map(|(n, _)| n.as_str())
    }

    /// The keys of the suffixes and infixes of the transient shown.
    pub fn menu_keys(&self) -> Vec<String> {
        self.current_menu()
            .and_then(|n| self.menus.get(n))
            .map(|m| m.groups.iter().flatten().filter(|e| !e.hidden).map(|e| e.key.clone()).collect())
            .unwrap_or_default()
    }

    /// Feed one key (Emacs notation, e.g. "h", "C-g", "<tab>") as if typed.
    /// Only keys that do not prompt should be used outside the terminal.
    pub fn press(&mut self, label: &str) -> Result<()> {
        self.handle_key(crate::ui::keys::key_from_label(label))
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// The visible lines of the status buffer.
    pub fn visible_lines(&self) -> &[BufferLine] {
        &self.lines
    }

    /// Text of the line at point.
    pub fn point_text(&self) -> &str {
        self.current_line().map_or("", |l| l.text.as_str())
    }

    /// Title and content of the text view (diff / log / file), if shown.
    pub fn text_view(&self) -> Option<(&str, &str)> {
        self.view_text.as_ref().map(|(t, c)| (t.as_str(), c.as_str()))
    }

    /// First line of the text view shown on screen.
    pub fn text_view_scroll(&self) -> usize {
        self.view_scroll
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
    pub(super) fn with_terminal<T>(&mut self, f: impl FnOnce(&Git) -> T) -> T {
        if !self.in_terminal {
            return f(&self.git);
        }
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        let r = f(&self.git);
        let _ = enable_raw_mode();
        let _ = execute!(stdout(), EnterAlternateScreen);
        // the alternate screen comes back empty
        self.repaint_all = true;
        r
    }

    pub(super) fn handle_transient_key(
        &mut self,
        key: Key,
        menu: Menu,
        mut active: BTreeMap<String, bool>,
    ) -> Result<()> {
        if self.transient_helpp {
            self.transient_help_mode_key(key, &menu);
            return Ok(());
        }
        // transient-map: C-M-p / C-M-n
        if key.ctrl && (key.code == "M-p" || key.code == "M-n") && self.pending.is_empty() {
            self.transient_history(key.code == "M-p");
            return Ok(());
        }
        // transient-common-commands: the value commands under C-x
        let cx = Key { code: "x".into(), ctrl: true };
        if self.pending == [cx.clone()] {
            self.pending.clear();
            match (key.ctrl, key.code.as_str()) {
                (false, "s") => self.transient_set_value(false),
                (true, "s") => self.transient_set_value(true),
                (true, "k") => self.transient_reset(),
                (false, "p") => self.transient_history(true),
                (false, "n") => self.transient_history(false),
                // transient-toggle-common
                (false, "t") => self.options.show_common_commands = !self.options.show_common_commands,
                // transient-quit-seq; transient-sticky-map
                (true, "g") => {}
                (true, "q") => self.stack.clear(),
                (true, "z") => self.transient_suspend(),
                _ => self.transient_undefined(&format!("C-x {}", key.label()), None),
            }
            return Ok(());
        }
        if key == cx && self.pending.is_empty() {
            self.pending.push(key);
            return Ok(());
        }
        // q quits the transient unless it is a suffix (magit-blame's
        // "Quit blaming")
        let q_suffix = key.code == "q" && self.pending.is_empty() && menu.find(&key).is_some();
        if (key.code == "q" && !q_suffix) || key.code == "esc" || (key.ctrl && key.code == "g") {
            self.pending.clear();
            self.stack.pop();
            return Ok(());
        }
        // transient-quit-all
        if key.ctrl && key.code == "q" && self.pending.is_empty() {
            self.stack.clear();
            return Ok(());
        }
        // transient-suspend (also in transient-sticky-map)
        if key.ctrl && key.code == "z" {
            self.transient_suspend();
            return Ok(());
        }
        // transient-show: the menu is always shown
        if key.ctrl && key.code == "t" && self.pending.is_empty() {
            return Ok(());
        }
        let mut typed = std::mem::take(&mut self.pending);
        typed.push(key);
        let entry: MenuEntry = match menu.lookup(&typed) {
            // transient--do-warn-inapt
            Lookup::Found(e) if e.inapt => {
                let keys: Vec<String> = typed.iter().map(Key::label).collect();
                let cmd = match &e.action {
                    Action::Command(c) | Action::Submenu(c) => format!("magit-{c}"),
                    _ => "nil".into(),
                };
                self.set_msg(&format!("Inapt command: `{}' (Use `C-g' to abort, `?' for help) [{cmd}]", keys.join(" ")));
                return Ok(());
            }
            Lookup::Found(e) => e.clone(),
            Lookup::Prefix => {
                self.pending = typed;
                return Ok(());
            }
            Lookup::None => {
                let keys: Vec<String> = typed.iter().map(Key::label).collect();
                let keys = keys.join(" ");
                match typed.as_slice() {
                    // an incomplete sequence followed by an unbound key:
                    // `undefined', this-command nil
                    [_, _, ..] => self.transient_undefined(&keys, None),
                    // a key bound in the buffer's keymaps (a non-suffix:
                    // transient--do-warn), else not bound at all;
                    // transient-help enters help-mode
                    [k] if k.code == "?" || (k.ctrl && k.code == "h") => self.transient_helpp = true,
                    [k] => match self.buffer_binding(k) {
                        Some(cmd) => self.transient_undefined(&keys, Some(&cmd)),
                        None => self.set_msg(&format!("{keys} is undefined")),
                    },
                    [] => {}
                }
                return Ok(());
            }
        };
        match entry.action.clone() {
            Action::Switch(sw) => {
                let on = *active.get(&sw).unwrap_or(&false);
                active.insert(sw.clone(), !on);
                if !on {
                    crate::ui::transient::unset_incompatible(&menu, &entry, &sw, &mut active);
                }
                if let Some(top) = self.stack.last_mut() {
                    top.1 = active;
                }
            }
            // :multi-value repeat: always read, the values set the initial
            // input; empty unsets
            Action::Option { arg, reader: Reader::Multi(choices) } => {
                let old: Vec<String> =
                    active.iter().filter(|(k, on)| **on && crate::ui::transient::option_matches(&arg, k)).map(|(k, _)| k.clone()).collect();
                let initial: Vec<&str> = old.iter().map(|k| &k[arg.len()..]).collect();
                if !initial.is_empty() {
                    *self.initial_input.borrow_mut() = Some(initial.join(","));
                }
                let table = Completion { candidates: choices, require_match: false, multiple: true };
                let Some(input) = self.read_line_hist(&arg, None, Hist::Infix(&arg), Some(table))? else {
                    return Ok(());
                };
                for o in &old {
                    active.remove(o);
                }
                let values: Vec<&str> = input.split(',').map(str::trim).filter(|v| !v.is_empty()).collect();
                if !values.is_empty() {
                    add_to_history(self.infix_history.borrow_mut().entry(arg.clone()).or_default(), &values.join(","), true);
                }
                for v in values {
                    let value = format!("{arg}{v}");
                    active.insert(value.clone(), true);
                    crate::ui::transient::unset_incompatible(&menu, &entry, &value, &mut active);
                }
                if let Some(top) = self.stack.last_mut() {
                    top.1 = active;
                }
            }
            Action::Option { arg, reader } => {
                let old = entry.action.value_in(&active);
                // transient-infix-read: a value set is unset, unless the
                // infix has :always-read (magit-diff:--diff-algorithm and
                // --diff-merges)
                let always_read = matches!(arg.as_str(), "--diff-algorithm=" | "-Xdiff-algorithm=" | "--diff-merges=");
                let new = if old.is_some() && !always_read {
                    None
                } else {
                    match self.read_option(&arg, &reader)? {
                        Some(v) => v,
                        None => return Ok(()),
                    }
                };
                if let Some(o) = old {
                    active.remove(&o);
                }
                if let Some(v) = new {
                    let value = format!("{arg}{v}");
                    active.insert(value.clone(), true);
                    crate::ui::transient::unset_incompatible(&menu, &entry, &value, &mut active);
                }
                if let Some(top) = self.stack.last_mut() {
                    top.1 = active;
                }
            }
            Action::Switches(args) => {
                // transient-switches: cycle to the next argument, then none
                let old = entry.action.value_in(&active);
                let next = match &old {
                    None => args.first().cloned(),
                    Some(o) => args.iter().skip_while(|a| *a != o).nth(1).cloned(),
                };
                if let Some(o) = old {
                    active.remove(&o);
                }
                if let Some(n) = next {
                    active.insert(n.clone(), true);
                    crate::ui::transient::unset_incompatible(&menu, &entry, &n, &mut active);
                }
                if let Some(top) = self.stack.last_mut() {
                    top.1 = active;
                }
            }
            Action::Submenu(name) => {
                self.stack.pop();
                self.open_menu(&name);
            }
            // the margin commands are transient: the menu stays
            Action::Command(cmd) if cmd.starts_with("margin-") => self.margin_command(&cmd),
            Action::Command(cmd) if cmd == "blame-cycle-style" => self.blame_cycle_style(),
            // the git variable infixes set the variable, the transient
            // stays (but magit-branch.<branch>.description)
            Action::Command(cmd) if gitvar::lookup(&cmd).is_some_and(|v| !matches!(v.kind, gitvar::VarKind::Description)) => {
                let v = gitvar::lookup(&cmd).unwrap_or_else(|| unreachable!());
                self.git_var_command(v, Some(&menu.name))?;
                self.refresh_menu_states(&menu.name);
            }
            // magit-diff-refresh / magit-log-refresh "s" and "w": refresh
            // the buffer and set or save the defaults
            Action::Command(cmd) if cmd == "transient-set-and-exit" || cmd == "transient-save-and-exit" => {
                self.transient_set_value(cmd == "transient-save-and-exit");
                self.stack.pop();
            }
            Action::Command(cmd) => {
                self.stack.pop();
                // transient--export
                let key = self.value_key(&menu.name);
                self.history_push(&key, self.args_from(&active));
                if menu.name == "diff-menu" {
                    self.transient_diff_args = Some(self.args_from(&active));
                }
                if menu.name == "log-menu" {
                    self.transient_log_args = Some(self.args_from(&active));
                }
                // magit-blame-arguments: outside magit-blame the value of
                // that transient
                let blame_cmd = matches!(cmd.as_str(), "blame-addition" | "blame-removal" | "blame-reverse" | "blame-echo");
                let active = if blame_cmd && menu.name != "blame-menu" {
                    self.prefix_args("blame-menu").into_iter().map(|a| (a, true)).collect()
                } else {
                    active
                };
                self.invoking_menu = Some(menu.name.clone());
                let r = self.run_command(&cmd, &active);
                self.invoking_menu = None;
                self.transient_diff_args = None;
                self.transient_log_args = None;
                r?;
            }
        }
        Ok(())
    }

    /// transient-suspend: save the transient stack (with the values) and
    /// exit; transient-resume sets it up again.
    pub(super) fn transient_suspend(&mut self) {
        self.pending.clear();
        self.transient_helpp = false;
        self.suspended = std::mem::take(&mut self.stack);
    }

    /// transient-resume
    pub(super) fn transient_resume(&mut self) {
        if self.suspended.is_empty() {
            self.set_msg("No suspended transient command");
        } else {
            self.stack = std::mem::take(&mut self.suspended);
        }
    }

    /// The description of transient-toggle-common.
    pub(super) fn toggle_common_description(&self) -> &'static str {
        if self.options.show_common_commands {
            "Hide common commands"
        } else {
            "Show common permanently"
        }
    }

    /// execute-extended-command (M-x): read a command and call it (with
    /// the prefix argument, shown before "M-x "), disabled commands
    /// through disabled-command-function. The commands are
    /// transient-resume, magit-clean and the transient prefixes.
    pub(super) fn execute_extended_command(&mut self) -> Result<()> {
        let mut commands: Vec<String> = [
            "transient-resume",
            "magit-clean",
            "magit-wip-mode",
            "magit-wip-commit",
            "magit-wip-log",
            "magit-wip-log-current",
            "magit-wip-log-index",
            "magit-wip-log-worktree",
            "magit-wip-purge",
            "magit-branch-orphan",
            "magit-branch-shelve",
            "magit-branch-unshelve",
            "magit-delete-shelved-branch",
        ]
        .map(String::from)
        .to_vec();
        for name in self.menus.keys() {
            let cmd = help::prefix_command(name);
            if !commands.contains(&cmd) {
                commands.push(cmd);
            }
        }
        commands.sort();
        let arg = match self.prefix_arg {
            None => String::new(),
            Some(PrefixArg::Minus) => "- ".into(),
            Some(PrefixArg::Raw(4)) => "C-u ".into(),
            Some(PrefixArg::Raw(n) | PrefixArg::Num(n)) => format!("{n} "),
        };
        let Some(cmd) = self.read_choice(&format!("{arg}M-x "), &commands, None)? else {
            return Ok(());
        };
        if self.options.disabled(&cmd) && !self.disabled_command_function(&cmd)? {
            return Ok(());
        }
        if cmd == "transient-resume" {
            self.transient_resume();
        } else if cmd == "magit-clean" {
            self.magit_clean()?;
        } else if cmd == "magit-wip-mode" {
            self.wip_mode_command();
        } else if cmd == "magit-wip-purge" {
            self.wip_purge()?;
        } else if cmd == "magit-fetch-modules" {
            // a prefix that fetches unless invoked with a prefix argument
            self.run_command("fetch-modules", &BTreeMap::new())?;
        } else if let Some(c) = cmd
            .strip_prefix("magit-")
            .filter(|c| c.starts_with("wip-") || c.starts_with("branch-") || *c == "delete-shelved-branch")
        {
            self.run_command(c, &BTreeMap::new())?;
        } else if let Some(name) = self.menus.keys().find(|n| help::prefix_command(n) == cmd).cloned() {
            self.open_menu(&name);
        }
        Ok(())
    }

    /// disabled-command-function for CMD invoked by M-x: whether to call
    /// it, asked with read-multiple-choice and the help in
    /// *Disabled Command*. `y` enables CMD (saved in the options file if
    /// confirmed), `!` all disabled commands for the session.
    pub(super) fn disabled_command_function(&mut self, cmd: &str) -> Result<bool> {
        let choices = [('n', "no"), ('y', "yes"), (' ', "(space bar) only once"), ('!', "use and enable all")];
        let help = disabled_command_help(cmd);
        let Some(c) = self.read_multiple_choice("Use this command?", &choices, &help)? else {
            self.set_msg("Quit");
            return Ok(false);
        };
        match c {
            '!' => self.options.disabled_command_function = false,
            'y' => {
                if self.user_init_file.is_some() && self.yes_no("Enable command for future editing sessions also? ")? {
                    self.enable_command(cmd)?;
                }
                self.options.put_disabled(cmd, false);
            }
            _ => {}
        }
        Ok(c != 'n')
    }

    /// enable-command (en/disable-command): the first line of the
    /// options file enabling or disabling CMD is replaced by
    /// "enable-command CMD" at its end.
    pub(super) fn enable_command(&mut self, cmd: &str) -> Result<()> {
        let Some(file) = self.user_init_file.clone() else { return Ok(()) };
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let mut lines: Vec<&str> = text.split_inclusive('\n').collect();
        let put = |l: &str| {
            let mut w = l.split_whitespace();
            matches!(w.next(), Some("enable-command" | "disable-command")) && w.next() == Some(cmd)
        };
        if let Some(i) = lines.iter().position(|l| put(l)) {
            lines.remove(i);
        }
        let mut text = lines.concat();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("enable-command {cmd}\n"));
        std::fs::write(&file, text)?;
        Ok(())
    }

    /// read-multiple-choice with SHOW-HELP: PROMPT and the CHOICES
    /// ("[N]o", "SPC (space bar) only once": the key in brackets, or
    /// before the name), HELP shown above the minibuffer. Any key
    /// answers at once: a key not among the choices (but `?` and C-h)
    /// prompts again after "Invalid choice.  ". None on C-g.
    pub(super) fn read_multiple_choice(&mut self, prompt: &str, choices: &[(char, &str)], help: &str) -> Result<Option<char>> {
        let names: Vec<String> = choices.iter().map(|(c, name)| rmc_key_description(*c, name)).collect();
        let full = format!("{prompt} ({}): ", names.join(", "));
        *self.minibuffer_above.borrow_mut() = help.lines().map(str::to_string).collect();
        let mut wrong = false;
        let answer = loop {
            let p = if wrong { format!("Invalid choice.  {full}") } else { full.clone() };
            let Some(ans) = self.read_instant(&p, vec![(ANY_KEY.into(), String::new())])? else {
                break None;
            };
            let mut it = ans.chars();
            match (it.next(), it.next()) {
                (Some(c), None) if choices.iter().any(|(k, _)| *k == c) => break Some(c),
                _ => wrong = !matches!(ans.as_str(), "?" | "C-h"),
            }
        };
        self.minibuffer_above.borrow_mut().clear();
        Ok(answer)
    }

    /// magit-wip-mode as a command: toggled, or enabled by a positive
    /// prefix argument (disabled by another one).
    pub(super) fn wip_mode_command(&mut self) {
        self.options.wip_mode = match self.prefix_arg {
            None => !self.options.wip_mode,
            Some(a) => a.numeric() >= 1,
        };
        self.set_msg(&format!("Magit-Wip mode {}abled", if self.options.wip_mode { "en" } else { "dis" }));
    }

    /// magit-wip-purge: delete the wip refs (refs/wip/index/REF and
    /// refs/wip/wtree/REF) whose REF (a branch, remote branch or tag)
    /// no longer exists, those of HEAD kept, after confirmation.
    pub(super) fn wip_purge(&mut self) -> Result<()> {
        let refs = self.git.git_lines(&["for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes", "refs/tags"]);
        let wiprefs: Vec<String> = self
            .git
            .git_lines(&["for-each-ref", "--format=%(refname)", "refs/wip/"])
            .into_iter()
            .filter(|w| !refs.iter().any(|r| w.get(15..) == Some(r.as_str())))
            .filter(|w| w != "refs/wip/index/HEAD" && w != "refs/wip/wtree/HEAD")
            .collect();
        if wiprefs.is_empty() {
            self.set_msg("All wip-refs have a corresponding ref");
            return Ok(());
        }
        let one = "Delete wip-ref %s without corresponding ref";
        let many = "Delete %d wip-refs without corresponding ref";
        if !self.confirm_items("purge-dangling-wiprefs", one, many, &wiprefs)? {
            return Ok(());
        }
        for w in &wiprefs {
            let _ = self.git.git_ok(&["update-ref", "-d", w]);
        }
        self.set_msg("Deleting wip-refs...done");
        self.refresh();
        Ok(())
    }

    /// magit-clean: remove the untracked files (prefix argument: and
    /// the ignored ones, two: just the ignored ones) after confirmation
    /// with yes-or-no-p (git clean -f -d [-x|-X]).
    pub(super) fn magit_clean(&mut self) -> Result<()> {
        let arg = self.prefix_numeric();
        let what = match arg {
            1 => "untracked",
            4 => "untracked and ignored",
            _ => "ignored",
        };
        if !self.yes_or_no_p(&format!("Remove {what} files? "))? {
            return Ok(());
        }
        self.wip_hook(&[], "before", "clean")?;
        let mut a = vec!["clean", "-f", "-d"];
        match arg {
            4 => a.push("-x"),
            16 => a.push("-X"),
            _ => {}
        }
        if let Err(e) = self.git.git_interactive(&a) {
            self.set_msg(&format!("{e}"));
        }
        self.refresh();
        Ok(())
    }

    /// transient-undefined: KEYS bound to CMD (nil: none) aren't a
    /// suffix of the transient shown.
    pub(super) fn transient_undefined(&mut self, keys: &str, cmd: Option<&str>) {
        let cmd = cmd.unwrap_or("nil");
        self.set_msg(&format!("Unbound suffix: `{keys}' (Use `C-g' to abort, `?' for help) [{cmd}]"));
    }

    /// A key of the transient MENU in help-mode (transient--helpp): the
    /// quit commands leave help-mode, `?' shows the help of the prefix,
    /// a suffix its help, other keys are transient-undefined.
    pub(super) fn transient_help_mode_key(&mut self, key: Key, menu: &Menu) {
        let cx = Key { code: "x".into(), ctrl: true };
        let describe = |s: &mut Self, cmd: &str, desc: &str| {
            let text = help::describe_function(&s.help_dirs, cmd, desc);
            s.show_transient_help(Ok(("*Help*".into(), text, 0)));
        };
        if self.pending == [cx.clone()] {
            self.pending.clear();
            let cmd = match (key.ctrl, key.code.as_str()) {
                (false, "s") => "transient-set",
                (true, "s") => "transient-save",
                (true, "k") => "transient-reset",
                (false, "p") => "transient-history-prev",
                (false, "n") => "transient-history-next",
                (false, "t") => "transient-toggle-common",
                (true, "q") => {
                    self.transient_helpp = false;
                    self.stack.clear();
                    return;
                }
                (true, "z") => "transient-suspend",
                // transient-quit-seq
                (true, "g") => return,
                _ => return self.transient_undefined(&format!("C-x {}", key.label()), None),
            };
            let seq = format!("C-x {}", key.label());
            let desc = match VALUE_COMMANDS.iter().chain(&STICKY_COMMANDS).find(|(k, _)| *k == seq || *k == key.label()) {
                Some((_, d)) => d,
                None => self.toggle_common_description(),
            };
            return describe(self, cmd, desc);
        }
        if key == cx && self.pending.is_empty() {
            self.pending.push(key);
            return;
        }
        if key.ctrl && self.pending.is_empty() && matches!(key.code.as_str(), "z" | "t") {
            return if key.code == "z" {
                describe(self, "transient-suspend", "Suspend transient stack")
            } else {
                describe(self, "transient-show", "Show the transient's state in the menu buffer")
            };
        }
        if key.ctrl && (key.code == "M-p" || key.code == "M-n") && self.pending.is_empty() {
            let (cmd, desc) = if key.code == "M-p" {
                ("transient-history-prev", "Previous value")
            } else {
                ("transient-history-next", "Next value")
            };
            return describe(self, cmd, desc);
        }
        // transient--quit-commands: transient--do-quit-one leaves help-mode
        let q_suffix = key.code == "q" && self.pending.is_empty() && menu.find(&key).is_some();
        if (key.code == "q" && !q_suffix) || key.code == "esc" || (key.ctrl && key.code == "g") {
            self.pending.clear();
            self.transient_helpp = false;
            return;
        }
        if key.ctrl && key.code == "q" && self.pending.is_empty() {
            self.transient_helpp = false;
            self.stack.clear();
            return;
        }
        let mut typed = std::mem::take(&mut self.pending);
        typed.push(key);
        let help = match menu.lookup(&typed) {
            Lookup::Found(e) => self.suffix_help(menu, &e.clone()),
            Lookup::Prefix => {
                self.pending = typed;
                return;
            }
            Lookup::None => {
                let keys: Vec<String> = typed.iter().map(Key::label).collect();
                let keys = keys.join(" ");
                match typed.as_slice() {
                    [k] if k.code == "?" || (k.ctrl && k.code == "h") => self.prefix_help(&menu.name),
                    [k] => {
                        let cmd = self.buffer_binding(k);
                        return self.transient_undefined(&keys, cmd.as_deref());
                    }
                    _ => return self.transient_undefined(&keys, None),
                }
            }
        };
        self.show_transient_help(help);
    }

    /// The help of the transient NAME (transient-show-help of a prefix):
    /// its info manual, else its manpage, else its documentation.
    pub(super) fn prefix_help(&self, name: &str) -> std::result::Result<(String, String, usize), String> {
        if let Some(spec) = help::info_manual(name) {
            return help::info_node(&self.help_dirs, spec).map(|(t, c)| (t, c, 0));
        }
        if let Some(page) = help::man_page(name) {
            return help::manpage(page, self.help_width()).map(|c| (format!("*Man {page}*"), c, 0));
        }
        let title = self.menus.get(name).map_or("", |m| m.title.as_str());
        Ok(("*Help*".into(), help::describe_function(&self.help_dirs, &help::prefix_command(name), title), 0))
    }

    /// The help of the suffix ENTRY of MENU: a sub-prefix's help, the
    /// manpage of the prefix at the description of an infix's argument,
    /// else the documentation of the command.
    pub(super) fn suffix_help(&self, menu: &Menu, entry: &MenuEntry) -> std::result::Result<(String, String, usize), String> {
        let prefix = help::prefix_command(&menu.name);
        let doc = |cmd: &str| Ok(("*Help*".to_string(), help::describe_function(&self.help_dirs, cmd, &entry.desc), 0));
        match &entry.action {
            Action::Switch(arg) | Action::Option { arg, .. } => match help::man_page(&menu.name) {
                Some(page) => help::manpage(page, self.help_width()).map(|c| {
                    let line = help::argument_line(&c, arg).unwrap_or(0);
                    (format!("*Man {page}*"), c, line)
                }),
                None => doc(&format!("transient:{prefix}:{arg}")),
            },
            // a transient-switches has no argument
            Action::Switches(_) => doc(&format!("transient:{prefix}:{}", entry.key)),
            Action::Submenu(name) => self.prefix_help(&resolve_menu(name, self.repo_state())),
            Action::Command(cmd) if self.menus.contains_key(cmd) => self.prefix_help(&resolve_menu(cmd, self.repo_state())),
            Action::Command(cmd) if cmd.starts_with("transient-") => doc(cmd),
            Action::Command(cmd) => doc(&format!("magit-{cmd}")),
        }
    }

    /// The width manpages are formatted for.
    pub(super) fn help_width(&self) -> usize {
        crossterm::terminal::size().map_or(80, |(w, _)| usize::from(w))
    }

    /// Leave help-mode and suspend the transient to show HELP (title,
    /// text, first line shown) until `q' resumes it (transient--display-help).
    pub(super) fn show_transient_help(&mut self, help: std::result::Result<(String, String, usize), String>) {
        self.transient_helpp = false;
        match help {
            Ok((title, text, line)) => {
                let saved = (self.view_text.take(), self.view_scroll);
                self.transient_help = Some(saved);
                self.view_text = Some((title, text));
                self.view_scroll = line;
                self.set_msg("Type `q' to resume transient command.");
            }
            Err(e) => self.set_msg(&format!("transient-help: {e}")),
        }
    }

    /// A key in the help of a suspended transient (transient-resume-mode):
    /// `q' resumes it (transient-resume), the line motions scroll.
    pub(super) fn transient_help_key(&mut self, key: &Key) {
        if key.code == "q" && !key.ctrl {
            if let Some((view, scroll)) = self.transient_help.take() {
                self.view_text = view;
                self.view_scroll = scroll;
            }
            self.set_msg("");
            return;
        }
        match status_binding(key) {
            Some(Binding::Down | Binding::SectionForward) => self.view_scroll += 1,
            Some(Binding::Up | Binding::SectionBackward) => self.view_scroll = self.view_scroll.saturating_sub(1),
            Some(Binding::KeyboardQuit) => self.set_msg("Quit"),
            _ => self.set_msg(&format!("{} is undefined", key.label())),
        }
    }

    /// The command KEY is bound to in the keymaps of the buffer shown
    /// (the name of the Magit command), if any.
    pub(super) fn buffer_binding(&self, key: &Key) -> Option<String> {
        let file = matches!(self.mode, BufferMode::File { .. }) && self.view_text.is_none();
        let b = status_binding(key)?;
        if file && !b.is_global() {
            return None;
        }
        Some(binding_command(&b))
    }

}
