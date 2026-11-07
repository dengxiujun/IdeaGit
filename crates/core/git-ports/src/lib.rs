//! Replaceable Git, storage, credential, journal, watcher, and process contracts.

mod branch;
mod commit;
mod head;
mod index;
mod journal;
mod process;
mod repository;
mod status;

pub use branch::BranchWriter;
pub use commit::{CommitOutcome, CommitWriter};
pub use head::{BranchReader, HeadReader};
pub use index::{IndexMutation, IndexMutationError, IndexWriter};
pub use journal::{JournalError, JournalStore};
pub use process::{
    DEFAULT_OUTPUT_LIMIT, EnvironmentVariable, ExecutionContext, ProcessOutput, ProcessProgress,
    ProcessRunner, ProcessSpec, ProcessStdin, ProcessStream, Sensitivity,
};
pub use repository::{RepositoryLocation, RepositoryReader};
pub use status::{StatusOptions, StatusParseError, StatusParser, StatusReader};
