//! ui/app/visit.rs — visiting files, blame and the file dispatch commands.
//!
//! `impl App` is split across the `app::*` modules; `use super::*`
//! gives each of them the parent's private fields and helpers.

use super::*;

impl App {

    /// magit-diff-visit--sides: the (revision, file) of the old and the new
    /// side of the diff of PATH in GROUP; revisions are "{worktree}",
    /// "{index}" or a revision.
    pub(super) fn visit_sides(&self, group: &str, path: &str) -> ((String, String), (String, String)) {
        let orig = self.buf.diff(group, path).and_then(|d| d.orig.clone()).unwrap_or_else(|| path.to_string());
        let wt = || "{worktree}".to_string();
        let abbrev = |r: &str| self.git.rev_short(r);
        let (old, new) = match (&self.mode, self.diff_type(group)) {
            (BufferMode::Revision { rev, .. }, _) => (abbrev(&format!("{rev}^")), abbrev_if_oid(&self.git, rev)),
            // a diff inside a log: its group is the commit
            (BufferMode::Log { .. } | BufferMode::LogSelect { .. } | BufferMode::Reflog { .. }, _) => {
                (abbrev(&format!("{group}^")), abbrev_if_oid(&self.git, group))
            }
            (BufferMode::Stash { stash }, _) => {
                let rev = match group {
                    "stash-index" => format!("{stash}^2"),
                    "stash-untracked" => format!("{stash}^3"),
                    _ => stash.clone(),
                };
                (abbrev(&format!("{rev}^")), rev)
            }
            (BufferMode::Diff(spec), "committed") => match &spec.range {
                Some(r) => self.split_range(r, spec.typearg.as_deref() == Some("--cached")),
                None => (wt(), wt()),
            },
            (_, "staged") => (abbrev("HEAD"), "{index}".to_string()),
            (_, "unstaged") => {
                let staged = self.git.git_ok(&["diff", "--cached", "--quiet", "--", &orig]);
                (if staged { abbrev("HEAD") } else { "{index}".to_string() }, wt())
            }
            _ => (wt(), wt()),
        };
        ((old, orig), (new, path.to_string()))
    }

    /// magit-split-range: "A..B" -> (A, B), "A...B" -> (merge base, B); a
    /// single revision is compared with the worktree (or the index).
    pub(super) fn split_range(&self, range: &str, cached: bool) -> (String, String) {
        let side = |s: &str| if s.is_empty() { "HEAD".to_string() } else { s.to_string() };
        if let Some((a, b)) = range.split_once("...") {
            let base = self.git.git_string(&["merge-base", &side(a), &side(b)]).unwrap_or_default();
            (self.git.rev_short(base.trim()), side(b))
        } else if let Some((a, b)) = range.split_once("..") {
            (side(a), side(b))
        } else {
            (range.to_string(), if cached { "{index}" } else { "{worktree}" }.to_string())
        }
    }

    /// magit-diff-hunk-line: the line in the visited file of body line LINE
    /// (or the heading) of hunk HUNK, on the old side when GOTO_FROM. The
    /// lines before it are counted unless their prefix (a column per
    /// parent in combined diffs) has a `-` (`+` for GOTO_FROM). On the
    /// heading Magit moves past the first `-` or `+` of the body, so that
    /// line is counted too.
    pub(super) fn hunk_line(fd: &FileDiff, hunk: usize, line: Option<usize>, goto_from: bool) -> usize {
        let Some(h) = fd.hunks.get(hunk) else { return 1 };
        let start = h.range().map_or(1, |r| if goto_from { r.old_start } else { r.new_start }).max(1);
        let prefix = h.header.chars().take_while(|c| *c == '@').count().saturating_sub(1).max(1);
        let skip = if goto_from { '+' } else { '-' };
        let target = match line {
            Some(li) => li,
            None => h.lines.iter().position(|l| l.starts_with(['-', '+'])).map_or(0, |i| i + 1),
        };
        start + h.lines.iter().take(target).filter(|l| !l.chars().take(prefix).any(|c| c == skip)).count()
    }

