//! ui/view.rs — the status buffer, built like magit-status-sections-hook:
//! headers, merge/rebase/am/sequencer/bisect sequences, untracked files,
//! unstaged and staged changes (with expandable file / hunk sections),
//! stashes, and the unpushed / unpulled / recent commit sections.
//!
//! Every section has an identity; its visibility (hidden or shown) is kept
//! across refreshes by the caller, and the initial visibility follows the
//! HIDE argument Magit passes to `magit-insert-section`.

use super::logbuf::{commit_limit, now, section_log, shortstats, wash_section_log, LogSection, Margin};
use crate::git::diff::FileDiff;
use crate::git::status::{FileEntry, Status};
use crate::git::Git;
use std::collections::HashMap;

/// What a buffer line is part of.
#[derive(Debug, Clone, PartialEq)]
pub enum LineKind {
    /// a header line (Head:, Merge:, Rebase:, Push:, Tag:)
    Head,
    SectionHeader { section: String, n: usize },
    File { section: String, path: String },
    Hunk { section: String, path: String, hunk: usize },
    /// body line LINE of hunk HUNK
    DiffLine { section: String, path: String, hunk: usize, line: usize },
    /// the `chmod` or `rename` pseudo-hunk (KIND) of a file
    Meta { section: String, path: String, kind: String },
    Stash { idx: usize, name: String, msg: String },
    Commit { section: String, hash: String },
    /// line LINE (1-based) of a file or blob buffer, in blame chunk CHUNK
    FileLine { line: usize, chunk: Option<usize> },
    /// the heading (or separator) shown before blame chunk CHUNK
    BlameHeading { chunk: usize },
    /// a branch or tag section of the refs buffer
    Ref { kind: super::refs::RefKind, name: String },
    /// the heading of a remote's section in the refs buffer
    Remote { name: String },
    Info,
}

#[derive(Debug, Clone)]
pub struct BufferLine {
    pub text: String,
    pub kind: LineKind,
    /// identity of the innermost section containing this line
    pub ident: String,
    /// first line of its section (where n / p stop)
    pub heading: bool,
    /// nesting level of that section (0 = top level)
    pub depth: usize,
    /// heading of a section that has a body (can be expanded / collapsed)
    pub has_body: bool,
    /// sections whose body contains this line, outermost first
    pub ancestors: Vec<String>,
    /// for visible headings with a body: whether the body is hidden
    pub hidden: Option<bool>,
    /// index of this line in `StatusBuffer::all`
    pub pos: usize,
    /// for headings with a body: hidden when first shown (the HIDE argument
    /// of magit-insert-section)
    pub hide: bool,
    /// text of the right margin (magit-make-margin-overlay), e.g. the
    /// author and age of a commit in log buffers
    pub margin: String,
}

/// A section of the buffer (magit-section): its heading line, extent,
/// parent and children. Index 0 is the root section (the whole buffer).
#[derive(Debug, Clone, Default)]
pub struct Section {
    pub ident: String,
    /// heading line (index into `all`)
    pub start: usize,
    /// first line after the section
    pub end: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// has a body (`content` in magit-section)
    pub content: bool,
    /// (1- (length (magit-section-ident s))): root 0, top-level sections 1
    pub level: usize,
}

/// Section identity -> hidden.
pub type Visibility = HashMap<String, bool>;

/// A Magit buffer (status, diff, revision, stash): every section expanded,
/// the diffs its file sections came from, and its header line.
#[derive(Debug, Clone, Default)]
pub struct StatusBuffer {
    pub all: Vec<BufferLine>,
    /// diff group (the `section` of file / hunk lines) -> its files
    pub diffs: Vec<(String, Vec<FileDiff>)>,
    /// header-line-format of diff / revision / stash buffers
    pub title: String,
    /// section tree; 0 is the root
    pub sections: Vec<Section>,
    /// innermost section of each line of `all`
    pub line_section: Vec<usize>,
    /// initial visibility of the sections that have a body
    defaults: HashMap<String, bool>,
    /// width of the right margin (0: no margin, as in the status buffer)
    pub margin_width: usize,
    /// the blame chunks of a file or blob buffer
    pub blame: Vec<super::blame::Chunk>,
    /// why blaming the file failed
    pub blame_error: Option<String>,
    /// sections whose body is only inserted once they are shown
    /// (magit-insert-section-body)
    pub deferred: Vec<String>,
}

impl StatusBuffer {
    pub(crate) fn new(all: Vec<BufferLine>, diffs: Vec<(String, Vec<FileDiff>)>, title: String) -> StatusBuffer {
        let (sections, line_section) = index_sections(&all);
        let defaults = all.iter().filter(|l| l.heading && l.has_body).map(|l| (l.ident.clone(), l.hide)).collect();
        StatusBuffer {
            all,
            diffs,
            title,
            sections,
            line_section,
            defaults,
            margin_width: 0,
            blame: Vec::new(),
            blame_error: None,
            deferred: Vec::new(),
        }
    }

    /// Whether the section IDENT is hidden under VIS (else its initial state).
    pub fn is_hidden(&self, vis: &Visibility, ident: &str) -> bool {
        vis.get(ident).copied().unwrap_or_else(|| self.defaults.get(ident).copied().unwrap_or(false))
    }

    /// The files of diff group GROUP.
    pub fn files(&self, group: &str) -> &[FileDiff] {
        self.diffs.iter().find(|(g, _)| g == group).map_or(&[], |(_, f)| f.as_slice())
    }

