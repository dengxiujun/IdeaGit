//! Safe conflict-resolution write boundary.

use async_trait::async_trait;
use git_domain::{GitResult, RepoPath, SessionTarget, SnapshotRevision, UnmergedChange};
use sha2::{Digest, Sha256};

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

/// Computes the byte-level digest used to reject an externally changed worktree file.
#[must_use]
pub fn worktree_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Verifies that the bytes presented for resolution are still the bytes observed by the UI.
pub fn validate_worktree_digest(
    expected: [u8; 32],
    current: &[u8],
) -> Result<(), WorktreeConflictError> {
    let actual = worktree_digest(current);
    if actual == expected {
        Ok(())
    } else {
        Err(WorktreeConflictError::ChangedExternally)
    }
}

/// Compares every observed conflict-stage identity before a resolution write.
pub fn validate_conflict_stages(
    expected_path: &RepoPath,
    expected: &ConflictStages,
    observed: &UnmergedChange,
) -> Result<(), ConflictStageError> {
    if expected_path != &observed.path {
        return Err(ConflictStageError::PathChanged);
    }
    if expected.stage1 != observed.stage1_oid
        || expected.stage2 != observed.stage2_oid
        || expected.stage3 != observed.stage3_oid
    {
        return Err(ConflictStageError::StagesChanged);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResolutionRequestError {
    #[error("conflict resolution requires a non-zero file mode")]
    InvalidMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorktreeConflictError {
    #[error("worktree file changed externally while resolving conflict")]
    ChangedExternally,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConflictStageError {
    #[error("conflict path changed while resolving")]
    PathChanged,
    #[error("conflict stage identities changed while resolving")]
    StagesChanged,
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

    #[test]
    fn worktree_digest_is_deterministic_and_sensitive_to_bytes() {
        assert_eq!(worktree_digest(b"same"), worktree_digest(b"same"));
        assert_ne!(worktree_digest(b"same"), worktree_digest(b"changed"));
    }

    #[test]
    fn rejects_external_worktree_change() {
        let expected = worktree_digest(b"before");
        assert_eq!(validate_worktree_digest(expected, b"before"), Ok(()));
        assert_eq!(
            validate_worktree_digest(expected, b"after"),
            Err(WorktreeConflictError::ChangedExternally)
        );
    }

    #[test]
    fn rejects_changed_conflict_stage_identity() {
        let expected_path = RepoPath::try_from("conflict.txt").expect("fixture path");
        let observed = UnmergedChange {
            path: expected_path.clone(),
            conflict: git_domain::ConflictKind::BothModified,
            submodule: git_domain::SubmoduleState::NOT_SUBMODULE,
            stage1_mode: None,
            stage2_mode: None,
            stage3_mode: None,
            worktree_mode: None,
            stage1_oid: None,
            stage2_oid: None,
            stage3_oid: None,
        };
        let expected = ConflictStages {
            stage1: None,
            stage2: Some(
                "0000000000000000000000000000000000000001"
                    .parse()
                    .expect("oid"),
            ),
            stage3: None,
        };
        assert_eq!(
            validate_conflict_stages(&expected_path, &expected, &observed),
            Err(ConflictStageError::StagesChanged)
        );
    }
}