    /// magit-diff-visit--offset: LINE of FILE at REV moved by the changes
    /// between REV (or the index) and the worktree.
    pub(super) fn visit_offset(&self, line: usize, file: &str, rev: &str) -> usize {
        let out = if rev == "{index}" {
            self.git.git_string(&["diff", "--no-ext-diff", "--", file])
        } else {
            self.git.git_string(&["diff", "--no-ext-diff", rev, "--", file])
        };
        let mut offset: isize = 0;
        for h in crate::git::diff::parse(&out.unwrap_or_default()).iter().flat_map(|f| f.hunks.iter()) {
            let Some(r) = h.range() else { continue };
            if r.old_start > line {
                break;
            }
            if r.old_start + r.old_len < line {
                offset += r.new_len as isize - r.old_len as isize;
                continue;
            }
            let mut rest = line - r.old_start;
            for l in &h.lines {
                if rest == 0 {
                    break;
                }
                match l.chars().next() {
                    Some(' ') => rest -= 1,
                    Some('-') => {
                        offset -= 1;
                        rest -= 1;
                    }
                    Some('+') => offset += 1,
                    _ => {}
                }
            }
            break;
        }
        (line as isize + offset).max(1) as usize
    }

    /// magit-diff-visit-file (WORKTREE: magit-diff-visit-worktree-file):
    /// show the version of the file at point, at the line at point. Added
    /// and context lines visit the new side, removed lines the old side.
    pub(super) fn visit_file(&mut self, group: &str, path: &str, hunk: Option<(usize, Option<usize>)>, worktree: bool) {
        let (old, new) = self.visit_sides(group, path);
        let fd = self.buf.diff(group, path).cloned();
        let on_removed = match (hunk, &fd) {
            (Some((h, Some(l))), Some(fd)) => {
                !fd.combined && fd.hunks.get(h).and_then(|x| x.lines.get(l)).is_some_and(|t| t.starts_with('-'))
            }
            _ => false,
        };
        let goto_from = !worktree && on_removed;
        let (rev, file) = if goto_from { old } else { new };
        let mut line = match (hunk, &fd) {
            (Some((h, l)), Some(fd)) => Some(Self::hunk_line(fd, h, l, goto_from)),
            _ => None,
        };
        let rev = if worktree || rev == "{worktree}" {
            if worktree && rev != "{worktree}" {
                line = line.map(|l| self.visit_offset(l, &file, &rev));
            }
            "{worktree}".to_string()
        } else {
            rev
        };
        if let Err(e) = crate::ui::blame::file_contents(&self.git, &rev, &file) {
            self.set_msg(&format!("{e:#}"));
            return;
        }
        self.find_file(&rev, &file, line.unwrap_or(1));
    }

    /// magit-find-file: the buffer visiting FILE at REV, at LINE.
    pub(super) fn find_file(&mut self, rev: &str, file: &str, line: usize) {
        self.setup_buffer(BufferMode::File { rev: rev.into(), file: file.into(), blame: None }, Vec::new());
        self.goto_file_line(line);
    }

    /// The line of the file at point (1-based), and its blame chunk.
    pub fn file_line(&self) -> Option<usize> {
        self.file_line_at_point().map(|(l, _)| l)
    }

    pub(super) fn file_line_at_point(&self) -> Option<(usize, Option<usize>)> {
        let lines = &self.lines;
        let i = self.cursor.min(lines.len().saturating_sub(1));
        // a blame heading belongs to the line after it
        lines[i..].iter().chain(lines[..i].iter().rev()).find_map(|l| match l.kind {
            LineKind::FileLine { line, chunk } => Some((line, chunk)),
            _ => None,
        })
    }

    pub(super) fn goto_file_line(&mut self, line: usize) {
        self.settle();
        if let Some(i) = self.lines.iter().position(|l| matches!(l.kind, LineKind::FileLine { line: n, .. } if n >= line)) {
            self.cursor = i;
        } else {
            self.cursor = self.lines.len().saturating_sub(1);
        }
    }

    /// magit-current-blame-chunk
    pub(super) fn current_blame_chunk(&self) -> Option<crate::ui::blame::Chunk> {
        let (_, chunk) = self.file_line_at_point()?;
        chunk.and_then(|c| self.buf.blame.get(c).cloned())
    }

    /// magit-blame-arguments outside the blame transient: its value
    pub(super) fn prefix_args(&self, name: &str) -> Vec<String> {
        match self.set_values.get(name).or(self.saved_values.get(name)) {
            Some(v) => v.clone(),
            None => prefix_value(name).iter().map(|s| s.to_string()).collect(),
        }
    }

