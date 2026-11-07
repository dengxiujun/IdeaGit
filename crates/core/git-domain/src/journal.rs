//! Durable operation-journal values shared by coordinators and storage adapters.

use serde::{Deserialize, Serialize};

use crate::{GitCommand, ObjectId, OperationId, SessionTarget};

/// Current journal schema written to durable storage.
pub const JOURNAL_SCHEMA_VERSION: u32 = 1;

/// Durable sub-phase for recovery; intentionally separate from public operation lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JournalPhase {
    IntentRecorded,
    SnapshotCaptured,
    TempIndexPrepared,
    PreviewReady,
    PreflightRevalidated,
    CommitInProgress,
    CommitObserved,
    ReconcilingIndex,
    Completed,
    RecoveryRequired,
}

/// Terminal recovery information persisted after an uncertain or failed operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JournalResult {
    Completed,
    RecoveryRequired { reason: String },
}

/// One durable operation intent and its observed progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRecord {
    pub schema_version: u32,
    pub operation_id: OperationId,
    pub target: SessionTarget,
    pub command: GitCommand,
    pub expected_head: Option<ObjectId>,
    pub expected_index: Option<ObjectId>,
    pub index_backup: Option<Vec<u8>>,
    pub candidate_tree: Option<ObjectId>,
    pub observed_commit: Option<ObjectId>,
    pub observed_parents: Vec<ObjectId>,
    pub phase: JournalPhase,
    pub result: Option<JournalResult>,
}

impl JournalRecord {
    /// Creates a new intent record. It is not complete until storage confirms the write.
    #[must_use]
    pub fn intent(operation_id: OperationId, command: GitCommand) -> Self {
        Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            target: command.target(),
            operation_id,
            command,
            expected_head: None,
            expected_index: None,
            index_backup: None,
            candidate_tree: None,
            observed_commit: None,
            observed_parents: Vec::new(),
            phase: JournalPhase::IntentRecorded,
            result: None,
        }
    }

    /// Returns whether the record still needs startup recovery inspection.
    #[must_use]
    pub const fn needs_recovery(&self) -> bool {
        !matches!(self.phase, JournalPhase::Completed) || self.result.is_none()
    }
}
