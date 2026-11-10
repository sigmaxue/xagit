//! move-file-to-trash: the system's trash can (macOS: `trash`, else the
//! freedesktop.org trash), or `trash-directory` when it is set.

use std::path::{Path, PathBuf};

/// Move the file or directory PATH to the trash: into TRASH_DIRECTORY
/// if given, else the system's trash.
pub fn move_to_trash(path: &Path, trash_directory: Option<&Path>) -> Result<(), String> {
    if std::fs::symlink_metadata(path).is_err() {
        return Err(format!("No such file or directory, {}", path.display()));
    }
    match trash_directory {
        Some(dir) => to_directory(path, dir),
        None if cfg!(target_os = "macos") => macos(path),
        None => freedesktop(path),
    }
}

/// The trash-directory case: move PATH into DIR, a numbered backup
/// name (`NAME.~N~`) if NAME exists there.
fn to_directory(path: &Path, dir: &Path) -> Result<(), String> {
    let abs = absolute(path);
    let dir = absolute(dir);
    if dir.starts_with(&abs) {
        return Err(format!("Trash directory `{}' is a subdirectory of `{}'", dir.display(), path.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut target = dir.join(&name);
    let mut n = 1;
    while std::fs::symlink_metadata(&target).is_ok() {
        target = dir.join(format!("{name}.~{n}~"));
        n += 1;
    }
    rename(&abs, &target)
}

/// system-move-file-to-trash on macOS: `trash` (macOS 15), else the Finder.
fn macos(path: &Path) -> Result<(), String> {
    let abs = absolute(path);
    let out = if Path::new("/usr/bin/trash").exists() {
        std::process::Command::new("/usr/bin/trash").arg(&abs).output()
    } else {
        let file = abs.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        std::process::Command::new("osascript")
            .args(["-e", &format!("tell application \"Finder\" to delete POSIX file \"{file}\"")])
            .output()
    }
    .map_err(|e| format!("Cannot move {} to trash: {e}", path.display()))?;
    if out.status.success() && std::fs::symlink_metadata(&abs).is_err() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!("Cannot move {} to trash: {err}", path.display()))
    }
}

/// The freedesktop.org trash: `$XDG_DATA_HOME/Trash` (or
/// ~/.local/share/Trash) with a .trashinfo file.
fn freedesktop(path: &Path) -> Result<(), String> {
    let data = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME").ok_or("HOME isn't set")?).join(".local/share"),
    };
    freedesktop_in(path, &data.join("Trash"), &deletion_date())
}

fn freedesktop_in(path: &Path, trash: &Path, date: &str) -> Result<(), String> {
    let abs = absolute(path);
    let (files, info) = (trash.join("files"), trash.join("info"));
    for d in [&files, &info] {
        if absolute(d).starts_with(&abs) {
            return Err(format!("The trash directory {} is a subdirectory of {}", d.display(), path.display()));
        }
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let name = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut base = name.clone();
    let mut n = 1;
    let info_file = loop {
        let f = info.join(format!("{base}.trashinfo"));
        let taken = std::fs::symlink_metadata(files.join(&base)).is_ok();
        if !taken {
            if let Ok(mut file) = std::fs::OpenOptions::new().write(true).create_new(true).open(&f) {
                use std::io::Write;
                let text = format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", hexify(&abs.to_string_lossy()));
                file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
                break f;
            }
        }
        base = format!("{name}.{n}");
        n += 1;
    };
    rename(&abs, &files.join(&base)).inspect_err(|_| {
        let _ = std::fs::remove_file(&info_file);
    })
}

/// trash--hexify-table: percent-encode all but the unreserved
/// characters and `/`.
fn hexify(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"/-_.!~*'()".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02x}")
            }
        })
        .collect()
}

/// The local time as `%Y-%m-%dT%T`.
fn deletion_date() -> String {
    std::process::Command::new("date")
        .arg("+%Y-%m-%dT%H:%M:%S")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn absolute(p: &Path) -> PathBuf {
    let p = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    // without a trailing slash (directory-file-name)
    PathBuf::from(p.to_string_lossy().trim_end_matches('/'))
}

/// rename-file, copying across file systems.
fn rename(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let out = std::process::Command::new("mv").arg(from).arg(to).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xagit-trash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn trash_directory() {
        let d = scratch("dir");
        let trash = d.join("trash");
        for _ in 0..3 {
            std::fs::write(d.join("a.txt"), "a").unwrap();
            move_to_trash(&d.join("a.txt"), Some(&trash)).unwrap();
            assert!(!d.join("a.txt").exists());
        }
        let mut names: Vec<String> =
            std::fs::read_dir(&trash).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["a.txt", "a.txt.~1~", "a.txt.~2~"]);
        std::fs::create_dir_all(d.join("sub/x")).unwrap();
        move_to_trash(&d.join("sub/"), Some(&trash)).unwrap();
        assert!(trash.join("sub/x").is_dir());
        assert!(move_to_trash(&d, Some(&trash)).unwrap_err().contains("is a subdirectory of"));
        assert!(move_to_trash(&d.join("none"), Some(&trash)).unwrap_err().starts_with("No such file"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn freedesktop_trash() {
        let d = scratch("xdg");
        let trash = d.join("Trash");
        for _ in 0..2 {
            std::fs::write(d.join("a b.txt"), "a").unwrap();
            freedesktop_in(&d.join("a b.txt"), &trash, "2026-11-09T10:00:00").unwrap();
        }
        assert!(trash.join("files/a b.txt").is_file() && trash.join("files/a b.txt.1").is_file());
        let info = std::fs::read_to_string(trash.join("info/a b.txt.1.trashinfo")).unwrap();
        let path = hexify(&d.join("a b.txt").to_string_lossy());
        assert!(path.ends_with("/a%20b.txt"), "{path}");
        assert_eq!(info, format!("[Trash Info]\nPath={path}\nDeletionDate=2026-11-09T10:00:00\n"));
        std::fs::remove_dir_all(d).unwrap();
    }
}
