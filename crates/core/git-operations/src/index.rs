//! Operation-level serialization for Index mutations.

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult};
use git_ports::{ExecutionContext, IndexMutation, IndexWriter};

use crate::{ResourceKey, ResourceKind, ResourceLockManager, ResourceScope};

/// Index writer decorator that acquires the concrete Worktree Index lock first.
#[derive(Debug)]
pub struct CoordinatedIndexWriter<W> {
    inner: W,
    locks: ResourceLockManager,
}

impl<W> CoordinatedIndexWriter<W> {
    #[must_use]
    pub fn new(inner: W, locks: ResourceLockManager) -> Self {
        Self { inner, locks }
    }
}

#[async_trait]
impl<W: IndexWriter> IndexWriter for CoordinatedIndexWriter<W> {
    #[allow(clippy::result_large_err)]
    async fn apply(&self, mutation: IndexMutation, context: ExecutionContext) -> GitResult<()> {
        let worktree = mutation.target().worktree_id().ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "Index mutation is unavailable for a bare repository",
            )
        })?;
        let _lock = self
            .locks
            .acquire_with_cancellation(
                [ResourceKey {
                    scope: ResourceScope::Worktree(worktree),
                    kind: ResourceKind::Index,
                }],
                context.cancellation.clone(),
            )
            .await
            .map_err(|error| GitError::new(GitErrorKind::Cancelled, error.to_string()))?;
        self.inner.apply(mutation, context).await
    }
}