    /// Section containing line P (0 = root).
    pub fn section_at(&self, p: usize) -> usize {
        self.line_section.get(p).copied().unwrap_or(0)
    }

    /// Whether line P is visible under VIS.
    pub fn is_visible(&self, vis: &Visibility, p: usize) -> bool {
        self.all.get(p).is_some_and(|l| !l.ancestors.iter().any(|a| self.is_hidden(vis, a)))
    }

    pub fn diff(&self, section: &str, path: &str) -> Option<&FileDiff> {
        self.files(section).iter().find(|f| f.path == path)
    }

    /// The lines shown when sections are collapsed according to VIS.
    pub fn visible(&self, vis: &Visibility) -> Vec<BufferLine> {
        self.all
            .iter()
            .filter(|l| !l.ancestors.iter().any(|a| self.is_hidden(vis, a)))
            .map(|l| {
                let mut l = l.clone();
                if l.has_body {
                    l.hidden = Some(self.is_hidden(vis, &l.ident));
                }
                l
            })
            .collect()
    }
}

/// magit-log-section-commit-count
pub const RECENT_COMMIT_COUNT: usize = 10;
/// "-n256" of magit-log-section-arguments
pub const LOG_SECTION_LIMIT: usize = 256;
/// magit-status-file-list-limit
pub const FILE_LIST_LIMIT: usize = 100;

/// Hidden-by-default for a section identity (Magit's HIDE arguments and
/// magit-section-initial-visibility-alist '((stashes . hide))).
pub fn default_hidden(ident: &str) -> bool {
    match ident {
        "untracked" | "stashes" | "recent" => true,
        _ if ident.starts_with("unpulled:") => true,
        "unpushed:@{push}.." => true,
        // file sections in the status buffer are hidden (magit-diff-insert-file-section),
        // their hunks are shown
        _ if ident.starts_with("unstaged/") || ident.starts_with("staged/") => !ident.contains("/@@ "),
        _ => false,
    }
}

/// Identity of a file section.
pub fn file_ident(section: &str, path: &str) -> String {
    format!("{section}/{path}")
}

/// Identity of a hunk section.
pub fn hunk_ident(section: &str, path: &str, header: &str) -> String {
    let ranges = header.splitn(3, "@@").nth(1).unwrap_or(header).trim();
    format!("{section}/{path}/@@ {ranges} @@")
}

/// Status buffer with default visibility (for tests and `J`).
pub fn render_status(g: &Git, st: &Status) -> Vec<BufferLine> {
    build_status(g, st, &[], &status_log_args(), &Margin::status()).visible(&Visibility::new())
}

/// magit-status-mode's magit-log-default-arguments
pub fn status_log_args() -> Vec<String> {
    vec![format!("-n{LOG_SECTION_LIMIT}"), "--decorate".into()]
}

/// Build the section tree from the lines' headings and ancestors.
fn index_sections(all: &[BufferLine]) -> (Vec<Section>, Vec<usize>) {
    let mut secs = vec![Section { end: all.len(), content: true, ..Section::default() }];
    let mut stack = vec![0usize];
    let mut of_line = Vec::with_capacity(all.len());
    for (i, l) in all.iter().enumerate() {
        stack.truncate(1 + l.ancestors.len());
        let s = if l.heading {
            let parent = *stack.last().unwrap_or(&0);
            let idx = secs.len();
            secs.push(Section {
                ident: l.ident.clone(),
                start: i,
                end: i + 1,
                parent: Some(parent),
                children: Vec::new(),
                content: l.has_body,
                level: stack.len(),
            });
            secs[parent].children.push(idx);
            if l.has_body {
                stack.push(idx);
            }
            idx
        } else {
            *stack.last().unwrap_or(&0)
        };
        of_line.push(s);
        let mut cur = Some(s);
        while let Some(c) = cur.filter(|&c| c != 0) {
            secs[c].end = secs[c].end.max(i + 1);
            cur = secs[c].parent;
        }
    }
    (secs, of_line)
}

/// Inserts sections in buffer order; DEPTH arguments give the nesting, from
/// which the ancestors of every line follow.
pub(crate) struct Builder {
    pub(crate) lines: Vec<BufferLine>,
    /// idents of the sections whose body is being inserted
    stack: Vec<String>,
    /// the margin of the commits of log sections (magit-status-margin)
    pub(crate) log_margin: Margin,
}

impl Builder {
    pub(crate) fn new() -> Builder {
        Builder { lines: Vec::new(), stack: Vec::new(), log_margin: Margin::status() }
    }

    fn push(&mut self, text: String, kind: LineKind, ident: &str, heading: bool, depth: usize, has_body: bool, ancestors: Vec<String>) {
        self.lines.push(BufferLine {
            text,
            kind,
            ident: ident.to_string(),
            heading,
            depth,
            has_body,
            ancestors,
            hidden: None,
            pos: self.lines.len(),
            hide: false,
            margin: String::new(),
        });
    }

    /// Set the margin of the line inserted last.
    pub(crate) fn margin(&mut self, text: String) {
        if let Some(l) = self.lines.last_mut() {
            l.margin = text;
        }
    }

    /// A heading without body (HEADING) or a body line of the section at DEPTH.
    pub(crate) fn line(&mut self, text: String, kind: LineKind, ident: &str, heading: bool, depth: usize) {
        if heading {
            self.stack.truncate(depth);
            let anc = self.stack.clone();
            self.push(text, kind, ident, true, depth, false, anc);
        } else {
            let anc = self.stack[..self.stack.len().min(depth + 1)].to_vec();
            self.push(text, kind, ident, false, depth, false, anc);
        }
    }

