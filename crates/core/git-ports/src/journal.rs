//! Durable Journal storage port.

use async_trait::async_trait;
use git_domain::{GitResult, JournalRecord, OperationId, RecoveryScope};
use thiserror::Error;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingJournalRecords {
    pub scoped: Vec<JournalRecord>,
    pub orphaned: Vec<JournalRecord>,
}

/// Storage operations required by Operation Journal and startup recovery.
#[async_trait]
pub trait JournalStore: Send + Sync {
    async fn append(&self, record: JournalRecord) -> GitResult<()>;
    async fn replace(&self, record: JournalRecord) -> GitResult<()>;
    async fn get(&self, operation_id: OperationId) -> GitResult<Option<JournalRecord>>;
    async fn pending(&self, scope: &RecoveryScope) -> GitResult<PendingJournalRecords>;
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
