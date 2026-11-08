//! Application boundary for executing a confirmed Partial Commit transaction.

use async_trait::async_trait;
use git_commit::{PartialCommitCoordinator, PartialCommitTransaction};
use git_domain::{
    GitError, GitErrorKind, GitResult, ObjectId, OperationId, SelectionCommitRequest, SessionTarget,
};
use git_operations::SelectionCommitHandler;
use git_ports::{
    CandidateTreeBuilder, CommitOutcome, CommitSideEffectObserver, CommitWriter, DiffReader,
    ExecutionContext, IndexReconciliationPublisher, JournalStore, RepositoryLocation,
    TemporaryIndexManager,
};
use tokio::sync::Mutex;

use crate::PreparedPartialCommit;

/// Port used by the application layer to hand an already confirmed transaction to Git writers.
///
/// Implementations are expected to be composed behind `OperationCoordinator`; this boundary
/// deliberately does not accept a plain `SelectionCommitRequest`, which would lose the prepared
/// Candidate Tree and Index identity.
#[async_trait]
pub trait ConfirmedPartialCommitRunner: Send + Sync {
    async fn commit(
        &self,
        target: SessionTarget,
        operation: OperationId,
        transaction: &mut PartialCommitTransaction,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome>;
}

#[async_trait]
impl<W, J, O, P> ConfirmedPartialCommitRunner for PartialCommitCoordinator<W, J, O, P>
where
    W: CommitWriter + Send + Sync,
    J: JournalStore + Send + Sync,
    O: CommitSideEffectObserver + Send + Sync,
    P: IndexReconciliationPublisher + Send + Sync,
{
    async fn commit(
        &self,
        target: SessionTarget,
        operation: OperationId,
        transaction: &mut PartialCommitTransaction,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        PartialCommitCoordinator::commit(self, target, operation, transaction, context).await
    }
}

/// Read-only identity probe used immediately before a confirmed Selection Commit.
#[async_trait]
pub trait SelectionCommitStateReader: Send + Sync {
    async fn read(
        &self,
        target: SessionTarget,
        context: ExecutionContext,
    ) -> GitResult<(Option<ObjectId>, Option<ObjectId>)>;
}

/// Confirms a prepared Partial Commit immediately before dispatching it to the write boundary.
#[derive(Debug)]
pub struct SelectionCommitExecutionService<R> {
    runner: R,
}

impl<R> SelectionCommitExecutionService<R> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self { runner }
    }
}

impl<R: ConfirmedPartialCommitRunner> SelectionCommitExecutionService<R> {
    #[allow(clippy::result_large_err)]
    pub async fn execute(
        &self,
        operation: OperationId,
        prepared: &mut PreparedPartialCommit,
        head: Option<ObjectId>,
        index: Option<ObjectId>,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        prepared.transaction.revalidate_before_commit(head, index)?;
        self.runner
            .commit(
                prepared.preview.target,
                operation,
                &mut prepared.transaction,
                context,
            )
            .await
    }
}

/// Bridges a prepared application preview to the OperationCoordinator handler contract.
#[derive(Debug)]
pub struct PreparedSelectionCommitHandler<R, P> {
    service: SelectionCommitExecutionService<R>,
    state: P,
    prepared: Mutex<Option<PreparedPartialCommit>>,
}

/// Application-owned Preview → Confirm session for Changelist commits.
///
/// The session keeps preparation and confirmation in one boundary: preparation only arms a
/// one-shot Handler, while confirmation is the only operation that can reach the commit runner.
#[derive(Debug)]
pub struct SelectionCommitPreviewSession<D, T, B, R, P> {
    preparation: crate::SelectionCommitPreparationService<D, T, B>,
    handler: PreparedSelectionCommitHandler<R, P>,
}

impl<D, T, B, R, P> SelectionCommitPreviewSession<D, T, B, R, P> {
    #[must_use]
    pub fn new(
        preparation: crate::SelectionCommitPreparationService<D, T, B>,
        handler: PreparedSelectionCommitHandler<R, P>,
    ) -> Self {
        Self {
            preparation,
            handler,
        }
    }
}