    /// Insert a heading at DEPTH; its body follows. Returns true (the body is
    /// always inserted; visibility is applied when the buffer is shown).
    fn heading(&mut self, text: String, kind: LineKind, ident: &str, depth: usize, has_body: bool) -> bool {
        self.heading_hide(text, kind, ident, depth, has_body, default_hidden(ident))
    }

    /// `heading` with an explicit initial visibility HIDE.
    pub(crate) fn heading_hide(&mut self, text: String, kind: LineKind, ident: &str, depth: usize, has_body: bool, hide: bool) -> bool {
        self.stack.truncate(depth);
        let anc = self.stack.clone();
        self.push(text, kind, ident, true, depth, has_body, anc);
        if let Some(l) = self.lines.last_mut() {
            l.hide = hide && has_body;
        }
        if has_body {
            self.stack.push(ident.to_string());
        }
        true
    }

    /// A further line of a multi-line heading (of a section without body at
    /// DEPTH); it is shown and hidden together with that heading.
    pub(crate) fn heading_cont(&mut self, text: String, kind: LineKind, ident: &str, depth: usize) {
        self.stack.truncate(depth);
        let anc = self.stack.clone();
        self.push(text, kind, ident, false, depth, false, anc);
    }

    /// An empty line in the body of the sections open at DEPTH (DEPTH 0:
    /// the root).
    pub(crate) fn empty(&mut self, depth: usize) {
        self.stack.truncate(depth);
        let anc = self.stack.clone();
        let ident = anc.last().cloned().unwrap_or_default();
        self.push(String::new(), LineKind::Info, &ident, false, depth.saturating_sub(1), false, anc);
    }

    /// The newline ending a top-level section (part of its body, as in Magit).
    fn blank(&mut self) {
        let anc = self.stack[..self.stack.len().min(1)].to_vec();
        let ident = anc.first().cloned().unwrap_or_default();
        self.push(String::new(), LineKind::Info, &ident, false, 0, false, anc);
    }

    /// A newline that belongs to the root section (after the headers).
    pub(crate) fn root_blank(&mut self) {
        self.stack.clear();
        self.push(String::new(), LineKind::Info, "", false, 0, false, Vec::new());
    }

    /// A top-level section of commits (log or sequence lines), with the
    /// margin of each commit (magit-log-format-margin).
    fn commit_section(&mut self, g: &Git, ident: &str, title: String, section: &str, commits: &[LogCommit]) {
        let n = commits.len();
        self.heading(title, LineKind::SectionHeader { section: section.into(), n }, ident, 0, n > 0);
        let revs: Vec<&str> = commits.iter().map(|c| c.hash.as_str()).collect();
        let stats = shortstats(g, &self.log_margin, &revs);
        let now = now();
        for c in commits {
            let id = format!("{ident}/{}", c.hash);
            let kind = LineKind::Commit { section: section.into(), hash: c.hash.clone() };
            self.line(c.text.clone(), kind, &id, true, 1);
            let m = self.log_margin.format(&c.hash, &c.author, &c.date, now, &stats);
            self.margin(m);
        }
        self.blank();
    }

    /// A top-level log section of the status buffer (magit--insert-log
    /// with the buffer's log ARGS), omitted when it has no commits. Other
    /// than "Recent commits", TITLE is followed by the number of commits
    /// (magit-log-insert-child-count), "N+" when that is the limit.
    fn log_section(&mut self, g: &Git, ident: &str, title: &str, section: &str, revs: Option<&str>, args: &[String]) {
        let (out, n) = section_log(g, revs, args);
        if n == 0 {
            return;
        }
        let title = if section != "recent" {
            let plus = if Some(n) == commit_limit(args) { "+" } else { "" };
            format!("{title} ({n}{plus})")
        } else {
            title.to_string()
        };
        self.heading(title, LineKind::SectionHeader { section: section.into(), n }, ident, 0, true);
        let margin = self.log_margin.clone();
        let ident = format!("{ident}/");
        wash_section_log(self, g, &out, &margin, &LogSection { depth: 1, section, ident: &ident });
        self.blank();
    }
}

/// Build the status buffer (magit-status-refresh-buffer); MARGIN is
/// magit-status-margin (shown for the commits and stashes), LOG_ARGS the
/// buffer's log arguments (magit-buffer-log-args).
pub fn build_status(g: &Git, st: &Status, diff_args: &[String], log_args: &[String], margin: &Margin) -> StatusBuffer {
    let mut b = Builder::new();
    b.log_margin = margin.clone();
    let unstaged = g.diff_sections(false, diff_args).unwrap_or_default();
    let staged = g.diff_sections(true, diff_args).unwrap_or_default();

    insert_headers(&mut b, g);
    insert_merge_log(&mut b, g);
    insert_rebase_sequence(&mut b, g);
    insert_am_sequence(&mut b, g);
    insert_sequencer_sequence(&mut b, g);
    insert_bisect(&mut b, g);
    insert_untracked(&mut b, &st.untracked);
    insert_changes(&mut b, "unstaged", "Unstaged changes", &st.unstaged, &unstaged);
    insert_changes(&mut b, "staged", "Staged changes", &st.staged, &staged);
    insert_stashes(&mut b, g, &st.stashes);
    insert_log_sections(&mut b, g, log_args);

    while b.lines.last().is_some_and(|l| l.text.is_empty() && l.kind == LineKind::Info) {
        b.lines.pop();
    }
    let mut buf = StatusBuffer::new(b.lines, vec![("unstaged".into(), unstaged), ("staged".into(), staged)], String::new());
    buf.margin_width = margin.shown_width();
    buf
}

