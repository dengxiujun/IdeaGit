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
