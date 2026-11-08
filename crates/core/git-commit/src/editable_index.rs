use git_domain::{
    EditedIndexSessionId, GitError, GitErrorKind, GitResult, ObjectId, RepoPath, SessionTarget,
};
use git_ports::{
    ExecutionContext, IndexReconciliationOutcome, IndexReconciliationPublisher,
    IndexReconciliationRequest,
};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditableIndexSession {
    pub id: EditedIndexSessionId,
    pub target: SessionTarget,
    pub path: RepoPath,
    pub original_index_oid: Option<ObjectId>,
    pub original_head_oid: Option<ObjectId>,
    pub desired_content_digest: [u8; 32],
    pub dirty: bool,
    desired_content: Vec<u8>,
}

impl EditableIndexSession {
    pub fn new(
        target: SessionTarget,
        path: RepoPath,
        original_index_oid: Option<ObjectId>,
        original_head_oid: Option<ObjectId>,
        content: Vec<u8>,
    ) -> Self {
        Self {
            id: EditedIndexSessionId::new(),
            target,
            path,
            original_index_oid,
            original_head_oid,
            desired_content_digest: digest(&content),
            dirty: false,
            desired_content: content,
        }
    }

    pub fn edit(&mut self, content: Vec<u8>) {
        self.desired_content_digest = digest(&content);
        self.desired_content = content;
        self.dirty = true;
    }

    #[allow(clippy::result_large_err)]
    pub fn apply_preflight(
        &self,
        target: SessionTarget,
        current_index_oid: Option<ObjectId>,
        current_head_oid: Option<ObjectId>,
    ) -> GitResult<Vec<u8>> {
        if !self.dirty {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "editable Index session has no confirmed edit",
            ));
        }
        if self.target != target
            || self.original_index_oid != current_index_oid
            || self.original_head_oid != current_head_oid
        {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "editable Index session is based on stale repository identities",
            ));
        }
        Ok(self.desired_content.clone())
    }

    /// Applies a previously constructed candidate Index through the approved publisher port.
    /// The session never writes the real Index itself.
    #[allow(clippy::result_large_err)]
    #[allow(clippy::too_many_arguments)]
    pub async fn apply<P: IndexReconciliationPublisher>(
        &self,
        target: SessionTarget,
        current_index_oid: Option<ObjectId>,
        current_head_oid: Option<ObjectId>,
        original_index_digest: [u8; 32],
        candidate_index: Vec<u8>,
        candidate_content: &[u8],
        publisher: &P,
        context: ExecutionContext,
    ) -> GitResult<IndexReconciliationOutcome> {
        let desired = self.apply_preflight(target, current_index_oid, current_head_oid)?;
        if desired.as_slice() != candidate_content {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "candidate Index content does not match the edited session",
            ));
        }
        let outcome = publisher
            .publish(
                IndexReconciliationRequest {
                    target,
                    expected_index_digest: original_index_digest,
                    candidate_index,
                },
                context,
            )
            .await?;
        if outcome == IndexReconciliationOutcome::Conflict {
            return Err(GitError::new(
                GitErrorKind::IndexChangedExternally,
                "real Index changed while applying editable session",
            ));
        }
        Ok(outcome)
    }
}

fn digest(content: &[u8]) -> [u8; 32] {
    Sha256::digest(content).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use git_domain::{OperationId, RepositoryId, WorktreeId};
    use git_ports::ProcessProgress;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[derive(Debug)]
    struct PublishingPort {
        outcome: IndexReconciliationOutcome,
    }

    #[async_trait]
    impl IndexReconciliationPublisher for PublishingPort {
        async fn publish(
            &self,
            _request: IndexReconciliationRequest,
            _context: ExecutionContext,
        ) -> GitResult<IndexReconciliationOutcome> {
            Ok(self.outcome)
        }
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(4);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    #[test]
    fn apply_requires_dirty_session_and_matching_identities() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let mut session = EditableIndexSession::new(
            target,
            RepoPath::try_from("file.txt").expect("path"),
            None,
            None,
            b"base".to_vec(),
        );
        assert!(session.apply_preflight(target, None, None).is_err());
        session.edit(b"edited".to_vec());
        assert_eq!(
            session
                .apply_preflight(target, None, None)
                .expect("apply preflight"),
            b"edited"
        );
        assert!(
            session
                .apply_preflight(
                    target,
                    Some(
                        "1111111111111111111111111111111111111111"
                            .parse()
                            .expect("oid")
                    ),
                    None
                )
                .is_err()
        );
    }

    #[tokio::test]
    async fn apply_publishes_matching_candidate_through_reconciliation_port() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let mut session = EditableIndexSession::new(
            target,
            RepoPath::try_from("file.txt").expect("path"),
            None,
            None,
            b"base".to_vec(),
        );
        session.edit(b"edited".to_vec());
        let publisher = PublishingPort {
            outcome: IndexReconciliationOutcome::Published,
        };
        let result = session
            .apply(
                target,
                None,
                None,
                [7; 32],
                b"candidate-index".to_vec(),
                b"edited",
                &publisher,
                context(),
            )
            .await
            .expect("apply");
        assert_eq!(result, IndexReconciliationOutcome::Published);
    }

    #[tokio::test]
    async fn apply_rejects_candidate_content_that_does_not_match_session() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let mut session = EditableIndexSession::new(
            target,
            RepoPath::try_from("file.txt").expect("path"),
            None,
            None,
            b"base".to_vec(),
        );
        session.edit(b"edited".to_vec());
        let publisher = PublishingPort {
            outcome: IndexReconciliationOutcome::Published,
        };
        let error = session
            .apply(
                target,
                None,
                None,
                [7; 32],
                b"candidate-index".to_vec(),
                b"other",
                &publisher,
                context(),
            )
            .await
            .expect_err("mismatched content must be rejected");
        assert_eq!(error.kind, GitErrorKind::StaleSelection);
    }
}