// ---------------------------------------------------------------- headers

fn rev_summary(g: &Git, rev: &str) -> Option<(String, String)> {
    let out = g.git_string(&["log", "-1", "--no-walk", "--format=%h%x00%s", rev, "--"]).ok()?;
    let (h, s) = out.split_once('\0')?;
    let s = if s.is_empty() { "(no commit message)" } else { s };
    Some((h.to_string(), s.to_string()))
}

fn config(g: &Git, key: &str) -> Option<String> {
    g.git_string(&["config", key]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn config_bool(g: &Git, key: &str) -> bool {
    g.git_string(&["config", "--type=bool", key]).map(|s| s.trim() == "true").unwrap_or(false)
}

/// magit-get-push-branch
pub fn push_branch(g: &Git, branch: &str) -> Option<String> {
    let remote = config(g, &format!("branch.{branch}.pushRemote")).or_else(|| config(g, "remote.pushDefault"))?;
    Some(format!("{remote}/{branch}"))
}

fn rev_exists(g: &Git, rev: &str) -> bool {
    g.git_ok(&["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")])
}

/// magit-insert-head-branch-header for BRANCH (or a detached HEAD).
pub(crate) fn head_header(g: &Git, branch: Option<&str>) -> String {
    let (hash, summary) = rev_summary(g, branch.unwrap_or("HEAD")).unwrap_or_default();
    format!("{:<10}{} {summary}", "Head: ", branch.unwrap_or(&hash))
}

/// magit-insert-upstream-branch-header: nothing unless BRANCH has an
/// upstream configured; UPSTREAM is shown if it exists.
pub(crate) fn upstream_header(g: &Git, br: &str, upstream: Option<&str>, keyword: Option<&str>) -> Option<String> {
    let remote = config(g, &format!("branch.{br}.remote"));
    let merge = config(g, &format!("branch.{br}.merge"));
    if remote.is_none() && merge.is_none() {
        return None;
    }
    let rebase = match config(g, &format!("branch.{br}.rebase")).as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => config_bool(g, "pull.rebase"),
    };
    let keyword = keyword.unwrap_or(if rebase { "Rebase: " } else { "Merge: " });
    let rest = match (upstream, &remote, &merge) {
        (Some(up), _, _) => {
            let s = rev_summary(g, up).map(|(_, s)| s).unwrap_or_else(|| "(no commit message)".into());
            format!("{up} {s}")
        }
        (None, Some(r), Some(m)) if r == "." => format!("{m} does not exist"),
        (None, Some(r), Some(m)) if g.remotes().contains(r) => format!("{m} does not exist on {r}"),
        (None, Some(r), Some(m)) => format!("{m} from {r}"),
        _ => "invalid upstream configuration".into(),
    };
    Some(format!("{keyword:<10}{rest}"))
}

fn insert_headers(b: &mut Builder, g: &Git) {
    if !rev_exists(g, "HEAD") {
        b.line("In the beginning there was darkness".into(), LineKind::Info, "", false, 0);
        b.root_blank();
        return;
    }
    let mut headers: Vec<(String, &str)> = Vec::new();
    let branch = g.current_branch();
    headers.push((head_header(g, branch.as_deref()), "header:head"));

    if let Some(br) = &branch {
        if let Some(text) = upstream_header(g, br, g.upstream_shortname().as_deref(), None) {
            headers.push((text, "header:upstream"));
        }
        // magit-insert-push-branch-header
        if let Some(target) = push_branch(g, br) {
            let rest = if rev_exists(g, &target) {
                let s = rev_summary(g, &target).map(|(_, s)| s).unwrap_or_default();
                format!("{target} {s}")
            } else {
                let remote = target.split('/').next().unwrap_or("").to_string();
                if g.remotes().contains(&remote) {
                    format!("{target} does not exist")
                } else {
                    format!("{remote} remote does not exist")
                }
            };
            headers.push((format!("{:<10}{rest}", "Push: "), "header:push"));
        }
    }

    // magit-insert-tags-header
    let this = current_tag(g);
    let next = next_tag(g, this.as_ref().map(|t| t.0.as_str()));
    if this.is_some() || next.is_some() {
        let fmt = |(t, n): &(String, usize)| if *n > 0 { format!("{t} ({n})") } else { t.clone() };
        let both = this.is_some() && next.is_some();
        let parts: Vec<String> = this.iter().chain(next.iter()).map(fmt).collect();
        let kw = if both { "Tags: " } else { "Tag: " };
        headers.push((format!("{kw:<10}{}", parts.join(", ")), "header:tag"));
    }
    // magit-insert-headers: the first header is made the parent of the rest
    let n = headers.len();
    for (i, (text, ident)) in headers.into_iter().enumerate() {
        if i == 0 {
            b.heading(text, LineKind::Head, ident, 0, n > 1);
        } else {
            b.line(text, LineKind::Head, ident, true, 1);
        }
    }
    b.root_blank();
}

/// magit-get-current-tag with distance: `git describe --long --tags`
pub fn current_tag(g: &Git) -> Option<(String, usize)> {
    let s = g.git_string(&["describe", "--long", "--tags"]).ok()?;
    let s = s.trim();
    let (rest, _hash) = s.rsplit_once("-g")?;
    let (tag, n) = rest.rsplit_once('-')?;
    Some((tag.to_string(), n.parse().unwrap_or(0)))
}

