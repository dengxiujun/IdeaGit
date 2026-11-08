//! Application boundary for preparing a confirmed Selection Commit.

use git_domain::{
    ChangeSelection, CommitOptions, GitResult, ObjectId, OperationId, SelectionCommitRequest,
    SessionTarget, SnapshotRevision,
};
use git_ports::{
    CandidateTreeBuilder, DiffReadRequest, DiffReader, ExecutionContext, RepositoryLocation,
    TemporaryIndexManager,
};

use crate::{CandidateTreePreviewService, PreparedPartialCommit, SelectionPatchPreviewService};

/// Composes read-only selection preview and candidate-tree construction into a prepared
/// transaction. This service never commits and never publishes the real Index.
#[derive(Debug)]
pub struct SelectionCommitPreparationService<D, T, B> {
    patch_preview: SelectionPatchPreviewService<D>,
    candidate_preview: CandidateTreePreviewService<T, B>,
}

/// UI/application input for creating a Selection Commit preview. Fingerprints are deliberately
/// absent: the application computes them from the freshly read Diff instead of trusting UI data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionCommitPreviewRequest {
    pub target: SessionTarget,
    pub diff_request: DiffReadRequest,
    pub selection: ChangeSelection,
    pub options: CommitOptions,
    pub expected_head: Option<ObjectId>,
    pub expected_index: Option<ObjectId>,
    pub original_index_digest: [u8; 32],
    pub expected_revision: SnapshotRevision,
}

impl<D, T, B> SelectionCommitPreparationService<D, T, B> {
    #[must_use]
    pub fn new(diff_reader: D, temporary_indexes: T, candidate_trees: B) -> Self {
        Self {
            patch_preview: SelectionPatchPreviewService::new(diff_reader),
            candidate_preview: CandidateTreePreviewService::new(temporary_indexes, candidate_trees),
        }
    }
}

