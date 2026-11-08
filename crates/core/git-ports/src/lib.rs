//! Replaceable Git, storage, credential, journal, watcher, and process contracts.

mod branch;
mod clone;
mod commit;
mod conflict;
mod credential;
mod head;
mod history;
mod index;
mod journal;
mod merge;
mod process;
mod remote;
mod repository;
mod resolution;
mod status;
mod version;

pub use branch::BranchWriter;
pub use clone::{CloneRequest, CloneWriter};
pub use commit::{CommitOutcome, CommitWriter};
pub use conflict::ConflictStageReader;
pub use credential::{
    CredentialBroker, CredentialError, CredentialKind, CredentialRequest, CredentialResponse,
};
pub use head::{BranchReader, HeadReader};
pub use history::HistoryReader;
pub use index::{IndexMutation, IndexMutationError, IndexWriter};
pub use journal::{JournalError, JournalStore, PendingJournalRecords};
pub use merge::{MergeWriter, SequencerReader, SequencerWriter};
pub use process::{
    DEFAULT_OUTPUT_LIMIT, EnvironmentVariable, ExecutionContext, ProcessOutput, ProcessProgress,
    ProcessRunner, ProcessSpec, ProcessStdin, ProcessStream, Sensitivity,
};
pub use remote::{PushPreviewReader, RemoteReader, RemoteWriter};
pub use repository::{RepositoryLocation, RepositoryReader};
pub use resolution::{
    ApplyResolutionRequest, ConflictResolutionWriter, ConflictStageError, ConflictStages,
    ResolutionRequestError, WorktreeConflictError, validate_conflict_stages,
    validate_worktree_digest, worktree_digest,
};
pub use status::{StatusOptions, StatusParseError, StatusParser, StatusReader};
pub use version::{FileVersions, VersionDescriptor, VersionReader};