/// magit-get-next-tag with distance: `git describe --contains HEAD`
pub fn next_tag(g: &Git, current: Option<&str>) -> Option<(String, usize)> {
    let s = g.git_string(&["describe", "--contains", "HEAD"]).ok()?;
    let tag: String = s.trim().chars().take_while(|c| *c != '~' && *c != '^').collect();
    if tag.is_empty() || Some(tag.as_str()) == current {
        return None;
    }
    let n = g
        .git_string(&["rev-list", "--count", &format!("HEAD..{tag}")])
        .ok()
        .and_then(|c| c.trim().parse().ok())
        .unwrap_or(0);
    Some((tag, n))
}

// ---------------------------------------------------------------- logs

/// Ref labels like Magit's log lines: "HEAD -> main" shows as "main",
/// "tag: v1" as "v1".
fn ref_labels(d: &str) -> String {
    d.split(", ")
        .filter(|r| !r.is_empty() && *r != "HEAD")
        .map(|r| r.strip_prefix("HEAD -> ").or_else(|| r.strip_prefix("tag: ")).unwrap_or(r))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A commit line of a log section.
pub struct LogCommit {
    pub hash: String,
    /// "abbrev refs subject"
    pub text: String,
    pub author: String,
    /// author date, seconds since the epoch
    pub date: String,
}

/// magit--insert-log for status sections.
pub fn log_lines(g: &Git, range: Option<&str>, limit: usize, extra: &[&str]) -> Vec<LogCommit> {
    let n = format!("-n{limit}");
    let mut a: Vec<&str> = vec!["log", "--format=%H%x00%h%x00%D%x00%aN%x00%at%x00%s", "--use-mailmap", &n];
    a.extend(extra);
    a.extend(range);
    a.push("--");
    let Ok(out) = g.git_string(&a) else { return Vec::new() };
    out.lines()
        .filter_map(|l| {
            let mut p = l.splitn(6, '\0');
            let (full, h, d) = (p.next()?, p.next()?, p.next()?);
            let (author, date, s) = (p.next()?, p.next()?, p.next().unwrap_or(""));
            let refs = ref_labels(d);
            let text = if refs.is_empty() { format!("{h} {s}") } else { format!("{h} {refs} {s}") };
            Some(LogCommit { hash: full.to_string(), text, author: author.to_string(), date: date.to_string() })
        })
        .collect()
}

fn insert_log_sections(b: &mut Builder, g: &Git, args: &[String]) {
    let Some(branch) = g.current_branch() else {
        // detached HEAD: no upstream, only recent commits
        insert_recent(b, g, args);
        return;
    };
    let upstream = g.upstream_shortname();
    let push = push_branch(g, &branch).filter(|t| rev_exists(g, t));
    // magit--insert-pushremote-log-p
    let pushremote_log = push.is_some() && push != upstream;

    if pushremote_log {
        let t = push.clone().unwrap_or_default();
        let range = format!("{t}..");
        b.log_section(g, "unpushed:@{push}..", &format!("Unpushed to {t}"), "unpushed", Some(&range), args);
    }
    // magit-insert-unpushed-to-upstream-or-recent
    match &upstream {
        Some(up) if !g.git_ok(&["merge-base", "--is-ancestor", "HEAD", up]) => {
            let title = format!("Unmerged into {up}");
            b.log_section(g, "unpushed:@{upstream}..", &title, "unpushed", Some("@{upstream}.."), args);
        }
        _ => insert_recent(b, g, args),
    }
    if pushremote_log {
        let t = push.unwrap_or_default();
        let range = format!("..{t}");
        b.log_section(g, "unpulled:..@{push}", &format!("Unpulled from {t}"), "unpulled", Some(&range), args);
    }
    if let Some(up) = &upstream {
        let title = format!("Unpulled from {up}");
        b.log_section(g, "unpulled:..@{upstream}", &title, "unpulled", Some("..@{upstream}"), args);
    }
}

/// magit-insert-recent-commits: the last magit-log-section-commit-count
/// commits, with the buffer's arguments other than the limit.
fn insert_recent(b: &mut Builder, g: &Git, args: &[String]) {
    let start = format!("HEAD~{RECENT_COMMIT_COUNT}");
    let range = rev_exists(g, &start).then(|| format!("{start}..HEAD"));
    let revs = range.filter(|_| args.iter().any(|a| a == "--graph"));
    let mut a = vec![format!("-n{RECENT_COMMIT_COUNT}")];
    a.extend(args.iter().filter(|x| !x.starts_with("-n")).cloned());
    b.log_section(g, "recent", "Recent commits", "recent", revs.as_deref(), &a);
}

// ---------------------------------------------------------------- sequences

fn file_line(g: &Git, rel: &str) -> Option<String> {
    let s = std::fs::read_to_string(g.gitdir.join(rel)).ok()?;
    s.lines().next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
}

fn file_lines(g: &Git, rel: &str) -> Vec<String> {
    std::fs::read_to_string(g.gitdir.join(rel))
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// magit-get-shortname
fn shortname(g: &Git, rev: &str) -> String {
    for refs in ["refs/tags/*", "refs/heads/*", "refs/remotes/*"] {
        if let Ok(n) = g.git_string(&["name-rev", "--name-only", "--no-undefined", &format!("--refs={refs}"), rev]) {
            let n = n.trim();
            if !n.is_empty() {
                let n = n.strip_prefix("tags/").or_else(|| n.strip_prefix("remotes/")).unwrap_or(n);
                return n.to_string();
            }
        }
    }
    g.rev_short(rev)
}

fn summary_line(g: &Git, kind: &str, rev: &str) -> (String, String) {
    let full = g.rev_parse(rev).unwrap_or_else(|_| rev.to_string());
    let text = match rev_summary(g, rev) {
        Some((h, s)) => format!("{kind} {h} {s}"),
        None => format!("{kind} {rev}"),
    };
    (full, text)
}

/// magit-insert-merge-log: "Merging <heads> (N)" with the merged commits.
fn insert_merge_log(b: &mut Builder, g: &Git) {
    if !g.merge_in_progress() {
        return;
    }
    let heads: Vec<String> = file_lines(g, "MERGE_HEAD").iter().filter(|l| !l.is_empty()).map(|h| shortname(g, h)).collect();
    let Some(first) = heads.first() else { return };
    let base = g.git_string(&["merge-base", "--octopus", "HEAD", first]).unwrap_or_default();
    let range = format!("{}..{first}", base.trim());
    let c = log_lines(g, Some(&range), LOG_SECTION_LIMIT, &["--decorate=full"]);
    let title = if c.is_empty() {
        format!("Merging {}:", heads.join(", "))
    } else {
        format!("Merging {} ({})", heads.join(", "), c.len())
    };
    b.commit_section(g, "unmerged", title, "merge-log", &c);
}

/// magit-sequence-insert-sequence: the stopped commit, done commits and onto.
fn sequence_tail(g: &Git, stop: Option<String>, onto: Option<String>) -> Vec<(String, String)> {
    let head = g.rev_parse("HEAD").unwrap_or_default();
    let onto = onto.and_then(|o| g.rev_parse(&o).ok()).unwrap_or_else(|| head.clone());
    let done: Vec<(String, String, String)> = g
        .git_string(&["log", "--format=%H%x00%h%x00%s", &format!("{onto}..HEAD"), "--"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, '\0');
            Some((p.next()?.to_string(), p.next()?.to_string(), p.next().unwrap_or("").to_string()))
        })
        .collect();
    let mut out = Vec::new();
    let stop = stop.and_then(|s| g.rev_parse(&s).ok());
    if let Some(s) = &stop {
        if !done.iter().any(|d| &d.0 == s) {
            let unmerged = !g.git_string(&["ls-files", "--unmerged"]).unwrap_or_default().trim().is_empty();
            let kind = if unmerged {
                "join"
            } else if g.anything_modified() {
                "work"
            } else {
                "gone"
            };
            out.push(summary_line(g, kind, s));
        }
    }
    for (rev, abbrev, msg) in &done {
        let kind = if Some(rev) == stop.as_ref() {
            if *rev == head { "stop" } else { "like" }
        } else {
            "done"
        };
        out.push((rev.clone(), format!("{kind} {abbrev} {msg}")));
    }
    out.push(summary_line(g, "onto", &onto));
    out
}

/// magit-insert-rebase-sequence
fn insert_rebase_sequence(b: &mut Builder, g: &Git) {
    if !g.rebase_in_progress() {
        return;
    }
    let mergep = g.gitdir.join("rebase-merge").is_dir();
    let dir = if mergep { "rebase-merge" } else { "rebase-apply" };
    let name = file_line(g, &format!("{dir}/head-name")).unwrap_or_else(|| "HEAD".into());
    let name = name.strip_prefix("refs/heads/").unwrap_or(&name).to_string();
    let onto_rev = file_line(g, &format!("{dir}/onto"));
    let onto_name = onto_rev.as_deref().map(|o| shortname(g, o)).unwrap_or_default();
    let mut lines: Vec<(String, String)> = Vec::new();
    if mergep {
        let comment = config(g, "core.commentChar").unwrap_or_else(|| "#".into());
        let todo: Vec<String> = file_lines(g, "rebase-merge/git-rebase-todo")
            .into_iter()
            .filter(|l| !l.trim().is_empty() && !l.starts_with(comment.as_str()))
            .collect();
        for l in todo.iter().rev() {
            let mut p = l.splitn(3, ' ');
            let action = p.next().unwrap_or("");
            let target = p.next().unwrap_or("");
            let rest = p.next().unwrap_or("");
            let is_commit = matches!(
                action,
                "pick" | "p" | "reword" | "r" | "edit" | "e" | "squash" | "s" | "fixup" | "f" | "drop" | "d"
            );
            if is_commit {
                let full = g.rev_parse(target).unwrap_or_else(|_| target.to_string());
                let abbrev = g.rev_short(target);
                let msg = rest.strip_prefix("# ").unwrap_or(rest);
                lines.push((full, format!("{action} {abbrev} {msg}")));
            } else {
                let t = [target, rest].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
                lines.push((String::new(), if t.is_empty() { action.to_string() } else { format!("{action} {t}") }));
            }
        }
        let stop = file_line(g, "rebase-merge/stopped-sha");
        lines.extend(sequence_tail(g, stop, onto_rev));
    } else {
        let rewritten: Vec<String> = file_lines(g, "rebase-apply/rewritten")
            .iter()
            .filter_map(|l| l.split_whitespace().next().map(str::to_string))
            .collect();
        let stop = file_line(g, "rebase-apply/original-commit");
        let mut patches = am_patches(g);
        patches.reverse();
        // (cdr (magit-rebase-patches)) after nreverse: skip the last patch file
        for p in patches.iter().skip(1) {
            let first = std::fs::read_to_string(p).unwrap_or_default();
            let hash = first.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("").to_string();
            if hash.is_empty() || rewritten.contains(&hash) || Some(&hash) == stop.as_ref() {
                continue;
            }
            lines.push(summary_line(g, "pick", &hash));
        }
        lines.extend(sequence_tail(g, stop, onto_rev));
    }
    sequence_section(b, "rebase-sequence", format!("Rebasing {name} onto {onto_name}"), &lines);
}

fn sequence_section(b: &mut Builder, ident: &str, title: String, lines: &[(String, String)]) {
    if b.heading(title, LineKind::SectionHeader { section: ident.into(), n: lines.len() }, ident, 0, !lines.is_empty()) {
        for (i, (hash, text)) in lines.iter().enumerate() {
            let kind = if hash.is_empty() {
                LineKind::Info
            } else {
                LineKind::Commit { section: ident.into(), hash: hash.clone() }
            };
            b.line(text.clone(), kind, &format!("{ident}/{i}"), true, 1);
        }
    }
    b.blank();
}

fn am_patches(g: &Git) -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(g.gitdir.join("rebase-apply"))
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.len() == 4 && n.bytes().all(|b| b.is_ascii_digit()))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// magit-insert-am-sequence
fn insert_am_sequence(b: &mut Builder, g: &Git) {
    if !g.am_in_progress() {
        return;
    }
    let mut patches = am_patches(g);
    patches.reverse();
    let last: usize = file_line(g, "rebase-apply/last").and_then(|s| s.parse().ok()).unwrap_or(0);
    let cur: usize = file_line(g, "rebase-apply/next").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut lines = Vec::new();
    let mut i = last;
    let mut it = patches.iter();
    while i >= cur && i > 0 {
        let Some(p) = it.next() else { break };
        let content = std::fs::read_to_string(p).unwrap_or_default();
        let hash = content.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
        let kind = if i == cur { "stop" } else { "pick" };
        if !hash.is_empty() && rev_exists(g, hash) {
            lines.push(summary_line(g, kind, hash));
        } else {
            let title = content
                .lines()
                .find_map(|l| l.strip_prefix("Subject: "))
                .or_else(|| content.lines().next())
                .unwrap_or("");
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            lines.push((String::new(), format!("{kind} {name} {title}")));
        }
        i -= 1;
    }
    lines.extend(sequence_tail(g, None, Some("ORIG_HEAD".into())));
    sequence_section(b, "rebase-sequence", "Applying patches".into(), &lines);
}

