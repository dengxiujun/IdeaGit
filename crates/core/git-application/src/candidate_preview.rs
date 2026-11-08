use git_commit::{PartialCommitPhase, PartialCommitTransaction};
use git_domain::{GitError, GitErrorKind, GitResult, ObjectId, OperationId, SessionTarget};
use git_ports::{
    CandidateTreeBuilder, CandidateTreeRequest, ExecutionContext, RepositoryLocation,
    TemporaryIndexLease, TemporaryIndexManager,
};

use crate::SelectionPatchPreview;

#[derive(Debug)]
pub struct CandidateTreePreview {
    pub target: SessionTarget,
    pub operation_id: OperationId,
    pub candidate_tree: ObjectId,
    pub patch: SelectionPatchPreview,
    pub temporary_index: TemporaryIndexLease,
}

#[derive(Debug)]
pub struct PreparedPartialCommit {
    pub preview: CandidateTreePreview,
    pub transaction: PartialCommitTransaction,
}

#[derive(Debug)]
pub struct CandidateTreePreviewService<T, B> {
    temporary_indexes: T,
    candidate_trees: B,
}

impl<T, B> CandidateTreePreviewService<T, B> {
    #[must_use]
    pub fn new(temporary_indexes: T, candidate_trees: B) -> Self {
        Self {
            temporary_indexes,
            candidate_trees,
        }
    }
}

