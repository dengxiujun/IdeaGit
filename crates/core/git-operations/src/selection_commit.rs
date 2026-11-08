//! OperationCoordinator boundary for confirmed Selection Commit handlers.

use async_trait::async_trait;
use git_domain::{
    GitCommand, GitError, GitErrorKind, GitResult, SelectionCommitRequest, SessionTarget,
};
use git_ports::ExecutionContext;
use std::sync::Arc;

use crate::OperationExecutor;

/// Application handler for a Selection Commit that has its preview/identity lifecycle managed.
#[async_trait]
pub trait SelectionCommitHandler: Send + Sync + 'static {
    async fn commit_selection(
        &self,
        target: SessionTarget,
        request: SelectionCommitRequest,
        context: ExecutionContext,
    ) -> GitResult<()>;
}

#[async_trait]
impl<H: SelectionCommitHandler + ?Sized> SelectionCommitHandler for Arc<H> {
    async fn commit_selection(
        &self,
        target: SessionTarget,
        request: SelectionCommitRequest,
        context: ExecutionContext,
    ) -> GitResult<()> {
        (**self).commit_selection(target, request, context).await
    }
}

/// Routes only `CommitSelection` through the OperationCoordinator executor boundary.
#[derive(Debug)]
pub struct SelectionCommitOperationExecutor<H> {
    target: SessionTarget,
    handler: H,
}

impl<H> SelectionCommitOperationExecutor<H> {
    #[must_use]
    pub fn new(target: SessionTarget, handler: H) -> Self {
        Self { target, handler }
    }
}

#[async_trait]
impl<H: SelectionCommitHandler> OperationExecutor for SelectionCommitOperationExecutor<H> {
    async fn execute(&self, command: GitCommand, context: ExecutionContext) -> GitResult<()> {
        if command.target() != self.target {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "selection commit target is no longer the active repository session",
            )
            .with_target(command.target())
            .with_operation(context.operation_id));
        }
        let GitCommand::CommitSelection { target, request } = command else {
            return Err(GitError::new(
                GitErrorKind::UnsupportedFeature,
                "selection commit executor accepts only CommitSelection",
            ));
        };
        self.handler
            .commit_selection(target, request, context)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{
        CommitOptions, OperationId, RepositoryId, SigningMode, SnapshotRevision, WorktreeId,
    };
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[derive(Debug, Clone)]
    struct Handler {
        calls: Arc<Mutex<u32>>,
    }

    #[async_trait]
    impl SelectionCommitHandler for Handler {
        async fn commit_selection(
            &self,
            _target: SessionTarget,
            _request: SelectionCommitRequest,
            _context: ExecutionContext,
        ) -> GitResult<()> {
            *self.calls.lock().expect("calls lock") += 1;
            Ok(())
        }
    }

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel(1);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn request() -> SelectionCommitRequest {
        SelectionCommitRequest {
            options: CommitOptions {
                message: b"selection\n".to_vec(),
                amend: false,
                allow_empty: false,
                signoff: false,
                author: None,
                committer: None,
                signing: SigningMode::Disable,
            },
            selections: Vec::new(),
            fingerprints: Vec::new(),
            expected_revision: SnapshotRevision::new(1),
        }
    }

    #[tokio::test]
    async fn routes_only_selection_commit_without_downgrading() {
        let calls = Arc::new(Mutex::new(0));
        let bound_target = target();
        let executor = SelectionCommitOperationExecutor::new(
            bound_target,
            Handler {
                calls: Arc::clone(&calls),
            },
        );
        executor
            .execute(
                GitCommand::CommitSelection {
                    target: bound_target,
                    request: request(),
                },
                context(),
            )
            .await
            .expect("selection command should reach handler");
        assert_eq!(*calls.lock().expect("calls lock"), 1);
    }

    #[tokio::test]
    async fn rejects_foreign_target_before_handler() {
        let calls = Arc::new(Mutex::new(0));
        let bound_target = target();
        let executor = SelectionCommitOperationExecutor::new(
            bound_target,
            Handler {
                calls: Arc::clone(&calls),
            },
        );
        let error = executor
            .execute(
                GitCommand::CommitSelection {
                    target: SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 2),
                    request: request(),
                },
                context(),
            )
            .await
            .expect_err("foreign target must be rejected");
        assert_eq!(error.kind, GitErrorKind::StaleSelection);
        assert_eq!(*calls.lock().expect("calls lock"), 0);
    }
}
