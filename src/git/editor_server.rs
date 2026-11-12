//! Let git use xagit's built-in text buffer as its editor (like Emacs'
//! with-editor): GIT_EDITOR is a small shell script that hands the file
//! git wants edited over to xagit and waits for an answer.  The git
//! command itself runs on a worker thread (its output captured, never on
//! the TUI's terminal); the UI shows the file in a buffer and answers
//! "finish" (the edited text has been written back) or "cancel" (the
//! script exits non-zero, so git aborts that step).
//!
//! Protocol, all inside one private directory per session:
//!   script writes   `req` = "<n>\t<path>"      (a new request)
//!   xagit writes    `ans-<n>` = "ok"|"cancel"  (after editing PATH)
//! The script then exits 0 (ok) or 1 (cancel).  Only one request is
//! outstanding at a time: the script waits for its answer before git can
//! start another editor.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// A file git asked xagit to edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRequest {
    pub seq: u64,
    pub path: PathBuf,
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The shared directory and the script git runs as its editor.
#[derive(Debug)]
pub struct EditorServer {
    dir: PathBuf,
}

impl EditorServer {
    pub fn new() -> std::io::Result<EditorServer> {
        let base = std::env::temp_dir().join(format!(
            "xagit-editor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&base)?;
        let script = base.join("editor.sh");
        let body = format!(
            r#"#!/bin/sh
d={dir}
n=$(($(cat "$d/seq" 2>/dev/null || echo 0) + 1))
echo "$n" > "$d/seq"
f="$1"
case "$f" in /*) ;; *) f="$PWD/$f" ;; esac
printf '%s\t%s\n' "$n" "$f" > "$d/req.tmp" && mv "$d/req.tmp" "$d/req"
while [ ! -f "$d/ans-$n" ]; do
  [ -f "$d/stop" ] && exit 1
  sleep 0.02
done
a=$(cat "$d/ans-$n")
rm -f "$d/ans-$n"
[ "$a" = ok ]
"#,
            dir = sh_quote(&base.to_string_lossy())
        );
        std::fs::write(&script, body)?;
        Ok(EditorServer { dir: base })
    }

    /// The value for GIT_EDITOR / GIT_SEQUENCE_EDITOR.
    pub fn command(&self) -> String {
        format!("sh {}", sh_quote(&self.dir.join("editor.sh").to_string_lossy()))
    }

    /// The pending request, if git is waiting for one (taken once).
    pub fn take_request(&self) -> Option<EditRequest> {
        let req = self.dir.join("req");
        let text = std::fs::read_to_string(&req).ok()?;
        let _ = std::fs::remove_file(&req);
        let (n, path) = text.trim_end_matches('\n').split_once('\t')?;
        Some(EditRequest { seq: n.parse().ok()?, path: PathBuf::from(path) })
    }

    /// Answer request SEQ: finished (ok) or cancelled.
    pub fn answer(&self, seq: u64, ok: bool) {
        let tmp = self.dir.join(format!("ans-{seq}.tmp"));
        if std::fs::write(&tmp, if ok { "ok" } else { "cancel" }).is_ok() {
            let _ = std::fs::rename(&tmp, self.dir.join(format!("ans-{seq}")));
        }
    }

    /// No more answers will come: a waiting script exits non-zero.
    pub fn stop(&self) {
        let _ = std::fs::write(self.dir.join("stop"), "");
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for EditorServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Wait (up to TIMEOUT) for a request, polling the small file handshake.
pub fn wait_request(server: &EditorServer, timeout: Duration) -> Option<EditRequest> {
    let until = std::time::Instant::now() + timeout;
    loop {
        if let Some(r) = server.take_request() {
            return Some(r);
        }
        if std::time::Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hands_over_edits_and_cancels() {
        let s = EditorServer::new().unwrap();
        let file = s.dir().join("msg");
        std::fs::write(&file, "old\n").unwrap();
        let cmd = s.command();
        let f = file.clone();
        let h = std::thread::spawn(move || {
            std::process::Command::new("sh").args(["-c", &format!("{cmd} \"$1\""), "x"]).arg(&f).status().unwrap()
        });
        let r = wait_request(&s, Duration::from_secs(5)).expect("request");
        assert_eq!(r.path, file);
        assert_eq!(r.seq, 1);
        std::fs::write(&r.path, "new\n").unwrap();
        s.answer(r.seq, true);
        assert!(h.join().unwrap().success());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");

        let cmd = s.command();
        let h = std::thread::spawn(move || {
            std::process::Command::new("sh").args(["-c", &format!("{cmd} \"$1\""), "x"]).arg(&file).status().unwrap()
        });
        let r = wait_request(&s, Duration::from_secs(5)).expect("request 2");
        assert_eq!(r.seq, 2);
        s.answer(r.seq, false);
        assert!(!h.join().unwrap().success());
    }

    #[test]
    fn stop_releases_a_waiting_script() {
        let s = EditorServer::new().unwrap();
        let file = s.dir().join("msg");
        std::fs::write(&file, "x\n").unwrap();
        let cmd = s.command();
        let h = std::thread::spawn(move || {
            std::process::Command::new("sh").args(["-c", &format!("{cmd} \"$1\""), "x"]).arg(&file).status().unwrap()
        });
        assert!(wait_request(&s, Duration::from_secs(5)).is_some());
        s.stop();
        assert!(!h.join().unwrap().success());
    }
}