/// magit-insert-sequencer-sequence
fn insert_sequencer_sequence(b: &mut Builder, g: &Git) {
    let picking = g.cherry_in_progress();
    if !picking && !g.revert_in_progress() {
        return;
    }
    let mut lines = Vec::new();
    let todo = file_lines(g, "sequencer/todo");
    for l in todo.iter().skip(1).rev() {
        let mut p = l.splitn(3, ' ');
        let (cmd, hash, msg) = (p.next().unwrap_or(""), p.next().unwrap_or(""), p.next().unwrap_or(""));
        if matches!(cmd, "pick" | "revert") && !hash.is_empty() {
            let full = g.rev_parse(hash).unwrap_or_else(|_| hash.to_string());
            let msg = msg.strip_prefix("# ").unwrap_or(msg);
            lines.push((full, format!("{cmd} {hash} {msg}")));
        }
    }
    let stop = file_line(g, if picking { "CHERRY_PICK_HEAD" } else { "REVERT_HEAD" });
    let onto = file_line(g, "sequencer/head");
    lines.extend(sequence_tail(g, stop, onto));
    let title = if picking { "Cherry Picking" } else { "Reverting" };
    sequence_section(b, "sequence", title.into(), &lines);
}

/// magit-insert-bisect-output / -rest / -log
fn insert_bisect(b: &mut Builder, g: &Git) {
    if !g.bisect_in_progress() {
        return;
    }
    let mut out = file_lines(g, "BISECT_CMD_OUTPUT");
    if out.is_empty() {
        out = vec![
            "Bisecting: (no saved bisect output)".into(),
            "It appears you have invoked \"git bisect\" from a shell.".into(),
            "There is nothing wrong with that, we just cannot display".into(),
            "anything useful here.  Consult the shell output instead.".into(),
        ];
    }
    let bad = out.iter().position(|l| l.ends_with(" is the first bad commit"));
    let heading = match bad {
        Some(i) => out.remove(i),
        None => out.remove(0),
    };
    let kind = match bad {
        Some(_) => LineKind::Commit {
            section: "bisect-output".into(),
            hash: heading.split_whitespace().next().unwrap_or("").to_string(),
        },
        None => LineKind::SectionHeader { section: "bisect-output".into(), n: out.len() },
    };
    if b.heading(heading, kind, "bisect-output", 0, !out.is_empty()) {
        for l in &out {
            b.line(l.clone(), LineKind::Info, "bisect-output", false, 0);
        }
    }
    b.blank();

    // git bisect visualize git log == git log --bisect
    let rest = log_lines(g, None, LOG_SECTION_LIMIT, &["--decorate=full", "--bisect"]);
    b.commit_section(g, "bisect-view", format!("Bisect Rest ({})", rest.len()), "bisect-view", &rest);

    // magit-wash-bisect-log
    let log = g.git_string(&["bisect", "log"]).unwrap_or_default();
    let mut items: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut first_bad = None;
    for l in log.lines() {
        if l.starts_with("# status:") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("# first bad commit: [") {
            first_bad = rest.split(']').next().map(str::to_string);
            continue;
        }
        if l.starts_with("git bisect ") {
            items.push((l.to_string(), std::mem::take(&mut pending)));
            continue;
        }
        if let Some(rest) = l.strip_prefix("# ") {
            // "# bad: [hash] subject"
            if let Some((_, after)) = rest.split_once(": [") {
                if let Some((hash, subj)) = after.split_once("] ") {
                    let abbrev = g.rev_short(hash);
                    pending.push((hash.to_string(), format!("{abbrev} {subj}")));
                }
            }
        }
    }
    let n = items.len() + usize::from(first_bad.is_some());
    if b.heading(format!("Bisect Log ({n})"), LineKind::SectionHeader { section: "bisect-log".into(), n }, "bisect-log", 0, n > 0) {
        for (i, (heading, commits)) in items.iter().enumerate() {
            let id = format!("bisect-log/{i}");
            if b.heading(heading.clone(), LineKind::Info, &id, 1, !commits.is_empty()) {
                for (hash, text) in commits {
                    b.line(text.clone(), LineKind::Commit { section: "bisect-log".into(), hash: hash.clone() }, &format!("{id}/{hash}"), true, 2);
                }
            }
        }
        if let Some(h) = first_bad {
            b.line(format!("{h} is the first bad commit"), LineKind::Info, "bisect-log/first-bad", true, 1);
        }
    }
    b.blank();
}

