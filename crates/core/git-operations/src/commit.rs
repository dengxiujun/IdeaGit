//! Commit preflight composition.

use async_trait::async_trait;
use git_domain::{
    GitError, GitErrorKind, GitResult, ObjectId, SessionTarget, StandardCommitRequest,
};
use git_ports::{CommitOutcome, CommitWriter, ExecutionContext};

/// Authoritative state needed before starting a real commit process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommitPreflightState {
    pub head: Option<ObjectId>,
    pub index: Option<ObjectId>,
}

#[async_trait]
pub trait CommitStateReader: Send + Sync {
    async fn read(
        &self,
        target: SessionTarget,
        context: ExecutionContext,
    ) -> GitResult<CommitPreflightState>;
}

/// Rejects stale commit requests before invoking the Git CLI writer.
#[derive(Debug)]
pub struct PreflightCommitWriter<W, R> {
    writer: W,
    reader: R,
}

impl<W, R> PreflightCommitWriter<W, R> {
    #[must_use]
    pub fn new(writer: W, reader: R) -> Self {
        Self { writer, reader }
    }
}

#[async_trait]
impl<W: CommitWriter, R: CommitStateReader> CommitWriter for PreflightCommitWriter<W, R> {
    #[allow(clippy::result_large_err)]
    async fn commit(
        &self,
        target: SessionTarget,
        request: StandardCommitRequest,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        let observed = self.reader.read(target, context.clone()).await?;
        if observed.head != request.expected_head {
            return Err(GitError::new(
                GitErrorKind::HeadChangedExternally,
                "HEAD changed since commit preview",
            )
            .with_target(target)
            .with_operation(context.operation_id));
        }
        if observed.index != request.expected_index {
            return Err(GitError::new(
                GitErrorKind::IndexChangedExternally,
                "Index changed since commit preview",
            )
            .with_target(target)
            .with_operation(context.operation_id));
        }
        self.writer.commit(target, request, context).await
    }
}