impl<D, T, B, R, P> SelectionCommitPreviewSession<D, T, B, R, P>
where
    D: DiffReader,
    T: TemporaryIndexManager,
    B: CandidateTreeBuilder,
    R: ConfirmedPartialCommitRunner + 'static,
    P: SelectionCommitStateReader + 'static,
{
    #[allow(clippy::result_large_err)]
    pub async fn prepare_preview(
        &self,
        location: &RepositoryLocation,
        operation: OperationId,
        request: crate::SelectionCommitPreviewRequest,
        context: ExecutionContext,
    ) -> GitResult<SelectionCommitRequest> {
        let prepared = self
            .preparation
            .prepare_preview(location, operation, request, context)
            .await?;
        let commit_request = prepared.transaction.request.clone();
        self.handler.arm(prepared).await;
        Ok(commit_request)
    }

    #[allow(clippy::result_large_err)]
    pub async fn confirm(
        &self,
        target: SessionTarget,
        request: SelectionCommitRequest,
        context: ExecutionContext,
    ) -> GitResult<()> {
        self.handler
            .commit_selection(target, request, context)
            .await
    }
}

impl<R, P> PreparedSelectionCommitHandler<R, P> {
    #[must_use]
    pub fn new(service: SelectionCommitExecutionService<R>, state: P) -> Self {
        Self {
            service,
            state,
            prepared: Mutex::new(None),
        }
    }

    pub async fn arm(&self, prepared: PreparedPartialCommit) {
        *self.prepared.lock().await = Some(prepared);
    }
}