    /// magit-blame-addition / -removal / -reverse with ARGS.
    pub(super) fn blame(&mut self, typ: BlameType, args: Vec<String>) -> Result<()> {
        let BufferMode::File { rev, file, blame } = self.mode.clone() else {
            self.set_msg("Buffer isn't visiting a file");
            return Ok(());
        };
        if typ != BlameType::Addition && (rev == "{worktree}" || rev == "{index}") {
            self.set_msg("Only blob buffers can be blamed in reverse");
            return Ok(());
        }
        let tracked = self.git.git_ok(&["ls-files", "--error-unmatch", "--", &file]);
        if rev == "{worktree}" && !tracked {
            self.set_msg("Buffer isn't visiting a tracked file");
            return Ok(());
        }
        // magit-blame--pre-blame-assert / -setup
        let (line, _) = self.file_line_at_point().unwrap_or((1, None));
        match blame {
            Some(b) if b.typ == typ => {
                let Some(chunk) = self.current_blame_chunk() else {
                    self.set_msg("Still blaming, commit data not available yet");
                    return Ok(());
                };
                let (Some(prev_rev), Some(prev_file)) = (chunk.prev_rev.clone(), chunk.prev_file.clone()) else {
                    self.set_msg("Chunk has no further history");
                    return Ok(());
                };
                // magit-blame-visit-other-file, keeping the style; a blob
                // buffer is read-only
                let blame = Blame { typ, style: b.style, recursive: true, args, read_only: true };
                self.setup_buffer(BufferMode::File { rev: prev_rev, file: prev_file, blame: Some(blame) }, Vec::new());
                self.goto_file_line(chunk.orig_line);
            }
            b => {
                let style = b.map_or(0, |b| b.style);
                let recursive = matches!(&self.mode, BufferMode::File { blame: Some(b), .. } if b.recursive);
                let blame = Blame { typ, style, recursive, args, read_only: true };
                self.mode = BufferMode::File { rev, file, blame: Some(blame) };
                self.refresh();
                self.goto_file_line(line);
            }
        }
        if let Some(e) = self.buf.blame_error.clone() {
            self.set_msg(&e);
        }
        Ok(())
    }

    /// magit-blame-echo: blame the additions in the lines style
    /// (magit-blame-echo-style), the summary of the chunk at point echoed,
    /// without magit-blame-read-only-mode; while blaming already, turn
    /// that mode off and use that style.
    pub(super) fn blame_echo(&mut self, args: Vec<String>) -> Result<()> {
        let BufferMode::File { rev, file, blame } = self.mode.clone() else {
            self.set_msg("Buffer isn't visiting a file");
            return Ok(());
        };
        if rev != "{worktree}" {
            self.set_msg("Blob buffers aren't supported");
            return Ok(());
        }
        let (line, _) = self.file_line_at_point().unwrap_or((1, None));
        let blame = match blame {
            Some(b) => Blame { style: ECHO_STYLE, read_only: false, ..b },
            None => {
                if !self.git.git_ok(&["ls-files", "--error-unmatch", "--", &file]) {
                    self.set_msg("Buffer isn't visiting a tracked file");
                    return Ok(());
                }
                Blame { typ: BlameType::Addition, style: ECHO_STYLE, recursive: false, args, read_only: false }
            }
        };
        self.mode = BufferMode::File { rev, file, blame: Some(blame) };
        self.refresh();
        self.goto_file_line(line);
        if let Some(e) = self.buf.blame_error.clone() {
            self.set_msg(&e);
        }
        Ok(())
    }

    /// magit-blame-quit: turn off blaming; a buffer created by a recursive
    /// blame is killed.
    pub(super) fn blame_quit(&mut self) {
        let BufferMode::File { rev, file, blame: Some(b) } = self.mode.clone() else {
            self.set_msg("Not blaming");
            return;
        };
        self.blame_previous_chunk = None;
        if b.recursive {
            self.kill_buffer();
            return;
        }
        let (line, _) = self.file_line_at_point().unwrap_or((1, None));
        self.mode = BufferMode::File { rev, file, blame: None };
        self.refresh();
        self.goto_file_line(line);
    }

