use async_trait::async_trait;
use git_domain::{GitResult, ObjectId, SessionTarget};
use git_shelf::UnshelveDecision;

use crate::{ExecutionContext, RepositoryLocation};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnshelveRequest {
    pub target: SessionTarget,
    pub base_head: Option<ObjectId>,
    pub current_head: Option<ObjectId>,
    pub worktree_dirty: bool,
    pub patches: Vec<Vec<u8>>,
}

impl UnshelveRequest {
    pub fn validate(&self) -> Result<(), UnshelveError> {
        if self.patches.is_empty() || self.patches.iter().any(Vec::is_empty) {
            return Err(UnshelveError::EmptyPatch);
        }
        if self.worktree_dirty || self.base_head != self.current_head {
            return Err(UnshelveError::RequiresExplicitConflictResolution);
        }
        Ok(())
    }

    #[must_use]
    pub fn decision(&self) -> UnshelveDecision {
        if self.worktree_dirty || self.base_head != self.current_head {
            UnshelveDecision::RequireExplicitConflictResolution
        } else {
            UnshelveDecision::Apply
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UnshelveError {
    #[error("unshelve requires at least one non-empty patch")]
    EmptyPatch,
    #[error("unshelve requires explicit conflict resolution for dirty or stale worktree")]
    RequiresExplicitConflictResolution,
}

#[async_trait]
pub trait ShelfWriter: Send + Sync {
    async fn unshelve(
        &self,
        location: &RepositoryLocation,
        request: UnshelveRequest,
        context: ExecutionContext,
    ) -> GitResult<()>;
}
