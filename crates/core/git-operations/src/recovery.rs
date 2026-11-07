//! Minimal startup recovery classification for journaled write operations.

use git_domain::{JournalRecord, ObjectId};

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
    use super::*;
    use git_domain::{
        GitCommand, JournalRecord, ObjectFormat, OperationId, RepositoryId, SessionTarget,
        WorktreeId,
    };

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