    /// magit-blame-cycle-style
    pub(super) fn blame_cycle_style(&mut self) {
        let BufferMode::File { blame: Some(b), .. } = &mut self.mode else {
            self.set_msg("Not blaming");
            return;
        };
        b.style = (b.style + 1) % STYLES.len();
        let (line, _) = self.file_line_at_point().unwrap_or((1, None));
        self.refresh();
        self.goto_file_line(line);
    }

    /// magit-blame-goto-chunk-hook (in post-command-hook while blaming):
    /// when point entered another chunk, magit-blame-maybe-show-message
    /// echoes its summary if the style has show-message.
    pub(super) fn blame_post_command(&mut self) {
        let BufferMode::File { blame: Some(b), .. } = &self.mode else { return };
        let style = b.style;
        let Some(c) = self.current_blame_chunk() else { return };
        let id = (self.buffer_name_of(&self.mode), c.orig_rev.clone(), c.final_line);
        if self.blame_previous_chunk.as_ref() == Some(&id) {
            return;
        }
        self.blame_previous_chunk = Some(id);
        if style_shows_message(style) {
            let msg = c.info.get("summary").cloned().unwrap_or_default();
            self.set_msg(&format!("{msg:?}"));
        }
    }

    /// magit-blame-next-chunk / -previous-chunk; with SAME, only chunks of
    /// the commit at point (magit-blame-next-chunk-same-commit).
    pub(super) fn blame_goto_chunk(&mut self, forward: bool, same: bool) {
        let Some((_, Some(cur))) = self.file_line_at_point() else {
            self.set_msg(if same { "No more chunks from same commit" } else { "No more chunks" });
            return;
        };
        let rev = self.buf.blame[cur].orig_rev.clone();
        let wanted = |c: usize, chunks: &[crate::ui::blame::Chunk]| !same || chunks[c].orig_rev == rev;
        let chunks = &self.buf.blame;
        let target = if forward {
            (cur + 1..chunks.len()).find(|&c| wanted(c, chunks))
        } else {
            (0..cur).rev().find(|&c| wanted(c, chunks))
        };
        let Some(t) = target else {
            self.set_msg(if same { "No more chunks from same commit" } else { "No more chunks" });
            return;
        };
        // the start of the chunk: its heading, if any
        let start = self.lines.iter().position(|l| {
            matches!(l.kind, LineKind::BlameHeading { chunk } if chunk == t)
                || matches!(l.kind, LineKind::FileLine { chunk: Some(c), .. } if c == t)
        });
        if let Some(i) = start {
            self.cursor = i;
        }
    }

    /// magit-blob-previous (OLDER) / magit-blob-next
    pub(super) fn blob_visit(&mut self, older: bool) {
        let BufferMode::File { rev, file, .. } = self.mode.clone() else {
            self.set_msg("Buffer isn't visiting a file or blob");
            return;
        };
        let target = if older { blob_ancestor(&self.git, &rev, &file) } else { blob_successor(&self.git, &rev, &file) };
        let Some((r, f)) = target else {
            self.set_msg(if older { "You have reached the beginning of time" } else { "You have reached the end of time" });
            return;
        };
        let (line, _) = self.file_line_at_point().unwrap_or((1, None));
        self.find_file(&r, &f, line);
        // magit-blob-visit
        if r != "{worktree}" && r != "{index}" {
            let s = self.git.git_string(&["log", "-1", "--format=%s\x1f%ct", &r]).unwrap_or_default();
            let (subject, time) = s.trim_end().split_once('\x1f').unwrap_or((s.trim_end(), "0"));
            let (n, unit) = crate::ui::logbuf::age(crate::ui::logbuf::now() - time.parse::<f64>().unwrap_or(0.0));
            self.set_msg(&format!("{subject} ({n} {unit} ago)"));
        }
    }

    /// (revision, file) of the file or blob buffer shown
    /// (magit-buffer-revision, magit-file-relative-name).
    pub(super) fn visited_file(&self) -> Option<(String, String)> {
        match &self.mode {
            BufferMode::File { rev, file, .. } => Some((rev.clone(), file.clone())),
            _ => None,
        }
    }

    /// magit-current-file: the visited file, the file at point, or the
    /// file the log buffer is limited to.
    pub(super) fn current_file_name(&self) -> Option<String> {
        self.visited_file().map(|f| f.1).or_else(|| self.current_file().map(|f| f.1)).or_else(|| match &self.mode {
            BufferMode::Log { files, .. } => files.first().cloned(),
            _ => None,
        })
    }

