//! Minimal startup recovery inspection for journaled write operations.

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult, JournalRecord, ObjectId, SessionTarget};
use git_ports::{ExecutionContext, JournalStore};

/// Repository state observed while inspecting a pending journal record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryObservation {
    pub head: Option<ObjectId>,
    pub index: Option<ObjectId>,
}

/// Safe classification of a pending operation. Classification never mutates Git state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryDisposition {
    /// The recorded intent has not produced an observable side effect.
    NotExecuted,
    /// The expected commit is now HEAD and the Index matches the recorded expectation.
    Committed,
    /// The expected commit is now HEAD, but the Index needs reconciliation.
    CommittedNeedsReconcile,
    /// Observed state cannot be proven to be either the preflight or committed state.
    Unknown,
}

/// Reads the authoritative repository state used by startup recovery.
#[async_trait]
pub trait RecoveryStateReader: Send + Sync {
    async fn read(
        &self,
        target: SessionTarget,
        context: ExecutionContext,
    ) -> GitResult<RecoveryObservation>;
}

/// One pending Journal record and its safe startup disposition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryInspection {
    pub record: JournalRecord,
    pub disposition: RecoveryDisposition,
}

/// Read-only startup recovery service for one repository session.
#[derive(Debug)]
pub struct StartupRecoveryInspector<J, R> {
    journal: J,
    reader: R,
}

impl<J, R> StartupRecoveryInspector<J, R> {
    #[must_use]
    pub fn new(journal: J, reader: R) -> Self {
        Self { journal, reader }
    }
}

impl<J: JournalStore, R: RecoveryStateReader> StartupRecoveryInspector<J, R> {
    /// Inspects all pending records for `target` without mutating Git or retrying any command.
    #[allow(clippy::result_large_err)]
    pub async fn inspect(
        &self,
        target: SessionTarget,
        context: ExecutionContext,
    ) -> GitResult<Vec<RecoveryInspection>> {
        let records = self.journal.pending(target).await?;
        if records.is_empty() {
            return Ok(Vec::new());
        }
        if records.iter().any(|record| record.target != target) {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "Journal returned a recovery record for another repository session",
            )
            .with_target(target)
            .with_operation(context.operation_id));
        }
        let observed = self.reader.read(target, context).await?;
        Ok(records
            .into_iter()
            .map(|record| {
                let disposition = classify(&record, &observed);
                RecoveryInspection {
                    record,
                    disposition,
                }
            })
            .collect())
    }
}

