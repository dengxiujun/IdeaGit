//! Commit preflight composition.

use async_trait::async_trait;
use git_domain::{
    GitCommand, GitError, GitErrorKind, GitResult, JournalPhase, JournalRecord, JournalResult,
    ObjectId, SessionTarget, StandardCommitRequest,
};
use git_ports::{CommitOutcome, CommitWriter, ExecutionContext, JournalStore};

use crate::{ResourceKey, ResourceKind, ResourceLockManager, ResourceScope};

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

/// Durable Journal wrapper for standard commits.
#[derive(Debug)]
pub struct JournaledCommitWriter<W, J> {
    writer: W,
    journal: J,
}

impl<W, J> JournaledCommitWriter<W, J> {
    #[must_use]
    pub fn new(writer: W, journal: J) -> Self {
        Self { writer, journal }
    }
}

#[async_trait]
impl<W: CommitWriter, J: JournalStore> CommitWriter for JournaledCommitWriter<W, J> {
    #[allow(clippy::result_large_err)]
    async fn commit(
        &self,
        target: SessionTarget,
        request: StandardCommitRequest,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        let mut record = JournalRecord::intent(
            context.operation_id,
            GitCommand::CommitIndex {
                target,
                request: request.clone(),
            },
        );
        record.expected_head = request.expected_head;
        record.expected_index = request.expected_index;
        record.phase = JournalPhase::CommitInProgress;
        self.journal.append(record.clone()).await?;
        let result = self.writer.commit(target, request, context).await;
        match &result {
            Ok(outcome) => {
                record.phase = JournalPhase::Completed;
                record.observed_commit = Some(outcome.commit);
                record.observed_parents = outcome.parents.clone();
                record.result = Some(JournalResult::Completed);
            }
            Err(error) => {
                record.phase = JournalPhase::RecoveryRequired;
                record.result = Some(JournalResult::RecoveryRequired {
                    reason: error.to_string(),
                });
            }
        }
        self.journal.replace(record).await?;
        result
    }
}

/// Commit writer that serializes shared refs and the concrete worktree Index.
#[derive(Debug)]
pub struct CoordinatedCommitWriter<W> {
    writer: W,
    locks: ResourceLockManager,
}

impl<W> CoordinatedCommitWriter<W> {
    #[must_use]
    pub fn new(writer: W, locks: ResourceLockManager) -> Self {
        Self { writer, locks }
    }
}

#[async_trait]
impl<W: CommitWriter> CommitWriter for CoordinatedCommitWriter<W> {
    #[allow(clippy::result_large_err)]
    async fn commit(
        &self,
        target: SessionTarget,
        request: StandardCommitRequest,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        let worktree = target.worktree_id().ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "commit is unavailable for a bare repository",
            )
        })?;
        let _lock = self
            .locks
            .acquire_with_cancellation(
                [
                    ResourceKey {
                        scope: ResourceScope::CommonGitDir(target.repository()),
                        kind: ResourceKind::Refs,
                    },
                    ResourceKey {
                        scope: ResourceScope::Worktree(worktree),
                        kind: ResourceKind::Index,
                    },
                ],
                context.cancellation.clone(),
            )
            .await
            .map_err(|error| GitError::new(GitErrorKind::Cancelled, error.to_string()))?;
        self.writer.commit(target, request, context).await
    }
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
