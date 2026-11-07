//! Branch operation composition.

use async_trait::async_trait;
use git_domain::{
    BranchCommand, GitCommand, GitError, GitErrorKind, GitResult, JournalPhase, JournalRecord,
    JournalResult, RepositorySnapshot, SessionTarget,
};
use git_ports::{BranchWriter, ExecutionContext, JournalStore};

use crate::{ResourceKey, ResourceKind, ResourceLockManager, ResourceScope, SnapshotRebuilder};

#[derive(Debug)]
pub struct CoordinatedBranchWriter<W> {
    writer: W,
    locks: ResourceLockManager,
}

impl<W> CoordinatedBranchWriter<W> {
    #[must_use]
    pub fn new(writer: W, locks: ResourceLockManager) -> Self {
        Self { writer, locks }
    }
}

#[derive(Debug)]
pub struct JournaledBranchWriter<W, J> {
    writer: W,
    journal: J,
}

impl<W, J> JournaledBranchWriter<W, J> {
    #[must_use]
    pub fn new(writer: W, journal: J) -> Self {
        Self { writer, journal }
    }
}

#[async_trait]
impl<W: BranchWriter, J: JournalStore> BranchWriter for JournaledBranchWriter<W, J> {
    #[allow(clippy::result_large_err)]
    async fn apply(
        &self,
        target: SessionTarget,
        command: BranchCommand,
        context: ExecutionContext,
    ) -> GitResult<()> {
        let mut record = JournalRecord::intent(
            context.operation_id,
            GitCommand::Branch {
                target,
                command: command.clone(),
            },
        );
        record.phase = JournalPhase::CommitInProgress;
        self.journal.append(record.clone()).await?;
        let result = self.writer.apply(target, command, context).await;
        record.phase = if result.is_ok() {
            JournalPhase::Completed
        } else {
            JournalPhase::RecoveryRequired
        };
        record.result = Some(match &result {
            Ok(()) => JournalResult::Completed,
            Err(error) => JournalResult::RecoveryRequired {
                reason: error.to_string(),
            },
        });
        self.journal.replace(record).await?;
        result
    }
}

#[async_trait]
impl<W: BranchWriter> BranchWriter for CoordinatedBranchWriter<W> {
    #[allow(clippy::result_large_err)]
    async fn apply(
        &self,
        target: SessionTarget,
        command: BranchCommand,
        context: ExecutionContext,
    ) -> GitResult<()> {
        let _lock = self
            .locks
            .acquire_with_cancellation(
                [ResourceKey {
                    scope: ResourceScope::CommonGitDir(target.repository()),
                    kind: ResourceKind::Refs,
                }],
                context.cancellation.clone(),
            )
            .await
            .map_err(|error| GitError::new(GitErrorKind::Cancelled, error.to_string()))?;
        self.writer.apply(target, command, context).await
    }
}

#[derive(Debug)]
pub struct ReconciledBranchWriter<W, R> {
    writer: W,
    rebuilder: R,
}

impl<W, R> ReconciledBranchWriter<W, R> {
    #[must_use]
    pub fn new(writer: W, rebuilder: R) -> Self {
        Self { writer, rebuilder }
    }

    #[allow(clippy::result_large_err)]
    pub async fn apply_and_rebuild(
        &self,
        target: SessionTarget,
        command: BranchCommand,
        context: ExecutionContext,
    ) -> GitResult<RepositorySnapshot>
    where
        W: BranchWriter,
        R: SnapshotRebuilder,
    {
        self.writer.apply(target, command, context.clone()).await?;
        self.rebuilder.rebuild(target, context).await
    }
}
