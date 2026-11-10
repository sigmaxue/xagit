//! xagit — a Magit-style git TUI in Rust.
//!
//! Library root so integration tests (tests/) can exercise the git command
//! layer and the transient menu definitions directly.

pub mod git;
pub mod ui;

/// Convenience re-exports for tests.
pub mod prelude {
    pub use crate::git::exec::{Editor, Git};
    pub use crate::git::libgit2::{FileStatus, LibGit2};
    pub use crate::git::ops::{SnapshotScope, Untracked};
    pub use crate::git::status::{FileEntry, Status, StatusKind};
    pub use tempfile::TempDir;
}