#[async_trait]
impl<R, P> SelectionCommitHandler for PreparedSelectionCommitHandler<R, P>
where
    R: ConfirmedPartialCommitRunner + 'static,
    P: SelectionCommitStateReader + 'static,
{
    async fn commit_selection(
        &self,
        target: SessionTarget,
        request: SelectionCommitRequest,
        context: ExecutionContext,
    ) -> GitResult<()> {
        // Consume the armed preview before any irreversible work. A confirmation is one-shot;
        // recovery, if needed, is represented by the transaction Journal rather than a silent
        // retry of the same prepared identity.
        let mut prepared = {
            let mut armed = self.prepared.lock().await;
            let Some(prepared) = armed.as_ref() else {
                return Err(GitError::new(
                    GitErrorKind::StaleSelection,
                    "selection commit preview is no longer armed",
                )
                .with_target(target)
                .with_operation(context.operation_id));
            };
            if prepared.preview.target != target || prepared.transaction.request != request {
                return Err(GitError::new(
                    GitErrorKind::StaleSelection,
                    "selection commit request does not match the armed preview",
                )
                .with_target(target)
                .with_operation(context.operation_id));
            }
            match armed.take() {
                Some(prepared) => prepared,
                None => {
                    return Err(GitError::new(
                        GitErrorKind::StaleSelection,
                        "selection commit preview is no longer armed",
                    )
                    .with_target(target)
                    .with_operation(context.operation_id));
                }
            }
        };
        let (head, index) = self.state.read(target, context.clone()).await?;
        self.service
            .execute(context.operation_id, &mut prepared, head, index, context)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{ChangeSelection, FileIdentity, FileKind, RepoPath, SnapshotRevision};
    use git_ports::TemporaryIndexLease;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[derive(Debug, Clone)]
    struct Runner {
        calls: Arc<Mutex<u32>>,
    }

    #[derive(Debug, Clone, Copy)]
    struct State;

    #[async_trait]
    impl SelectionCommitStateReader for State {
        async fn read(
            &self,
            _target: SessionTarget,
            _context: ExecutionContext,
        ) -> GitResult<(Option<ObjectId>, Option<ObjectId>)> {
            Ok((None, None))
        }
    }

    #[async_trait]
    impl ConfirmedPartialCommitRunner for Runner {
        async fn commit(
            &self,
            _target: SessionTarget,
            _operation: OperationId,
            _transaction: &mut PartialCommitTransaction,
            _context: ExecutionContext,
        ) -> GitResult<CommitOutcome> {
            *self.calls.lock().expect("calls lock") += 1;
            Err(git_domain::GitError::new(
                git_domain::GitErrorKind::Internal,
                "test runner should not be called",
            ))
        }
    }

    fn prepared() -> PreparedPartialCommit {
        let target = SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        );
        let file = FileIdentity {
            target,
            path: RepoPath::try_from("file.txt").expect("path"),
            previous_path: None,
            kind: FileKind::Regular,
        };
        let selection = ChangeSelection::WholeFile { file };
        let request = git_domain::SelectionCommitRequest {
            options: git_domain::CommitOptions {
                message: b"test\n".to_vec(),
                amend: false,
                allow_empty: false,
                signoff: false,
                author: None,
                committer: None,
                signing: git_domain::SigningMode::Disable,
            },
            selections: vec![selection.clone()],
            fingerprints: vec![git_domain::SelectionFingerprint {
                expected_head: None,
                expected_index: None,
                diff_generation: SnapshotRevision::new(1),
                content_digest: vec![1; 32],
            }],
            expected_revision: SnapshotRevision::new(1),
        };
        PreparedPartialCommit {
            preview: crate::CandidateTreePreview {
                target,
                operation_id: OperationId::new(),
                candidate_tree: "1111111111111111111111111111111111111111"
                    .parse()
                    .expect("tree"),
                patch: crate::SelectionPatchPreview {
                    target,
                    generation: SnapshotRevision::new(1),
                    selection,
                    patch: Vec::new(),
                    selection_digest: [1; 32],
                },
                temporary_index: TemporaryIndexLease {
                    path: PathBuf::from("/tmp/ideagit-test-index"),
                    transaction_id: OperationId::new(),
                },
            },
            transaction: PartialCommitTransaction::begin(request).expect("transaction"),
        }
    }

    #[tokio::test]
    async fn stale_confirmation_never_reaches_the_commit_runner() {
        let calls = Arc::new(Mutex::new(0));
        let service = SelectionCommitExecutionService::new(Runner {
            calls: Arc::clone(&calls),
        });
        let mut prepared = prepared();
        let (progress, _receiver) = mpsc::channel(1);
        let error = service
            .execute(
                OperationId::new(),
                &mut prepared,
                Some(
                    "2222222222222222222222222222222222222222"
                        .parse()
                        .expect("head"),
                ),
                None,
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect_err("unprepared transaction must be rejected");
        assert_eq!(error.kind, git_domain::GitErrorKind::Internal);
        assert_eq!(*calls.lock().expect("calls lock"), 0);
    }

    #[tokio::test]
    async fn armed_handler_reads_identity_before_reaching_runner() {
        let calls = Arc::new(Mutex::new(0));
        let service = SelectionCommitExecutionService::new(Runner {
            calls: Arc::clone(&calls),
        });
        let handler = PreparedSelectionCommitHandler::new(service, State);
        let mut prepared = prepared();
        let target = prepared.preview.target;
        prepared
            .transaction
            .temp_index_prepared(None, None)
            .expect("temporary Index");
        prepared
            .transaction
            .candidate_ready(prepared.preview.candidate_tree)
            .expect("candidate tree");
        prepared
            .transaction
            .reconciliation_candidate([1; 32], b"candidate-index".to_vec())
            .expect("candidate Index");
        let request = prepared.transaction.request.clone();
        handler.arm(prepared).await;
        let (progress, _receiver) = mpsc::channel(1);
        let error = handler
            .commit_selection(
                target,
                request.clone(),
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect_err("test runner returns a sentinel error");
        assert_eq!(error.kind, git_domain::GitErrorKind::Internal);
        assert_eq!(*calls.lock().expect("calls lock"), 1);

        let (progress, _receiver) = mpsc::channel(1);
        let stale = handler
            .commit_selection(
                target,
                request,
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect_err("a confirmed preview must not be reusable");
        assert_eq!(stale.kind, GitErrorKind::StaleSelection);
        assert_eq!(*calls.lock().expect("calls lock"), 1);
    }

    #[derive(Debug, Clone, Copy)]
    struct RecoveryRunner;

    #[async_trait]
    impl ConfirmedPartialCommitRunner for RecoveryRunner {
        async fn commit(
            &self,
            _target: SessionTarget,
            _operation: OperationId,
            transaction: &mut PartialCommitTransaction,
            _context: ExecutionContext,
        ) -> GitResult<CommitOutcome> {
            transaction.phase = git_commit::PartialCommitPhase::RecoveryRequired;
            Err(git_domain::GitError::new(
                git_domain::GitErrorKind::HooksMutatedRepository,
                "commit side effects require recovery",
            ))
        }
    }

    #[tokio::test]
    async fn handler_propagates_recovery_failure_without_reporting_success() {
        let handler = PreparedSelectionCommitHandler::new(
            SelectionCommitExecutionService::new(RecoveryRunner),
            State,
        );
        let mut prepared = prepared();
        let target = prepared.preview.target;
        prepared
            .transaction
            .temp_index_prepared(None, None)
            .expect("temporary Index");
        prepared
            .transaction
            .candidate_ready(prepared.preview.candidate_tree)
            .expect("candidate tree");
        prepared
            .transaction
            .reconciliation_candidate([1; 32], b"candidate-index".to_vec())
            .expect("candidate Index");
        let request = prepared.transaction.request.clone();
        handler.arm(prepared).await;

        let (progress, _receiver) = mpsc::channel(1);
        let error = handler
            .commit_selection(
                target,
                request,
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect_err("recovery failure must not be reported as success");

        assert_eq!(error.kind, git_domain::GitErrorKind::HooksMutatedRepository);
    }
}
