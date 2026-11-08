//! Minimal startup recovery inspection for journaled write operations.

use async_trait::async_trait;
use git_domain::{
    GitError, GitErrorKind, GitResult, JournalRecord, ObjectId, RecoveryAction, RecoveryScope,
    SequencerKind, SequencerSnapshot, SessionTarget,
};
use git_ports::{ExecutionContext, JournalStore};

/// Repository state observed while inspecting a pending journal record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryObservation {
    pub head: Option<ObjectId>,
    pub index: Option<ObjectId>,
    pub sequencer: Option<SequencerSnapshot>,
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
    pub current_target: SessionTarget,
    pub disposition: RecoveryDisposition,
}

/// Complete startup result, including legacy records whose repository identity is unprovable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupRecoveryReport {
    pub inspections: Vec<RecoveryInspection>,
    pub orphaned: Vec<JournalRecord>,
    pub sequencer: Option<SequencerSnapshot>,
}

impl StartupRecoveryReport {
    #[must_use]
    pub fn blocks_writes(&self) -> bool {
        self.sequencer.as_ref().is_some_and(|state| state.active)
            || !self.orphaned.is_empty()
            || self.inspections.iter().any(|inspection| {
                matches!(
                    inspection.disposition,
                    RecoveryDisposition::CommittedNeedsReconcile | RecoveryDisposition::Unknown
                )
            })
    }

