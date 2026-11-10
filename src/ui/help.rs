//! Help in transients (transient-help, `?` / `C-h`): the manual of the
//! prefix (its info-manual or man-page), the manpage of an infix scrolled
//! to the description of its argument, the description of a command.

use std::path::PathBuf;

/// The :man-page of the transient NAME (transient-define-prefix).
pub fn man_page(name: &str) -> Option<&'static str> {
    Some(match name {
        "commit-menu" => "git-commit",
        "branch-menu" | "branch-config-menu" | "show-refs-menu" => "git-branch",
        "remote-menu" => "git-remote",
        "fetch-menu" => "git-fetch",
        "pull-menu" => "git-pull",
        "push-menu" => "git-push",
        "merge-menu" => "git-merge",
        "rebase-menu" => "git-rebase",
        "reset-menu" => "git-reset",
        "tag-menu" => "git-tag",
        "stash-menu" => "git-stash",
        "gitignore-menu" => "gitignore",
        "diff-menu" | "diff-refresh-menu" => "git-diff",
        "log-menu" | "log-refresh-menu" => "git-log",
        "shortlog-menu" => "git-shortlog",
        "cherry-pick-menu" => "git-cherry-pick",
        "revert-menu" => "git-revert",
        "bisect-menu" => "git-bisect",
        "worktree-menu" => "git-worktree",
        "submodule-menu" => "git-submodule",
        "notes-menu" => "git-notes",
        "subtree-menu" => "git-subtree",
        "patches-menu" => "git-am",
        "blame-menu" => "git-blame",
        _ => return None,
    })
}

/// The :info-manual of the transient NAME (shown instead of the
/// manpage for the prefix itself).
pub fn info_manual(name: &str) -> Option<&'static str> {
    Some(match name {
        "dispatch" => "(magit)Top",
        "commit-menu" => "(magit)Initiating a Commit",
        "margin-settings-menu" => "(magit) Log Margin",
        "ediff-menu" => "(ediff)",
        "file-dispatch" => "(magit) Minor Mode for Buffers Visiting Files",
        _ => return None,
    })
}

/// The Magit command of the transient NAME.
pub fn prefix_command(name: &str) -> String {
    let base = match name {
        "dispatch" => return "magit-dispatch".into(),
        "file-dispatch" => return "magit-file-dispatch".into(),
        "patches-menu" => return "magit-am".into(),
        "branch-config-menu" => return "magit-branch-configure".into(),
        "margin-settings-menu" => return "magit-margin-settings".into(),
        n => n.trim_end_matches("-menu"),
    };
    format!("magit-{base}")
}

/// The text of the manpage PAGE formatted for WIDTH columns (Man-getpage
/// with `man`, without the overstrikes).
pub fn manpage(page: &str, width: usize) -> Result<String, String> {
    let out = std::process::Command::new("man")
        .arg(page)
        .env("MANPAGER", "cat")
        .env("PAGER", "cat")
        .env("MANWIDTH", width.max(20).to_string())
        .env("GROFF_NO_SGR", "1")
        .output()
        .map_err(|e| format!("Cannot run man: {e}"))?;
    let text = strip_overstrike(&String::from_utf8_lossy(&out.stdout));
    if !out.status.success() || text.trim().is_empty() {
        return Err(format!("No manual entry for {page}"));
    }
    Ok(text)
}

/// The directories searched for info manuals and the Lisp sources of
/// Magit and transient: $INFOPATH, the Magit and transient packages of
/// ~/.emacs.d/elpa, and the usual info directories.
pub fn default_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("INFOPATH") {
        dirs.extend(std::env::split_paths(&p).filter(|d| !d.as_os_str().is_empty()));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let elpa = PathBuf::from(home).join(".emacs.d").join("elpa");
        if let Ok(rd) = std::fs::read_dir(&elpa) {
            let mut pkgs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    n.starts_with("magit-") || n.starts_with("transient-")
                })
                .collect();
            pkgs.sort();
            pkgs.reverse();
            dirs.extend(pkgs);
        }
    }
    dirs.extend(["/opt/homebrew/share/info", "/usr/local/share/info", "/usr/share/info"].map(PathBuf::from));
    dirs
}

