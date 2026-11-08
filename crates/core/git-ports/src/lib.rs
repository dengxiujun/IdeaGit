//! Replaceable Git, storage, credential, journal, watcher, and process contracts.

mod branch;
mod candidate_tree;
mod clone;
mod commit;
mod conflict;
mod credential;
mod diff;
mod editable_index;
mod head;
mod history;
mod index;
mod index_reconciliation;
mod journal;
mod merge;
mod process;
mod rebase;
mod remote;
mod repository;
mod resolution;
mod side_effect;
mod status;
mod temporary_index;
mod version;

pub use branch::BranchWriter;
pub use candidate_tree::{CandidateTreeBuilder, CandidateTreeRequest};
pub use clone::{CloneRequest, CloneWriter};
pub use commit::{CommitOutcome, CommitWriter, PartialCommitWriter};
pub use conflict::ConflictStageReader;
pub use credential::{
    CredentialBroker, CredentialError, CredentialKind, CredentialRequest, CredentialResponse,
};
pub use diff::{DiffDocument, DiffReadRequest, DiffReader};
pub use editable_index::{
    EditableIndexCandidate, EditableIndexCandidateBuilder, EditableIndexCandidateRequest,
};
pub use head::{BranchReader, HeadReader};
pub use history::HistoryReader;
pub use index::{
    HunkStageStateReader, HunkStageWriter, IndexMutation, IndexMutationError, IndexWriter,
    StageHunksRequest, StageHunksState,
};
pub use index_reconciliation::{
    IndexReconciliationOutcome, IndexReconciliationPublisher, IndexReconciliationRequest,
};
pub use journal::{JournalError, JournalStore, PendingJournalRecords};
pub use merge::{MergeWriter, SequencerReader, SequencerWriter};
pub use process::{
    DEFAULT_OUTPUT_LIMIT, EnvironmentVariable, ExecutionContext, ProcessOutput, ProcessProgress,
    ProcessRunner, ProcessSpec, ProcessStdin, ProcessStream, Sensitivity,
};
pub use rebase::InteractiveRebasePlanReader;
pub use remote::{PushPreviewReader, RemoteReader, RemoteWriter};
pub use repository::{RepositoryLocation, RepositoryReader};
pub use resolution::{
    ApplyResolutionRequest, ConflictResolutionWriter, ConflictStageError, ConflictStages,
    ResolutionPreflightError, ResolutionRequestError, WorktreeConflictError,
    validate_conflict_stages, validate_resolution_request, validate_worktree_digest,
    worktree_digest,
};
pub use side_effect::{CommitSideEffectObserver, CommitSideEffectSnapshot};
pub use status::{StatusOptions, StatusParseError, StatusParser, StatusReader};
pub use temporary_index::{TemporaryIndexContext, TemporaryIndexLease, TemporaryIndexManager};
pub use version::{FileVersions, VersionDescriptor, VersionReader};