    pub(super) fn is_tracked(&self, file: &str) -> bool {
        self.git.git_ok(&["ls-files", "--error-unmatch", "--", file])
    }

    /// magit-read-file (TRACKED: magit-read-tracked-file) with
    /// require-match, the default being the current file.
    pub(super) fn read_file_name(&mut self, prompt: &str, tracked: bool) -> Result<Option<String>> {
        let mut choices = self.git.git_lines(&["ls-files", "--full-name", "-z"]);
        if !tracked {
            choices.extend(self.git.untracked_files());
        }
        let default = self.current_file_name().filter(|f| choices.contains(f));
        self.read_choice(prompt, &choices, default.as_deref())
    }

    /// magit-read-file-from-rev
    pub(super) fn read_file_from_rev(&mut self, rev: &str, prompt: &str, include_dirs: bool) -> Result<Option<String>> {
        let mut choices = self.git.git_lines(&["ls-tree", "-r", "--full-tree", "--name-only", "-z", rev]);
        // magit-revision-directories
        if include_dirs {
            let dirs = self.git.git_lines(&["ls-tree", "-z", "--full-tree", "-r", "-d", "--name-only", rev]);
            choices.extend(dirs.into_iter().map(|d| format!("{d}/")));
            choices.sort();
        }
        let default = self.current_file_name().filter(|f| choices.contains(f));
        self.read_choice(prompt, &choices, default.as_deref())
    }

    /// magit-completing-read with require-match.
    pub(super) fn read_choice(&mut self, prompt: &str, choices: &[String], default: Option<&str>) -> Result<Option<String>> {
        let table = Completion { candidates: choices.to_vec(), require_match: true, multiple: false };
        let Some(v) = self.completing_read(prompt, default, Hist::Minibuffer, Some(table))?.filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        if choices.contains(&v) {
            Ok(Some(v))
        } else {
            self.set_msg("[No match]");
            Ok(None)
        }
    }

    /// magit-branch-or-commit-at-point, else the current branch.
    pub(super) fn branch_or_commit_default(&self) -> Option<String> {
        match self.current_line().map(|l| l.kind.clone()) {
            Some(LineKind::Commit { hash, .. }) => Some(self.git.rev_short(&hash)),
            Some(LineKind::Ref { name, .. }) => Some(name),
            _ => self.git.current_branch(),
        }
    }

