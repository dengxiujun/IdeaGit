//! Commit history read port.

use async_trait::async_trait;
use git_domain::{CommitPage, GitResult, LogQuery, SessionTarget};

use crate::ExecutionContext;

#[async_trait]
pub trait HistoryReader: Send + Sync {
    async fn log(
        &self,
        target: SessionTarget,
        query: LogQuery,
        context: ExecutionContext,
    ) -> GitResult<CommitPage>;
}