impl<D, T, B> SelectionCommitPreparationService<D, T, B>
where
    D: DiffReader,
    T: TemporaryIndexManager,
    B: CandidateTreeBuilder,
{
    /// Reads the current Diff, computes the selection identity, and prepares a candidate
    /// transaction. No Git commit or real Index publication occurs in this method.
    #[allow(clippy::result_large_err)]
    pub async fn prepare_preview(
        &self,
        location: &RepositoryLocation,
        operation: OperationId,
        request: SelectionCommitPreviewRequest,
        context: ExecutionContext,
    ) -> GitResult<PreparedPartialCommit> {
        if request.diff_request.target != request.target
            || request.selection.target() != request.target
        {
            return Err(git_domain::GitError::new(
                git_domain::GitErrorKind::StaleSelection,
                "selection preview target does not match the active session",
            )
            .with_target(request.target)
            .with_operation(operation));
        }
        let patch = self
            .patch_preview
            .prepare(
                location,
                request.target,
                request.diff_request.clone(),
                request.selection.clone(),
                context.clone(),
            )
            .await?;
        let commit_request = SelectionCommitRequest {
            options: request.options,
            selections: vec![request.selection],
            fingerprints: vec![git_domain::SelectionFingerprint {
                expected_head: request.expected_head,
                expected_index: request.expected_index,
                diff_generation: patch.generation,
                content_digest: patch.selection_digest.to_vec(),
            }],
            expected_revision: request.expected_revision,
        };
        let preview = self
            .candidate_preview
            .prepare(location, request.target, operation, patch, context)
            .await?;
        CandidateTreePreviewService::<T, B>::prepare_transaction(
            preview,
            commit_request,
            request.expected_head,
            request.expected_index,
            request.original_index_digest,
        )
    }

    /// Prepares a transaction from the current diff and candidate Index without performing a
    /// Git commit. The caller supplies the captured real Index digest so reconciliation remains
    /// bound to the same snapshot that was shown to the user.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::result_large_err)]
    pub async fn prepare(
        &self,
        location: &RepositoryLocation,
        target: SessionTarget,
        operation: OperationId,
        diff_request: DiffReadRequest,
        selection: ChangeSelection,
        commit_request: SelectionCommitRequest,
        expected_head: Option<ObjectId>,
        expected_index: Option<ObjectId>,
        original_index_digest: [u8; 32],
        context: ExecutionContext,
    ) -> GitResult<PreparedPartialCommit> {
        let patch = SelectionPatchPreviewService::prepare(
            &self.patch_preview,
            location,
            target,
            diff_request,
            selection,
            context.clone(),
        )
        .await?;
        let preview = self
            .candidate_preview
            .prepare(location, target, operation, patch, context)
            .await?;
        CandidateTreePreviewService::<T, B>::prepare_transaction(
            preview,
            commit_request,
            expected_head,
            expected_index,
            original_index_digest,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use git_diff::parse;
    use git_domain::{
        CommitOptions, FileIdentity, FileKind, RepoPath, RepositoryId, SigningMode,
        SnapshotRevision, WorktreeId,
    };
    use git_ports::{CandidateTreeRequest, DiffDocument, ProcessProgress, TemporaryIndexLease};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[derive(Debug, Clone, Copy)]
    struct Reader;

    #[async_trait]
    impl DiffReader for Reader {
        async fn read(
            &self,
            _location: &RepositoryLocation,
            _request: DiffReadRequest,
            _context: ExecutionContext,
        ) -> GitResult<DiffDocument> {
            Ok(DiffDocument {
                generation: SnapshotRevision::new(7),
                patch: parse(b"diff --git a/file.txt b/file.txt\n@@ -1 +1 @@\n-old\n+new\n")
                    .expect("patch"),
            })
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct Indexes;

    #[async_trait]
    impl TemporaryIndexManager for Indexes {
        async fn create(
            &self,
            _repository: &RepositoryLocation,
            operation: OperationId,
            _context: ExecutionContext,
        ) -> GitResult<TemporaryIndexLease> {
            let path = std::env::temp_dir().join(format!("ideagit-selection-{operation}.index"));
            std::fs::write(&path, b"candidate-index").expect("candidate index");
            Ok(TemporaryIndexLease {
                path,
                transaction_id: operation,
            })
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct Tree;

    #[async_trait]
    impl CandidateTreeBuilder for Tree {
        async fn build(
            &self,
            request: CandidateTreeRequest,
            _context: ExecutionContext,
        ) -> GitResult<ObjectId> {
            assert!(!request.patch.is_empty());
            "1111111111111111111111111111111111111111"
                .parse()
                .map_err(|_| git_domain::GitError::new(git_domain::GitErrorKind::Internal, "oid"))
        }
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(2);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    #[tokio::test]
    async fn prepares_candidate_transaction_without_writing_real_index() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let file = FileIdentity {
            target,
            path: RepoPath::try_from("file.txt").expect("path"),
            previous_path: None,
            kind: FileKind::Regular,
        };
        let selection = ChangeSelection::WholeFile { file };
        let request = SelectionCommitPreviewRequest {
            options: CommitOptions {
                message: b"prepared\n".to_vec(),
                amend: false,
                allow_empty: false,
                signoff: false,
                author: None,
                committer: None,
                signing: SigningMode::Disable,
            },
            target,
            diff_request: DiffReadRequest {
                target,
                before: git_diff::ContentSource::Head,
                after: git_diff::ContentSource::Worktree,
                paths: vec![RepoPath::try_from("file.txt").expect("path")],
                context_lines: 3,
            },
            selection: selection.clone(),
            expected_head: None,
            expected_index: None,
            original_index_digest: [9; 32],
            expected_revision: SnapshotRevision::new(7),
        };
        let service = SelectionCommitPreparationService::new(Reader, Indexes, Tree);
        let prepared = service
            .prepare_preview(
                &RepositoryLocation::worktree("/repo", "/repo/.git", "/repo/.git"),
                OperationId::new(),
                request,
                context(),
            )
            .await
            .expect("prepared transaction");

        assert!(prepared.transaction.candidate_tree.is_some());
        assert_eq!(prepared.transaction.original_index_digest, Some([9; 32]));
        assert_eq!(
            prepared.transaction.candidate_index,
            Some(b"candidate-index".to_vec())
        );
        let _ = std::fs::remove_file(prepared.preview.temporary_index.path.clone());
    }

    #[tokio::test]
    async fn rejects_preview_for_a_foreign_diff_target_before_creating_index() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let foreign = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let selection = ChangeSelection::WholeFile {
            file: FileIdentity {
                target,
                path: RepoPath::try_from("file.txt").expect("path"),
                previous_path: None,
                kind: FileKind::Regular,
            },
        };
        let service = SelectionCommitPreparationService::new(Reader, Indexes, Tree);
        let error = service
            .prepare_preview(
                &RepositoryLocation::worktree("/repo", "/repo/.git", "/repo/.git"),
                OperationId::new(),
                SelectionCommitPreviewRequest {
                    target,
                    diff_request: DiffReadRequest {
                        target: foreign,
                        before: git_diff::ContentSource::Head,
                        after: git_diff::ContentSource::Worktree,
                        paths: Vec::new(),
                        context_lines: 3,
                    },
                    selection,
                    options: CommitOptions {
                        message: b"foreign\n".to_vec(),
                        amend: false,
                        allow_empty: false,
                        signoff: false,
                        author: None,
                        committer: None,
                        signing: SigningMode::Disable,
                    },
                    expected_head: None,
                    expected_index: None,
                    original_index_digest: [0; 32],
                    expected_revision: SnapshotRevision::new(1),
                },
                context(),
            )
            .await
            .expect_err("foreign Preview target must be rejected");

        assert_eq!(error.kind, git_domain::GitErrorKind::StaleSelection);
    }
}
