//! Replaceable Git, storage, credential, journal, watcher, and process contracts.

mod process;
mod repository;

pub use process::{
    DEFAULT_OUTPUT_LIMIT, EnvironmentVariable, ExecutionContext, ProcessOutput, ProcessProgress,
    ProcessRunner, ProcessSpec, ProcessStdin, ProcessStream, Sensitivity,
};
pub use repository::{RepositoryLocation, RepositoryReader};