// ---------------------------------------------------------------- files

/// magit-insert-untracked-files ("Untracked files (N)", hidden by default)
fn insert_untracked(b: &mut Builder, files: &[FileEntry]) {
    if files.is_empty() {
        return;
    }
    let n = files.len();
    let kind = LineKind::SectionHeader { section: "untracked".into(), n };
    if b.heading(format!("Untracked files ({n})"), kind, "untracked", 0, true) {
        for f in files.iter().take(FILE_LIST_LIMIT) {
            let id = file_ident("untracked", &f.path);
            b.line(f.path.clone(), LineKind::File { section: "untracked".into(), path: f.path.clone() }, &id, true, 1);
        }
        if n > FILE_LIST_LIMIT {
            b.line(format!("{} files not listed", n - FILE_LIST_LIMIT), LineKind::Info, "untracked/info", true, 1);
        }
    }
    b.blank();
}

/// Magit's status word for a porcelain status letter.
pub fn status_word(code: char) -> &'static str {
    match code {
        'A' => "new file",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'T' => "new type",
        'U' => "unmerged",
        _ => "modified",
    }
}

/// magit-insert-unstaged-changes / magit-insert-staged-changes
fn insert_changes(b: &mut Builder, section: &str, title: &str, files: &[FileEntry], diffs: &[FileDiff]) {
    if files.is_empty() {
        return;
    }
    let n = files.len();
    let kind = LineKind::SectionHeader { section: section.into(), n };
    if b.heading(format!("{title} ({n})"), kind, section, 0, true) {
        for f in files {
            let d = diffs.iter().find(|d| d.path == f.path);
            let word = if f.is_unmerged() { "unmerged" } else { status_word(f.code) };
            let mut text = match &f.orig_path {
                Some(orig) => format!("{word:<11}{orig} -> {}", f.path),
                None => format!("{word:<11}{}", f.path),
            };
            if f.is_unmerged() {
                text.push_str(f.conflict_desc());
            }
            if d.is_some_and(|d| d.binary) {
                text.push_str(" (binary)");
            }
            let id = file_ident(section, &f.path);
            let hunks = d.map(|d| d.hunks.as_slice()).unwrap_or(&[]);
            let fk = LineKind::File { section: section.into(), path: f.path.clone() };
            if b.heading(text, fk, &id, 1, !hunks.is_empty()) {
                for (hi, h) in hunks.iter().enumerate() {
                    let hid = hunk_ident(section, &f.path, &h.header);
                    let hk = LineKind::Hunk { section: section.into(), path: f.path.clone(), hunk: hi };
                    if b.heading(h.header.clone(), hk, &hid, 2, !h.lines.is_empty()) {
                        for (li, l) in h.lines.iter().enumerate() {
                            let lk = LineKind::DiffLine { section: section.into(), path: f.path.clone(), hunk: hi, line: li };
                            b.line(l.clone(), lk, &hid, false, 2);
                        }
                    }
                }
            }
        }
    }
    b.blank();
}