    /// Returns the safe follow-up actions for an observed startup state.
    ///
    /// The returned actions are affordances only. Each action still requires a fresh preflight
    /// and must be executed through the operation coordinator. Revert is intentionally reduced to
    /// inspection because the current port does not expose a revert-specific continue/abort pair.
    #[must_use]
    pub fn recovery_actions(&self) -> Vec<RecoveryAction> {
        let Some(sequencer) = self.sequencer.as_ref().filter(|state| state.active) else {
            return vec![RecoveryAction::RefreshOnly];
        };

        match sequencer.kind {
            SequencerKind::Merge => {
                vec![RecoveryAction::ContinueMerge, RecoveryAction::AbortMerge]
            }
            SequencerKind::Rebase => vec![
                RecoveryAction::ContinueRebase,
                RecoveryAction::SkipRebase,
                RecoveryAction::AbortRebase,
            ],
            SequencerKind::CherryPick => vec![
                RecoveryAction::ContinueCherryPick,
                RecoveryAction::AbortCherryPick,
            ],
            SequencerKind::Revert => vec![RecoveryAction::InspectRepository],
        }
    }
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
        scope: &RecoveryScope,
        context: ExecutionContext,
    ) -> GitResult<StartupRecoveryReport> {
        let pending = self.journal.pending(scope).await?;
        if pending.scoped.is_empty() {
            return Ok(StartupRecoveryReport {
                inspections: Vec::new(),
                orphaned: pending.orphaned,
                sequencer: None,
            });
        }
        if pending
            .scoped
            .iter()
            .any(|record| record.recovery_scope.as_ref() != Some(scope) || record.is_legacy())
        {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "Journal returned a recovery record for another persistent scope",
            )
            .with_target(target)
            .with_operation(context.operation_id));
        }
        let observed = self.reader.read(target, context).await?;
        let inspections = pending
            .scoped
            .into_iter()
            .map(|record| {
                let disposition = classify(&record, &observed);
                RecoveryInspection {
                    record,
                    current_target: target,
                    disposition,
                }
            })
            .collect();
        Ok(StartupRecoveryReport {
            inspections,
            orphaned: pending.orphaned,
            sequencer: observed.sequencer,
        })
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
        GitCommand, JournalRecord, ObjectFormat, OperationId, RecoveryPath, RepositoryId,
        SequencerKind, SequencerSnapshot, SessionTarget, WorktreeId,
    };
    use git_ports::PendingJournalRecords;
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

        async fn pending(&self, scope: &RecoveryScope) -> GitResult<PendingJournalRecords> {
            let records = self.0.lock().expect("journal lock");
            Ok(PendingJournalRecords {
                scoped: records
                    .iter()
                    .filter(|item| {
                        item.recovery_scope.as_ref() == Some(scope) && item.needs_recovery()
                    })
                    .cloned()
                    .collect(),
                orphaned: records
                    .iter()
                    .filter(|item| item.is_legacy() && item.needs_recovery())
                    .cloned()
                    .collect(),
            })
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
            scope(),
        )
    }

    fn scope() -> RecoveryScope {
        RecoveryScope::worktree(
            RecoveryPath::unix(b"/repo/.git".to_vec()).expect("scope path"),
            RecoveryPath::unix(b"/repo/.git".to_vec()).expect("scope path"),
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
            scope(),
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
                    sequencer: None,
                },
            },
        );

        let report = inspector
            .inspect(target, &scope(), context())
            .await
            .expect("inspection should succeed");

        assert_eq!(report.inspections.len(), 2);
        assert_eq!(
            report.inspections[0].disposition,
            RecoveryDisposition::NotExecuted
        );
        assert_eq!(
            report.inspections[1].disposition,
            RecoveryDisposition::Unknown
        );
        assert_eq!(report.inspections[0].current_target, target);
        assert!(report.blocks_writes());
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
                    sequencer: None,
                },
            },
        );

        let report = inspector
            .inspect(target, &scope(), context())
            .await
            .expect("empty inspection should succeed");
        assert!(report.inspections.is_empty());
        assert!(report.orphaned.is_empty());
        assert!(!report.blocks_writes());
        assert!(calls.lock().expect("reader lock").is_empty());
    }

    #[test]
    fn active_sequencer_blocks_startup_writes() {
        let report = StartupRecoveryReport {
            inspections: Vec::new(),
            orphaned: Vec::new(),
            sequencer: Some(SequencerSnapshot {
                kind: SequencerKind::CherryPick,
                active: true,
                current: None,
                total: None,
                conflicts: Vec::new(),
            }),
        };
        assert!(report.blocks_writes());
    }

    #[test]
    fn recovery_actions_follow_active_sequencer_kind() {
        let cases = [
            (
                SequencerKind::Merge,
                vec![RecoveryAction::ContinueMerge, RecoveryAction::AbortMerge],
            ),
            (
                SequencerKind::Rebase,
                vec![
                    RecoveryAction::ContinueRebase,
                    RecoveryAction::SkipRebase,
                    RecoveryAction::AbortRebase,
                ],
            ),
            (
                SequencerKind::CherryPick,
                vec![
                    RecoveryAction::ContinueCherryPick,
                    RecoveryAction::AbortCherryPick,
                ],
            ),
            (
                SequencerKind::Revert,
                vec![RecoveryAction::InspectRepository],
            ),
        ];

        for (kind, expected) in cases {
            let report = StartupRecoveryReport {
                inspections: Vec::new(),
                orphaned: Vec::new(),
                sequencer: Some(SequencerSnapshot {
                    kind,
                    active: true,
                    current: None,
                    total: None,
                    conflicts: Vec::new(),
                }),
            };
            assert_eq!(report.recovery_actions(), expected);
        }
    }

    #[test]
    fn inactive_or_missing_sequencer_only_allows_refresh() {
        let report = StartupRecoveryReport::default();
        assert_eq!(report.recovery_actions(), vec![RecoveryAction::RefreshOnly]);
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
                    index: None,
                    sequencer: None,
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
                    index: Some(index),
                    sequencer: None,
                }
            ),
            RecoveryDisposition::Committed
        );
        assert_eq!(
            classify(
                &record,
                &RecoveryObservation {
                    head: Some(commit),
                    index: None,
                    sequencer: None,
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
                    index: None,
                    sequencer: None,
                }
            ),
            RecoveryDisposition::Unknown
        );
    }
}