/// The node of the info manual SPEC (`(manual)node`, the Top node if
/// none) found in DIRS: (title, text).
pub fn info_node(dirs: &[PathBuf], spec: &str) -> Result<(String, String), String> {
    let (manual, node) = match spec.strip_prefix('(').and_then(|s| s.split_once(')')) {
        Some((m, n)) => (m, n.trim()),
        None => return Err(format!("Invalid info node: {spec}")),
    };
    let node = if node.is_empty() { "Top" } else { node };
    let file = format!("{manual}.info");
    let Some(dir) = dirs.iter().find(|d| d.join(&file).is_file()) else {
        return Err(format!("Info file {manual} does not exist"));
    };
    // the file and the parts of a split manual (FILE-1, FILE-2, ...)
    let mut parts = vec![dir.join(&file)];
    parts.extend((1..).map(|i| dir.join(format!("{file}-{i}"))).take_while(|p| p.is_file()));
    for part in parts {
        let text = std::fs::read_to_string(&part).map_err(|e| e.to_string())?;
        for chunk in text.split('\u{1f}') {
            let chunk = chunk.trim_start_matches('\n');
            let Some((header, body)) = chunk.split_once('\n') else { continue };
            let name = header
                .split(",  ")
                .find_map(|f| f.trim().strip_prefix("Node: "))
                .map(|n| n.trim_end_matches(',').trim());
            if header.starts_with("File: ") && name == Some(node) {
                return Ok((format!("*info* ({manual}) {node}"), body.trim_start_matches('\n').to_string()));
            }
        }
    }
    Err(format!("No such node or anchor: {node}"))
}

/// The definition forms whose name is the command.
const DEFINERS: [&str; 5] =
    ["defun", "transient-define-prefix", "transient-define-suffix", "transient-define-infix", "transient-define-argument"];

/// describe-function for the command NAME: where it is defined in the
/// Lisp files of DIRS, its argument list and documentation. DESC
/// describes the commands that aren't found.
pub fn describe_function(dirs: &[PathBuf], name: &str, desc: &str) -> String {
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        let mut files: Vec<PathBuf> =
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "el")).collect();
        files.sort();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let Some(rest) = DEFINERS.iter().find_map(|d| {
                let form = format!("({d} {name}");
                text.match_indices(&form)
                    .map(|(i, _)| &text[i + form.len()..])
                    .find(|r| r.starts_with(char::is_whitespace))
            }) else {
                continue;
            };
            let fname = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let (args, doc) = definition(rest);
            let args: Vec<String> = args
                .split_whitespace()
                .map(|a| if a.starts_with('&') { a.to_string() } else { a.to_uppercase() })
                .collect();
            let call = std::iter::once(name.to_string()).chain(args).collect::<Vec<_>>().join(" ");
            let doc = doc.unwrap_or_else(|| "Not documented.".into());
            return format!("{name} is an interactive Lisp function in `{fname}'.\n\n({call})\n\n{doc}\n");
        }
    }
    format!("{name} is an interactive command.\n\n{desc}.\n")
}

/// The argument list (without its parentheses) and the documentation
/// string after the name of a definition.
fn definition(rest: &str) -> (String, Option<String>) {
    let rest = rest.trim_start();
    let Some(after) = rest.strip_prefix('(') else { return (String::new(), None) };
    let mut depth = 1;
    let Some(end) = after.char_indices().find_map(|(i, c)| {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        (depth == 0).then_some(i)
    }) else {
        return (String::new(), None);
    };
    let args = after[..end].to_string();
    let Some(body) = after[end + 1..].trim_start().strip_prefix('"') else { return (args, None) };
    let mut doc = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('\n') | None => {}
                Some('n') => doc.push('\n'),
                Some('t') => doc.push('\t'),
                Some(c) => doc.push(c),
            },
            c => doc.push(c),
        }
    }
    // the calling convention given by `\(fn ARGS)' isn't documentation
    if let Some(i) = doc.find("\n(fn") {
        doc.truncate(i);
    }
    (args, Some(doc.trim_end().to_string()))
}

/// Remove the bold and underline overstrikes (`X\bX`, `_\bX`) of man's
/// output (what `col -b` does).
fn strip_overstrike(s: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(s.len());
    for c in s.chars() {
        if c == '\u{8}' {
            out.pop();
        } else {
            out.push(c);
        }
    }
    out.into_iter().collect()
}

/// Whether ITEM is an option as matched by transient's
/// `-\(?:. \|-[^[:space:]]+ \)?[^[:space:]]+`.
fn is_option(item: &str) -> bool {
    let Some(rest) = item.strip_prefix('-') else { return false };
    match rest.split_once(' ') {
        None => !rest.is_empty(),
        Some((a, b)) => {
            let short = a.chars().count() == 1;
            let long = a.starts_with('-') && a.len() > 1;
            (short || long) && !b.is_empty() && !b.contains(char::is_whitespace)
        }
    }
}

/// Whether S is `[^[:space:]]*[^.[:space:]]`.
fn value(s: &str) -> bool {
    !s.is_empty() && !s.contains(char::is_whitespace) && !s.ends_with('.')
}

