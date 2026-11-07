//! Pure Git domain values, identities, snapshots, commands, and errors; no IO or UI.
//!
//! This crate owns shared value semantics. It deliberately does not convert between Git's raw
//! repository paths and operating-system paths; adapters must perform that platform-specific work.

mod command;
mod error;
mod head;
mod id;
mod object_id;
mod path;
mod snapshot;
mod status;

pub use command::{
    BranchCommand, ChangeSelection, CommandValidationError, CommitOptions, EditedIndexSessionId,
    FileIdentity, FileKind, GitCommand, LineSpan, RemoteCommand, SelectionCommitRequest,
    SelectionFingerprint, SequencerCommand, SigningMode, StandardCommitRequest,
};
pub use error::{GitError, GitErrorKind, GitResult, RecoveryAction};
pub use head::{BranchTracking, HeadState};
pub use id::{OperationId, RepositoryId, SessionTarget, SnapshotRevision, WorktreeId};
pub use object_id::{ObjectFormat, ObjectId, ObjectIdError};
pub use path::{LocalPath, RepoPath, RepoPathError};
pub use snapshot::{
    ConflictMetadata, FileChange, RepositorySnapshot, SnapshotBuildError, SnapshotBuilder,
    SnapshotConsistency, SnapshotRead, SnapshotStaleReason, WorktreeSnapshotState,
};
pub use status::{
    BranchHead, BranchOid, BranchStatus, ChangeKind, ConflictKind, FileMode, OrdinaryChange,
    RefName, RefNameError, RenamedChange, Similarity, SimilarityKind, StatusData, StatusEntry,
    SubmoduleState, UnmergedChange,
};
