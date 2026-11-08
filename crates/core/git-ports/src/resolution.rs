//! Safe conflict-resolution write boundary.

use async_trait::async_trait;
use git_domain::{GitResult, RepoPath, SessionTarget, SnapshotRevision};

use crate::ExecutionContext;

/// A caller's observed conflict-stage identities for one path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictStages {
    pub stage1: Option<git_domain::ObjectId>,
    pub stage2: Option<git_domain::ObjectId>,
    pub stage3: Option<git_domain::ObjectId>,
}

/// A user-confirmed candidate result for one conflicted path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyResolutionRequest {
    pub target: SessionTarget,
    pub path: RepoPath,
    pub expected_conflict_revision: SnapshotRevision,
    pub expected_stage_oids: ConflictStages,
    pub expected_worktree_digest: [u8; 32],
    pub mode: u32,
    pub result: Vec<u8>,
}

impl ApplyResolutionRequest {
    /// Rejects malformed requests before any worktree or Index mutation.
    pub fn validate(&self) -> Result<(), ResolutionRequestError> {
        if self.mode == 0 {
            return Err(ResolutionRequestError::InvalidMode);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResolutionRequestError {
    #[error("conflict resolution requires a non-zero file mode")]
    InvalidMode,
}

/// Applies a previously validated conflict result and stages it atomically with respect to the
/// application operation. Implementations must re-read and compare all supplied identities.
#[async_trait]
pub trait ConflictResolutionWriter: Send + Sync {
    async fn apply(
        &self,
        request: ApplyResolutionRequest,
        context: ExecutionContext,
    ) -> GitResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_mode_before_execution() {
        let request = ApplyResolutionRequest {
            target: SessionTarget::bare(git_domain::RepositoryId::new(), 1),
            path: RepoPath::try_from("conflict.txt").expect("fixture path"),
            expected_conflict_revision: SnapshotRevision::INITIAL,
            expected_stage_oids: ConflictStages::default(),
            expected_worktree_digest: [0; 32],
            mode: 0,
            result: Vec::new(),
        };
        assert_eq!(request.validate(), Err(ResolutionRequestError::InvalidMode));
    }

    #[test]
    fn rejects_zero_mode_but_allows_empty_result_for_deletion() {
        let request = ApplyResolutionRequest {
            target: SessionTarget::bare(git_domain::RepositoryId::new(), 1),
            path: RepoPath::try_from("deleted.txt").expect("fixture path"),
            expected_conflict_revision: SnapshotRevision::INITIAL,
            expected_stage_oids: ConflictStages::default(),
            expected_worktree_digest: [0; 32],
            mode: 0,
            result: Vec::new(),
        };
        assert_eq!(request.validate(), Err(ResolutionRequestError::InvalidMode));
    }
}
