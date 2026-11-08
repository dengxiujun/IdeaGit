//! Operation-level serialization for Index mutations.

use async_trait::async_trait;
use git_domain::{
    GitCommand, GitError, GitErrorKind, GitResult, JournalPhase, JournalRecord, JournalResult,
    RecoveryScope, RepositorySnapshot, SessionTarget,
};
use git_ports::{ExecutionContext, IndexMutation, IndexWriter, JournalStore};

use crate::{RecoveryStateReader, ResourceKey, ResourceKind, ResourceLockManager, ResourceScope};

/// Index writer decorator that acquires the concrete Worktree Index lock first.
#[derive(Debug)]
pub struct CoordinatedIndexWriter<W> {
    inner: W,
    locks: ResourceLockManager,
}

/// Read-after-write boundary used to publish authoritative state after Index mutation.
#[async_trait]
pub trait SnapshotRebuilder: Send + Sync {
    async fn rebuild(
        &self,
        target: SessionTarget,
        context: ExecutionContext,
    ) -> GitResult<RepositorySnapshot>;
}

/// Composes a journaled/locked Index writer with an authoritative post-write rebuild.
#[derive(Debug)]
pub struct ReconciledIndexWriter<W, R> {
    writer: W,
    rebuilder: R,
}

impl<W, R> ReconciledIndexWriter<W, R> {
    #[must_use]
    pub fn new(writer: W, rebuilder: R) -> Self {
        Self { writer, rebuilder }
    }

    #[allow(clippy::result_large_err)]
    pub async fn apply_and_rebuild(
        &self,
        mutation: IndexMutation,
        context: ExecutionContext,
    ) -> GitResult<RepositorySnapshot>
    where
        W: IndexWriter,
        R: SnapshotRebuilder,
    {
        let target = mutation.target();
        self.writer.apply(mutation, context.clone()).await?;
        self.rebuilder.rebuild(target, context).await
    }
}

#[async_trait]
impl<W: IndexWriter, R: SnapshotRebuilder> IndexWriter for ReconciledIndexWriter<W, R> {
    #[allow(clippy::result_large_err)]
    async fn apply(&self, mutation: IndexMutation, context: ExecutionContext) -> GitResult<()> {
        self.apply_and_rebuild(mutation, context).await.map(|_| ())
    }
}

/// Journal decorator that records Index intent before execution and terminal outcome afterwards.
#[derive(Debug)]
pub struct JournaledIndexWriter<W, J, R> {
    inner: W,
    journal: J,
    recovery_scope: RecoveryScope,
    state_reader: R,
}

impl<W, J, R> JournaledIndexWriter<W, J, R> {
    #[must_use]
    pub fn new(inner: W, journal: J, recovery_scope: RecoveryScope, state_reader: R) -> Self {
        Self {
            inner,
            journal,
            recovery_scope,
            state_reader,
        }
    }
}

