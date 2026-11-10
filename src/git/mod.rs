pub mod apply;
pub mod diff;
pub mod exec;
pub mod libgit2;
pub mod ops;
pub mod query;
pub mod status;
pub mod wip;

pub use exec::Git;
#[allow(unused_imports)]
pub use libgit2::{FileStatus, LibGit2};

#[allow(unused_imports)]
pub use status::{FileEntry, Status, StatusKind};
