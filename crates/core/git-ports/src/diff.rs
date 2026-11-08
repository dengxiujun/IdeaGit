use async_trait::async_trait;
use git_diff::{ContentSource, Patch};
use git_domain::{GitResult, RepoPath, SessionTarget, SnapshotRevision};

use crate::{ExecutionContext, RepositoryLocation};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffReadRequest {
    pub target: SessionTarget,
    pub before: ContentSource,
    pub after: ContentSource,
    pub paths: Vec<RepoPath>,
    pub context_lines: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffDocument {
    pub generation: SnapshotRevision,
    pub patch: Patch,
}

/// Read-only diff source. Implementations must not mutate the worktree or Index.
#[async_trait]
pub trait DiffReader: Send + Sync {
    async fn read(
        &self,
        location: &RepositoryLocation,
        request: DiffReadRequest,
        context: ExecutionContext,
    ) -> GitResult<DiffDocument>;
}
