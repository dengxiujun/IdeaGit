//! Replaceable Git, storage, credential, journal, watcher, and process contracts.

mod branch;
mod commit;
mod credential;
mod head;
mod history;
mod index;
mod journal;
mod merge;
mod process;
mod remote;
mod repository;
mod status;

pub use branch::BranchWriter;
pub use commit::{CommitOutcome, CommitWriter};
pub use credential::{
    CredentialBroker, CredentialError, CredentialKind, CredentialRequest, CredentialResponse,
};
pub use head::{BranchReader, HeadReader};
pub use history::HistoryReader;
pub use index::{IndexMutation, IndexMutationError, IndexWriter};
pub use journal::{JournalError, JournalStore, PendingJournalRecords};
pub use merge::MergeWriter;
pub use process::{
    DEFAULT_OUTPUT_LIMIT, EnvironmentVariable, ExecutionContext, ProcessOutput, ProcessProgress,
    ProcessRunner, ProcessSpec, ProcessStdin, ProcessStream, Sensitivity,
};
pub use remote::{PushPreviewReader, RemoteReader, RemoteWriter};
pub use repository::{RepositoryLocation, RepositoryReader};
pub use status::{StatusOptions, StatusParseError, StatusParser, StatusReader};
