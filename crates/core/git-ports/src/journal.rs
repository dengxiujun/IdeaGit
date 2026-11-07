//! Durable Journal storage port.

use async_trait::async_trait;
use git_domain::{GitResult, JournalRecord, OperationId, SessionTarget};
use thiserror::Error;

/// Storage operations required by Operation Journal and startup recovery.
#[async_trait]
pub trait JournalStore: Send + Sync {
    async fn append(&self, record: JournalRecord) -> GitResult<()>;
    async fn replace(&self, record: JournalRecord) -> GitResult<()>;
    async fn get(&self, operation_id: OperationId) -> GitResult<Option<JournalRecord>>;
    async fn pending(&self, target: SessionTarget) -> GitResult<Vec<JournalRecord>>;
}

/// Port-level Journal failures that adapters may map into [`git_domain::GitError`].
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum JournalError {
    #[error("journal record is already present")]
    AlreadyExists,
    #[error("journal record was not found")]
    NotFound,
    #[error("journal schema is unsupported")]
    UnsupportedSchema,
}