/// Classifies one pending journal record against a fresh repository observation.
#[must_use]
pub fn classify(record: &JournalRecord, observed: &RecoveryObservation) -> RecoveryDisposition {
    if let Some(commit) = record.observed_commit {
        if observed.head != Some(commit) {
            return RecoveryDisposition::Unknown;
        }
        return if observed.index == record.expected_index {
            RecoveryDisposition::Committed
        } else {
            RecoveryDisposition::CommittedNeedsReconcile
        };
    }

    if observed.head == record.expected_head && observed.index == record.expected_index {
        RecoveryDisposition::NotExecuted
    } else {
        RecoveryDisposition::Unknown
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use git_domain::{
        GitCommand, JournalRecord, ObjectFormat, OperationId, RepositoryId, SessionTarget,
        WorktreeId,
    };
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

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
            let Some(slot) = records
                .iter_mut()
                .find(|item| item.operation_id == record.operation_id)
            else {
                return Err(GitError::new(GitErrorKind::Internal, "missing record"));
            };
            *slot = record;
            Ok(())
        }

        async fn get(&self, operation_id: OperationId) -> GitResult<Option<JournalRecord>> {
            Ok(self
                .0
                .lock()
                .expect("journal lock")
                .iter()
                .find(|item| item.operation_id == operation_id)
                .cloned())
        }

        async fn pending(&self, target: SessionTarget) -> GitResult<Vec<JournalRecord>> {
            Ok(self
                .0
                .lock()
                .expect("journal lock")
                .iter()
                .filter(|item| item.target == target && item.needs_recovery())
                .cloned()
                .collect())
        }
    }

    #[derive(Clone)]
    struct RecordingReader {
        calls: Arc<Mutex<Vec<SessionTarget>>>,
        observed: RecoveryObservation,
    }

    #[async_trait]
    impl RecoveryStateReader for RecordingReader {
        async fn read(
            &self,
            target: SessionTarget,
            _context: ExecutionContext,
        ) -> GitResult<RecoveryObservation> {
            self.calls.lock().expect("reader lock").push(target);
            Ok(self.observed.clone())
        }
    }

    fn record() -> JournalRecord {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        JournalRecord::intent(
            OperationId::new(),
            GitCommand::StageFiles {
                target,
                paths: Vec::new(),
            },
        )
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel(1);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    #[tokio::test]
    async fn startup_inspector_reads_state_once_and_classifies_every_pending_record() {
        let mut first = record();
        let target = first.target;
        let head =
            git_domain::ObjectId::from_bytes(ObjectFormat::Sha1, &[6; 20]).expect("valid OID");
        first.expected_head = Some(head);
        let mut second = JournalRecord::intent(
            OperationId::new(),
            GitCommand::StageFiles {
                target,
                paths: Vec::new(),
            },
        );
        second.expected_head = Some(
            git_domain::ObjectId::from_bytes(ObjectFormat::Sha1, &[7; 20]).expect("valid OID"),
        );
        let journal = MemoryJournal(Arc::new(Mutex::new(vec![first, second])));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let inspector = StartupRecoveryInspector::new(
            journal,
            RecordingReader {
                calls: Arc::clone(&calls),
                observed: RecoveryObservation {
                    head: Some(head),
                    index: None,
                },
            },
        );

        let inspections = inspector
            .inspect(target, context())
            .await
            .expect("inspection should succeed");

        assert_eq!(inspections.len(), 2);
        assert_eq!(inspections[0].disposition, RecoveryDisposition::NotExecuted);
        assert_eq!(inspections[1].disposition, RecoveryDisposition::Unknown);
        assert_eq!(*calls.lock().expect("reader lock"), vec![target]);
    }

    #[tokio::test]
    async fn startup_inspector_does_not_read_repository_without_pending_records() {
        let target = record().target;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let inspector = StartupRecoveryInspector::new(
            MemoryJournal::default(),
            RecordingReader {
                calls: Arc::clone(&calls),
                observed: RecoveryObservation {
                    head: None,
                    index: None,
                },
            },
        );

        assert!(
            inspector
                .inspect(target, context())
                .await
                .expect("empty inspection should succeed")
                .is_empty()
        );
        assert!(calls.lock().expect("reader lock").is_empty());
    }

    #[test]
    fn unchanged_state_is_not_executed() {
        let mut record = record();
        let head =
            git_domain::ObjectId::from_bytes(ObjectFormat::Sha1, &[1; 20]).expect("valid OID");
        record.expected_head = Some(head);
        record.expected_index = None;
        assert_eq!(
            classify(
                &record,
                &RecoveryObservation {
                    head: Some(head),
                    index: None
                }
            ),
            RecoveryDisposition::NotExecuted
        );
    }

    #[test]
    fn observed_commit_requires_matching_head_and_index() {
        let mut record = record();
        let commit =
            git_domain::ObjectId::from_bytes(ObjectFormat::Sha1, &[2; 20]).expect("valid OID");
        let index =
            git_domain::ObjectId::from_bytes(ObjectFormat::Sha1, &[3; 20]).expect("valid OID");
        record.observed_commit = Some(commit);
        record.expected_index = Some(index);
        assert_eq!(
            classify(
                &record,
                &RecoveryObservation {
                    head: Some(commit),
                    index: Some(index)
                }
            ),
            RecoveryDisposition::Committed
        );
        assert_eq!(
            classify(
                &record,
                &RecoveryObservation {
                    head: Some(commit),
                    index: None
                }
            ),
            RecoveryDisposition::CommittedNeedsReconcile
        );
    }

    #[test]
    fn conflicting_state_is_unknown() {
        let mut record = record();
        let commit =
            git_domain::ObjectId::from_bytes(ObjectFormat::Sha1, &[4; 20]).expect("valid OID");
        record.observed_commit = Some(commit);
        let other =
            git_domain::ObjectId::from_bytes(ObjectFormat::Sha1, &[5; 20]).expect("valid OID");
        assert_eq!(
            classify(
                &record,
                &RecoveryObservation {
                    head: Some(other),
                    index: None
                }
            ),
            RecoveryDisposition::Unknown
        );
    }
}