    /// The files of the selected file sections (magit-region-values 'file).
    pub(super) fn region_files(&self) -> Option<Vec<String>> {
        let secs = self.region_sections(true)?;
        secs.iter()
            .map(|&s| match &self.buf.all[self.sec(s).start].kind {
                LineKind::File { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    /// The file commands of magit-file-dispatch (magit-files.el).
    pub(super) fn file_command(&mut self, cmd: &str) -> Result<()> {
        match cmd {
            // magit-file-stage / magit-file-unstage
            "file-stage" | "file-unstage" => {
                let Some((_, file)) = self.visited_file().filter(|(r, _)| r == "{worktree}") else {
                    self.set_msg("Not visiting a file");
                    return Ok(());
                };
                if cmd == "file-unstage" {
                    self.git.unstage(&file)?;
                    self.set_msg("unstaged");
                } else {
                    let mut a = vec!["add"];
                    if self.git.git_ok(&["check-ignore", "-q", "--", &file]) {
                        if !self.yes_no("Visited file is ignored; stage anyway?")? {
                            self.set_msg("Abort");
                            return Ok(());
                        }
                        a.push("--force");
                    }
                    a.extend(["--", &file]);
                    self.git.git_interactive(&a)?;
                    self.set_msg("staged");
                }
                self.refresh();
            }
            // magit-file-untrack
            "file-untrack" => {
                let files = match self.region_files() {
                    Some(files) => {
                        if !self.is_tracked(&files[0]) {
                            self.set_msg("Already untracked");
                            return Ok(());
                        }
                        if !self.confirm_files("untrack", "Untrack", &files)? {
                            return Ok(());
                        }
                        files
                    }
                    None => match self.read_file_name("Untrack file", true)? {
                        Some(f) => vec![f],
                        None => return Ok(()),
                    },
                };
                // with a prefix argument even files with staged and
                // unstaged changes
                let mut a = vec!["rm", "--cached"];
                if self.prefix_arg.is_some() {
                    a.push("--force");
                }
                a.push("--");
                a.extend(files.iter().map(String::as_str));
                let res = self.git.git_interactive(&a);
                self.done("untracked", res.map(|()| String::new()));
            }
            // magit-file-rename
            "file-rename" => {
                let Some(file) = self.read_file_name("Rename file", false)? else {
                    return Ok(());
                };
                let dir = match file.rfind('/') {
                    Some(i) => file[..=i].to_string(),
                    None => String::new(),
                };
                let prompt = format!("Move {file} to destination");
                let Some(newname) = self.read_line_prompt(&prompt, Some(&dir).filter(|d| !d.is_empty()).map(|d| d.as_str()))?
                else {
                    return Ok(());
                };
                let top = self.git.toplevel.clone();
                let base = file.rsplit('/').next().unwrap_or(&file).to_string();
                let dst = if newname.ends_with('/') || top.join(&newname).is_dir() {
                    format!("{}/{base}", newname.trim_end_matches('/'))
                } else {
                    newname
                };
                if top.join(&dst).exists() {
                    self.set_msg(&format!("{dst} already exists"));
                    return Ok(());
                }
                let dstdir = top.join(&dst).parent().map(std::path::Path::to_path_buf).unwrap_or(top.clone());
                if !dstdir.is_dir() {
                    self.set_msg(&format!("Destination directory {} does not exist", dstdir.display()));
                    return Ok(());
                }
                if self.is_tracked(&file) {
                    self.git.git_interactive(&["mv", &file, &dst])?;
                } else {
                    std::fs::rename(top.join(&file), top.join(&dst))?;
                }
                // the buffer visiting the file now visits the new name
                if let BufferMode::File { rev, file: f, .. } = &mut self.mode {
                    if rev == "{worktree}" && *f == file {
                        *f = dst.clone();
                    }
                }
                self.set_msg(&format!("renamed {file} to {dst}"));
                self.refresh();
            }
            // magit-file-delete
            "file-delete" => {
                let files = match self.region_files() {
                    Some(files) => {
                        if !self.confirm_files("delete", "Delete", &files)? {
                            return Ok(());
                        }
                        files
                    }
                    None => match self.read_file_name("Delete file", false)? {
                        Some(f) => vec![f],
                        None => return Ok(()),
                    },
                };
                let top = self.git.toplevel.clone();
                let dirs: Vec<&String> = files.iter().filter(|f| top.join(f).is_dir()).collect();
                // with a prefix argument even files with uncommitted changes
                let mut args = vec!["rm"];
                if self.prefix_arg.is_some() {
                    args.push("--force");
                }
                if !dirs.is_empty() {
                    let what = if dirs.len() == 1 { "directory" } else { "directories" };
                    let names = dirs.iter().map(|d| d.as_str()).collect::<Vec<_>>().join(",");
                    if !self.yes_or_no_p(&format!("Recursively delete {what} \"{names}\":"))? {
                        self.set_msg("Abort");
                        return Ok(());
                    }
                    args.push("-r");
                }
                if self.is_tracked(&files[0]) {
                    args.push("--");
                    args.extend(files.iter().map(String::as_str));
                    let res = self.git.git_interactive(&args);
                    self.done("deleted", res.map(|()| String::new()));
                } else {
                    for f in &files {
                        let p = top.join(f);
                        if p.is_dir() {
                            std::fs::remove_dir_all(p)?;
                        } else {
                            std::fs::remove_file(p)?;
                        }
                    }
                    self.set_msg("deleted");
                    self.refresh();
                }
            }
            // magit-file-checkout
            "file-checkout" => {
                let default = match self.visited_file() {
                    Some((r, _)) if r != "{worktree}" && r != "{index}" => Some(r),
                    _ => self.branch_or_commit_default(),
                };
                let Some(rev) = self.read_rev("Checkout from revision", default.as_deref())? else {
                    return Ok(());
                };
                let Some(file) = self.read_file_from_rev(&rev, "Checkout file", true)? else {
                    return Ok(());
                };
                let res = self.git.git_interactive(&["checkout", &rev, "--", &file]);
                self.done("checked out", res.map(|()| String::new()));
            }
            // magit-diff-buffer-file: the commit of a blob, else the
            // changes to the file since the current branch
            "diff-buffer-file" => {
                let Some((rev, file)) = self.visited_file() else {
                    self.set_msg("Buffer isn't visiting a file");
                    return Ok(());
                };
                if rev != "{worktree}" && rev != "{index}" {
                    let args = match &self.transient_diff_args {
                        Some(a) => a.clone(),
                        None => self.diff_args.clone(),
                    };
                    self.setup_buffer(BufferMode::Revision { rev, files: vec![file] }, args);
                } else {
                    let line = self.file_line().unwrap_or(1);
                    let range = self.git.current_branch().unwrap_or_else(|| "HEAD".into());
                    let spec = DiffSpec { range: Some(range), typearg: None, typ: "unstaged".into(), files: vec![file.clone()] };
                    let args = self.transient_diff_args.clone().unwrap_or_else(|| self.diff_args.clone());
                    self.setup_buffer(BufferMode::Diff(spec), args);
                    self.goto_file_position(&file, line, &[]);
                }
            }
            // magit-log-buffer-file: with a region, the log of its lines
            "log-buffer-file" | "log-trace-definition" => {
                let Some((rev, file)) = self.visited_file() else {
                    self.set_msg("Buffer isn't visiting a file");
                    return Ok(());
                };
                let rev = if rev == "{worktree}" || rev == "{index}" {
                    self.git.current_branch().unwrap_or_else(|| "HEAD".into())
                } else {
                    rev
                };
                let mut args = self.log_arguments();
                // with a prefix argument follow renames
                if cmd == "log-buffer-file" && self.prefix_arg.is_some() && !args.iter().any(|a| a == "--follow") {
                    args.insert(0, "--follow".into());
                }
                let mut files = vec![file.clone()];
                let lines = if cmd == "log-trace-definition" {
                    let Some(name) = self.which_function() else {
                        self.set_msg("No function at point found");
                        return Ok(());
                    };
                    Some(format!("-L:{}:{file}", regex_quote(&name).replace(':', "\\:")))
                } else {
                    self.mark.take().and_then(|m| {
                        let line_at = |p: usize| match self.buf.all.get(p).map(|l| &l.kind) {
                            Some(LineKind::FileLine { line, .. }) => Some(*line),
                            _ => None,
                        };
                        let (a, b) = (line_at(m)?, self.file_line()?);
                        Some(format!("-L{},{}:{file}", a.min(b), a.max(b)))
                    })
                };
                if let Some(l) = lines {
                    args.retain(|a| !a.starts_with("-L"));
                    args.insert(0, l);
                    files.clear();
                }
                self.setup_buffer(BufferMode::Log { revs: vec![rev], files }, args);
            }
            "blob-previous" => self.blob_visit(true),
            "blob-next" => self.blob_visit(false),
            // magit-find-file
            "find-file" => {
                let default = self.branch_or_commit_default();
                let Some(rev) = self.read_rev("Find file from revision", default.as_deref())? else {
                    return Ok(());
                };
                let from = if rev == "{worktree}" || rev == "{index}" { "HEAD" } else { rev.as_str() };
                let Some(file) = self.read_file_from_rev(from, "Find file", false)? else {
                    return Ok(());
                };
                let line = match self.visited_file() {
                    Some((_, f)) if f == file => self.file_line().unwrap_or(1),
                    _ => 1,
                };
                if let Err(e) = crate::ui::blame::file_contents(&self.git, &rev, &file) {
                    self.set_msg(&format!("{e:#}"));
                    return Ok(());
                }
                self.find_file(&rev, &file, line);
            }
            // magit-blob-visit-file
            "blob-visit-file" => {
                let Some((_, file)) = self.visited_file() else {
                    self.set_msg("Not visiting a blob");
                    return Ok(());
                };
                let line = self.file_line().unwrap_or(1);
                self.find_file("{worktree}", &file, line);
            }
            // magit-status-here: the status buffer, at the visited
            // position in the diff of its unstaged or staged changes
            "status-here" => {
                let pos = self.visited_file().map(|(_, f)| (f, self.file_line().unwrap_or(1)));
                self.switch_to_mode(BufferMode::Status);
                if let Some((file, line)) = pos {
                    self.goto_file_position(&file, line, &["unstaged", "staged"]);
                }
            }
            "edit-line-commit" => self.edit_line_commit()?,
            _ => {}
        }
        Ok(())
    }

}
