//! Pure Git domain values, identities, snapshots, commands, and errors; no IO or UI.
//!
//! This crate owns shared value semantics. It deliberately does not convert between Git's raw
//! repository paths and operating-system paths; adapters must perform that platform-specific work.

mod error;
mod id;
mod object_id;
mod path;

pub use error::{GitError, GitErrorKind, GitResult, RecoveryAction};
pub use id::{OperationId, RepositoryId, SessionTarget, SnapshotRevision, WorktreeId};
pub use object_id::{ObjectFormat, ObjectId, ObjectIdError};
pub use path::{LocalPath, RepoPath, RepoPathError};