impl<T: TemporaryIndexManager, B: CandidateTreeBuilder> CandidateTreePreviewService<T, B> {
    #[allow(clippy::result_large_err)]
    pub async fn prepare(
        &self,
        location: &RepositoryLocation,
        target: SessionTarget,
        operation_id: OperationId,
        patch: SelectionPatchPreview,
        context: ExecutionContext,
    ) -> GitResult<CandidateTreePreview> {
        if patch.target != target {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "candidate tree target does not match the active session",
            )
            .with_target(target)
            .with_operation(operation_id));
        }
        let temporary_index = self
            .temporary_indexes
            .create(location, operation_id, context.clone())
            .await?;
        let worktree_root = location.worktree_root().ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "candidate trees require a worktree",
            )
            .with_target(target)
            .with_operation(operation_id)
        })?;
        let candidate_tree = self
            .candidate_trees
            .build(
                CandidateTreeRequest {
                    temporary_index: temporary_index.path.clone(),
                    worktree_root: worktree_root.to_path_buf(),
                    patch: patch.patch.clone(),
                },
                context,
            )
            .await?;
        Ok(CandidateTreePreview {
            target,
            operation_id,
            candidate_tree,
            patch,
            temporary_index,
        })
    }

    #[allow(clippy::result_large_err)]
    pub fn prepare_transaction(
        preview: CandidateTreePreview,
        request: git_domain::SelectionCommitRequest,
        expected_head: Option<ObjectId>,
        expected_index: Option<ObjectId>,
        original_index_digest: [u8; 32],
    ) -> GitResult<PreparedPartialCommit> {
        if request.selections.len() != 1 || request.selections[0] != preview.patch.selection {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "selection request does not match the prepared preview",
            )
            .with_target(preview.target)
            .with_operation(preview.operation_id));
        }
        let candidate_index = std::fs::read(&preview.temporary_index.path).map_err(|error| {
            GitError::new(
                GitErrorKind::Internal,
                "candidate Index could not be read for transaction preparation",
            )
            .with_diagnostic(error.to_string())
            .with_target(preview.target)
            .with_operation(preview.operation_id)
        })?;
        let mut transaction = PartialCommitTransaction::begin(request)?;
        transaction.temp_index_prepared(expected_head, expected_index)?;
        transaction.candidate_ready(preview.candidate_tree)?;
        transaction.reconciliation_candidate(original_index_digest, candidate_index)?;
        debug_assert_eq!(transaction.phase, PartialCommitPhase::PreviewReady);
        Ok(PreparedPartialCommit {
            preview,
            transaction,
        })
    }

    #[allow(clippy::result_large_err)]
    pub fn confirm_transaction(
        prepared: &mut PreparedPartialCommit,
        head: Option<ObjectId>,
        index: Option<ObjectId>,
    ) -> GitResult<ObjectId> {
        prepared
            .transaction
            .revalidate_before_commit(head, index)
            .map(|()| prepared.preview.candidate_tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use git_domain::{RepositoryId, WorktreeId};
    use git_ports::ProcessProgress;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[derive(Clone)]
    struct Indexes {
        path: PathBuf,
        operation: Arc<Mutex<Option<OperationId>>>,
    }

    #[async_trait]
    impl TemporaryIndexManager for Indexes {
        async fn create(
            &self,
            _repository: &RepositoryLocation,
            operation: OperationId,
            _context: ExecutionContext,
        ) -> GitResult<TemporaryIndexLease> {
            *self.operation.lock().expect("operation lock") = Some(operation);
            std::fs::write(&self.path, b"candidate-index").map_err(|error| {
                GitError::new(GitErrorKind::PermissionDenied, "test index write")
                    .with_diagnostic(error.to_string())
            })?;
            Ok(TemporaryIndexLease {
                path: self.path.clone(),
                transaction_id: operation,
            })
        }
    }

    #[derive(Clone)]
    struct Trees {
        seen: Arc<Mutex<Option<CandidateTreeRequest>>>,
    }

    #[async_trait]
    impl CandidateTreeBuilder for Trees {
        async fn build(
            &self,
            request: CandidateTreeRequest,
            _context: ExecutionContext,
        ) -> GitResult<ObjectId> {
            *self.seen.lock().expect("request lock") = Some(request);
            "1111111111111111111111111111111111111111"
                .parse()
                .map_err(|_| GitError::new(GitErrorKind::Internal, "test object id"))
        }
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(2);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn patch(target: SessionTarget) -> SelectionPatchPreview {
        SelectionPatchPreview {
            target,
            generation: git_domain::SnapshotRevision::new(1),
            selection: git_domain::ChangeSelection::WholeFile {
                file: git_domain::FileIdentity {
                    target,
                    path: git_domain::RepoPath::try_from("file.txt").expect("path"),
                    previous_path: None,
                    kind: git_domain::FileKind::Regular,
                },
            },
            patch: b"patch".to_vec(),
            selection_digest: [7; 32],
        }
    }

    #[tokio::test]
    async fn prepares_candidate_tree_and_keeps_the_index_lease_alive() {
        let target = target();
        let operation = OperationId::new();
        let path = std::env::temp_dir().join(format!("ideagit-preview-{operation}"));
        let indexes = Indexes {
            path: path.clone(),
            operation: Arc::new(Mutex::new(None)),
        };
        let trees = Trees {
            seen: Arc::new(Mutex::new(None)),
        };
        let service = CandidateTreePreviewService::new(indexes.clone(), trees.clone());
        let location = RepositoryLocation::worktree("/repo", "/repo/.git", "/repo/.git");
        let preview = service
            .prepare(&location, target, operation, patch(target), context())
            .await
            .expect("candidate preview");
        assert_eq!(preview.operation_id, operation);
        assert_eq!(preview.temporary_index.path, path);
        assert_eq!(
            *indexes.operation.lock().expect("operation lock"),
            Some(operation)
        );
        assert_eq!(
            trees
                .seen
                .lock()
                .expect("request lock")
                .as_ref()
                .map(|request| request.patch.as_slice()),
            Some(b"patch".as_slice())
        );
        drop(preview);
    }

    #[tokio::test]
    async fn binds_candidate_tree_and_index_identity_before_confirmation() {
        let target = target();
        let operation = OperationId::new();
        let path = std::env::temp_dir().join(format!("ideagit-transaction-{operation}"));
        let service = CandidateTreePreviewService::new(
            Indexes {
                path,
                operation: Arc::new(Mutex::new(None)),
            },
            Trees {
                seen: Arc::new(Mutex::new(None)),
            },
        );
        let preview = service
            .prepare(
                &RepositoryLocation::worktree("/repo", "/repo/.git", "/repo/.git"),
                target,
                operation,
                patch(target),
                context(),
            )
            .await
            .expect("preview");
        let request = git_domain::SelectionCommitRequest {
            options: git_domain::CommitOptions {
                message: b"partial\n".to_vec(),
                amend: false,
                allow_empty: false,
                signoff: false,
                author: None,
                committer: None,
                signing: git_domain::SigningMode::Disable,
            },
            selections: vec![preview.patch.selection.clone()],
            fingerprints: vec![git_domain::SelectionFingerprint {
                expected_head: None,
                expected_index: None,
                diff_generation: preview.patch.generation,
                content_digest: preview.patch.selection_digest.to_vec(),
            }],
            expected_revision: git_domain::SnapshotRevision::new(1),
        };
        let mut prepared = CandidateTreePreviewService::<Indexes, Trees>::prepare_transaction(
            preview, request, None, None, [3; 32],
        )
        .expect("transaction");
        assert_eq!(prepared.transaction.phase, PartialCommitPhase::PreviewReady);
        assert_eq!(
            prepared.transaction.candidate_tree,
            Some(prepared.preview.candidate_tree)
        );
        assert_eq!(prepared.transaction.original_index_digest, Some([3; 32]));
        let error = CandidateTreePreviewService::<Indexes, Trees>::confirm_transaction(
            &mut prepared,
            Some(
                "1111111111111111111111111111111111111111"
                    .parse()
                    .expect("oid"),
            ),
            None,
        )
        .expect_err("changed HEAD must expire the transaction");
        assert_eq!(error.kind, GitErrorKind::HeadChangedExternally);
        assert_eq!(
            prepared.transaction.phase,
            PartialCommitPhase::RecoveryRequired
        );
    }

    #[tokio::test]
    async fn rejects_a_candidate_preview_for_another_session() {
        let service = CandidateTreePreviewService::new(
            Indexes {
                path: PathBuf::from("/tmp/unused"),
                operation: Arc::new(Mutex::new(None)),
            },
            Trees {
                seen: Arc::new(Mutex::new(None)),
            },
        );
        let error = service
            .prepare(
                &RepositoryLocation::worktree("/repo", "/repo/.git", "/repo/.git"),
                target(),
                OperationId::new(),
                patch(target()),
                context(),
            )
            .await
            .expect_err("foreign preview");
        assert_eq!(error.kind, GitErrorKind::StaleSelection);
    }
}
