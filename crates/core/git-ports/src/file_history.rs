use crate::ExecutionContext;
use async_trait::async_trait;
use git_domain::{
    BlameLine, FileHistoryEntry, FileHistoryQuery, GitResult, RepoPath, SessionTarget,
};

#[async_trait]
pub trait FileHistoryReader: Send + Sync {
    async fn history(
        &self,
        query: FileHistoryQuery,
        context: ExecutionContext,
    ) -> GitResult<Vec<FileHistoryEntry>>;
}
#[async_trait]
pub trait BlameReader: Send + Sync {
    async fn blame(
        &self,
        target: SessionTarget,
        path: RepoPath,
        context: ExecutionContext,
    ) -> GitResult<Vec<BlameLine>>;
}