/// magit-insert-stashes ("Stashes (N)", hidden by default)
fn insert_stashes(b: &mut Builder, g: &Git, stashes: &[(String, String)]) {
    if stashes.is_empty() {
        return;
    }
    // the margins: author and date of each entry of the stash reflog
    let idents: Vec<(String, String)> = if b.log_margin.enable {
        g.git_string(&["reflog", "--format=%aN%x00%at", "refs/stash", "--"])
            .unwrap_or_default()
            .lines()
            .map(|l| {
                let (a, d) = l.split_once('\0').unwrap_or((l, ""));
                (a.to_string(), d.to_string())
            })
            .collect()
    } else {
        Vec::new()
    };
    let now = now();
    let n = stashes.len();
    let kind = LineKind::SectionHeader { section: "stashes".into(), n };
    if b.heading(format!("Stashes ({n})"), kind, "stashes", 0, true) {
        for (i, (name, msg)) in stashes.iter().enumerate() {
            let id = format!("stashes/{name}");
            let k = LineKind::Stash { idx: i, name: name.clone(), msg: msg.clone() };
            b.line(format!("{name} {msg}"), k, &id, true, 1);
            if let Some((author, date)) = idents.get(i) {
                let m = b.log_margin.author(author, date, now);
                b.margin(m);
            }
        }
    }
    b.blank();
}

/// Current mode line shown in the footer.
pub fn modeline(app_name: &str, repo: &str, msg: &str) -> String {
    format!("{app_name} {repo}   {msg}")
}