#[async_trait]
impl<W: IndexWriter, J: JournalStore, R: RecoveryStateReader> IndexWriter
    for JournaledIndexWriter<W, J, R>
{
    #[allow(clippy::result_large_err)]
    async fn apply(&self, mutation: IndexMutation, context: ExecutionContext) -> GitResult<()> {
        mutation
            .validate()
            .map_err(|error| GitError::new(GitErrorKind::InvalidPath, error.to_string()))?;
        let command = match &mutation {
            IndexMutation::Stage { target, paths } => GitCommand::StageFiles {
                target: *target,
                paths: paths.clone(),
            },
            IndexMutation::Unstage { target, paths } => GitCommand::UnstageFiles {
                target: *target,
                paths: paths.clone(),
            },
        };
        let mut record =
            JournalRecord::intent(context.operation_id, command, self.recovery_scope.clone());
        let state = self
            .state_reader
            .read(mutation.target(), context.clone())
            .await?;
        record.expected_head = state.head;
        record.expected_index = state.index;
        self.journal.append(record.clone()).await?;
        let result = self.inner.apply(mutation, context).await;
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

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use git_domain::{
        GitResult, OperationId, RecoveryPath, RepoPath, RepositoryId, SessionTarget, WorktreeId,
    };
    use git_ports::{JournalStore, PendingJournalRecords};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[derive(Clone, Default)]
    struct MemoryJournal(Arc<Mutex<Vec<JournalRecord>>>);

    #[async_trait]
    impl JournalStore for MemoryJournal {
        async fn append(&self, record: JournalRecord) -> GitResult<()> {
            self.0.lock().expect("journal lock").push(record);
            Ok(())
        }
        async fn replace(&self, record: JournalRecord) -> GitResult<()> {
            let mut records = self.0.lock().expect("journal lock");
            let slot = records
                .iter_mut()
                .find(|item| item.operation_id == record.operation_id)
                .expect("intent");
            *slot = record;
            Ok(())
        }
        async fn get(&self, id: OperationId) -> GitResult<Option<JournalRecord>> {
            Ok(self
                .0
                .lock()
                .expect("journal lock")
                .iter()
                .find(|item| item.operation_id == id)
                .cloned())
        }
        async fn pending(&self, scope: &RecoveryScope) -> GitResult<PendingJournalRecords> {
            Ok(PendingJournalRecords {
                scoped: self
                    .0
                    .lock()
                    .expect("journal lock")
                    .iter()
                    .filter(|item| item.recovery_scope.as_ref() == Some(scope))
                    .cloned()
                    .collect(),
                orphaned: Vec::new(),
            })
        }
    }

    struct SuccessWriter;
    #[async_trait]
    impl IndexWriter for SuccessWriter {
        async fn apply(
            &self,
            _mutation: IndexMutation,
            _context: ExecutionContext,
        ) -> GitResult<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FixedStateReader {
        state: crate::RecoveryObservation,
    }

    #[async_trait]
    impl RecoveryStateReader for FixedStateReader {
        async fn read(
            &self,
            _target: SessionTarget,
            _context: ExecutionContext,
        ) -> GitResult<crate::RecoveryObservation> {
            Ok(self.state.clone())
        }
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel(1);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    #[tokio::test]
    async fn journal_records_completed_index_operation() {
        let journal = MemoryJournal::default();
        let scope = RecoveryScope::worktree(
            RecoveryPath::unix(b"/repo/.git".to_vec()).expect("scope path"),
            RecoveryPath::unix(b"/repo/.git".to_vec()).expect("scope path"),
        );
        let expected_head =
            git_domain::ObjectId::from_bytes(git_domain::ObjectFormat::Sha1, &[1; 20])
                .expect("head OID");
        let expected_index =
            git_domain::ObjectId::from_bytes(git_domain::ObjectFormat::Sha1, &[2; 20])
                .expect("Index OID");
        let writer = JournaledIndexWriter::new(
            SuccessWriter,
            journal.clone(),
            scope.clone(),
            FixedStateReader {
                state: crate::RecoveryObservation {
                    head: Some(expected_head),
                    index: Some(expected_index),
                    sequencer: None,
                },
            },
        );
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        writer
            .apply(
                IndexMutation::Stage {
                    target,
                    paths: vec![RepoPath::try_from("a.txt").expect("path")],
                },
                context(),
            )
            .await
            .expect("operation");
        let records = journal.0.lock().expect("journal lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].phase, JournalPhase::Completed);
        assert_eq!(records[0].result, Some(JournalResult::Completed));
        assert_eq!(records[0].recovery_scope, Some(scope));
        assert_eq!(records[0].expected_head, Some(expected_head));
        assert_eq!(records[0].expected_index, Some(expected_index));
    }
}

impl<W> CoordinatedIndexWriter<W> {
    #[must_use]
    pub fn new(inner: W, locks: ResourceLockManager) -> Self {
        Self { inner, locks }
    }
}

#[async_trait]
impl<W: IndexWriter> IndexWriter for CoordinatedIndexWriter<W> {
    #[allow(clippy::result_large_err)]
    async fn apply(&self, mutation: IndexMutation, context: ExecutionContext) -> GitResult<()> {
        let worktree = mutation.target().worktree_id().ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "Index mutation is unavailable for a bare repository",
            )
        })?;
        let _lock = self
            .locks
            .acquire_with_cancellation(
                [ResourceKey {
                    scope: ResourceScope::Worktree(worktree),
                    kind: ResourceKind::Index,
                }],
                context.cancellation.clone(),
            )
            .await
            .map_err(|error| GitError::new(GitErrorKind::Cancelled, error.to_string()))?;
        self.inner.apply(mutation, context).await
    }
}