/// Whether ITEM is the option ARG (transient--goto-argument-description).
fn is_argument(item: &str, arg: &str) -> bool {
    let name = arg.strip_suffix('=').unwrap_or(arg);
    let Some(rest) = item.strip_prefix(name) else { return false };
    if arg.ends_with('=') {
        // `\(?: \|\[?=\)VALUE`
        [" ", "[=", "="].iter().any(|p| rest.strip_prefix(p).is_some_and(value))
    } else {
        // `\(?:\(?: \| ?[\[<:]\)VALUE\)?`
        let (space, r) = match rest.strip_prefix(' ') {
            Some(r) => (true, r),
            None => (false, rest),
        };
        rest.is_empty() || (space && value(r)) || r.strip_prefix(['[', '<', ':']).is_some_and(value)
    }
}

/// The line of the manpage TEXT describing the argument ARG: a line
/// starting with whitespace, of options separated by ", ", one of them
/// ARG (with its value).
pub fn argument_line(text: &str, arg: &str) -> Option<usize> {
    text.lines().position(|line| {
        if !line.starts_with([' ', '\t']) {
            return false;
        }
        let t = line.trim_start();
        if arg == "--" {
            return t == "--" || t.starts_with("-- ") || t.starts_with("[--]");
        }
        let items: Vec<&str> = t.split(", ").collect();
        items.iter().any(|i| is_argument(i, arg)) && items.iter().all(|i| is_argument(i, arg) || is_option(i))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAN: &str = "NAME\n       git-commit - Record changes\n\n       --author=<author> is used\n       \
                       to override.\n       -a, --all\n           Tell the command.\n       --author=<author>\n           \
                       Override.\n       -v, --verbose\n       -S[<key-id>], --gpg-sign[=<key-id>], --no-gpg-sign\n       \
                       -m <msg>, --message=<msg>\n       -m parent-number, --mainline parent-number\n       [--] <path>...\n";

    #[test]
    fn arguments() {
        let at = |a: &str| argument_line(MAN, a);
        assert_eq!(at("--author="), Some(7), "not the sentence starting with it");
        assert_eq!(at("--all"), Some(5));
        assert_eq!(at("-a"), Some(5));
        assert_eq!(at("--verbose"), Some(9));
        assert_eq!(at("-S"), Some(10));
        assert_eq!(at("--gpg-sign="), Some(10));
        assert_eq!(at("--no-gpg-sign"), Some(10));
        assert_eq!(at("--message="), Some(11));
        assert_eq!(at("-m"), Some(11));
        assert_eq!(at("--mainline="), Some(12));
        assert_eq!(at("--"), Some(13));
        assert_eq!(at("--amend"), None);
        assert_eq!(at("--auth"), None);
    }

    #[test]
    fn overstrikes() {
        assert_eq!(strip_overstrike("N\u{8}NA\u{8}AM\u{8}ME\u{8}E _\u{8}x"), "NAME x");
        assert_eq!(prefix_command("commit-menu"), "magit-commit");
        assert_eq!(prefix_command("patches-menu"), "magit-am");
    }

    #[test]
    fn definitions() {
        let (args, doc) = definition(" (&optional args)\n  \"Create a new commit.\n\\\\[foo] \\\"x\\\".\\n\\n(fn ARGS)\"\n  (interactive)");
        assert_eq!(args, "&optional args");
        assert_eq!(doc.as_deref(), Some("Create a new commit.\n\\[foo] \"x\"."));
        assert_eq!(definition(" ()\n  (interactive))"), (String::new(), None));
    }

    #[test]
    fn info_nodes() {
        let dir = std::env::temp_dir().join(format!("xagit-help-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("m.info"),
            "This is m.info.\n\u{1f}\nFile: m.info,  Node: Top,  Next: A B,  Up: (dir)\n\nTop text\n\
             \u{1f}\nFile: m.info,  Node: A B,  Prev: Top,  Up: Top\n\n1 A B\n=====\nText\n\u{1f}\nTag Table:\n",
        )
        .unwrap();
        let dirs = vec![dir.clone()];
        assert_eq!(info_node(&dirs, "(m)Top"), Ok(("*info* (m) Top".into(), "Top text\n".into())));
        assert_eq!(info_node(&dirs, "(m)"), Ok(("*info* (m) Top".into(), "Top text\n".into())));
        assert_eq!(info_node(&dirs, "(m) A B").map(|n| n.1), Ok("1 A B\n=====\nText\n".into()));
        assert_eq!(info_node(&dirs, "(m)C"), Err("No such node or anchor: C".into()));
        assert_eq!(info_node(&dirs, "(x)"), Err("Info file x does not exist".into()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
