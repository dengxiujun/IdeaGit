//! Replaceable Git, storage, credential, journal, watcher, and process contracts.

mod head;
mod process;
mod repository;
mod status;

pub use head::{BranchReader, HeadReader};
pub use process::{
    DEFAULT_OUTPUT_LIMIT, EnvironmentVariable, ExecutionContext, ProcessOutput, ProcessProgress,
    ProcessRunner, ProcessSpec, ProcessStdin, ProcessStream, Sensitivity,
};
pub use repository::{RepositoryLocation, RepositoryReader};
pub use status::{StatusOptions, StatusParseError, StatusParser, StatusReader};
