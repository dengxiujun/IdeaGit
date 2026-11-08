use async_trait::async_trait;
use git_domain::{GitResult, ObjectId, SessionTarget};

use crate::ExecutionContext;

/// Repository facts captured around a commit side-effect boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommitSideEffectSnapshot {
    pub head: Option<ObjectId>,
    pub worktree_digest: [u8; 32],
    pub refs_digest: [u8; 32],
}

/// Port used by the coordinator to observe Hook/Ref/Worktree side effects.
#[async_trait]
pub trait CommitSideEffectObserver: Send + Sync {
    async fn capture(
        &self,
        target: SessionTarget,
        context: ExecutionContext,
    ) -> GitResult<CommitSideEffectSnapshot>;

    async fn observe(
        &self,
        target: SessionTarget,
        context: ExecutionContext,
    ) -> GitResult<CommitSideEffectSnapshot>;
}
