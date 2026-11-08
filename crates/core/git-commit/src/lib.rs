//! Candidate previews and commit transactions invoked by the operation coordinator.
//!
//! INFRA-001 skeleton only; behavior is implemented by subsequent tracked tasks.

mod editable_index;
mod reconciliation;

pub use editable_index::EditableIndexSession;
pub use reconciliation::{RealIndexReconciler, ReconciliationPlan};

use git_domain::{
    GitCommand, GitError, GitErrorKind, GitResult, JournalPhase, JournalRecord, JournalResult,
    ObjectId, OperationId, RecoveryScope, SelectionCommitRequest,
};
use git_ports::{CommitOutcome, CommitWriter, ExecutionContext, JournalStore};

/// Durable state of a partial-commit transaction. The states deliberately mirror the CCD
/// journal phases and never imply that a real commit is atomic with Index reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartialCommitPhase {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartialCommitTransaction {
    pub request: SelectionCommitRequest,
    pub expected_head: Option<ObjectId>,
    pub expected_index: Option<ObjectId>,
    pub candidate_tree: Option<ObjectId>,
    pub observed_commit: Option<ObjectId>,
    pub phase: PartialCommitPhase,
}

/// Journal coordinator for the irreversible boundary of a partial commit.
#[derive(Debug)]
pub struct PartialCommitJournal<J> {
    store: J,
    scope: RecoveryScope,
}

#[derive(Debug)]
pub struct PartialCommitCoordinator<W, J> {
    writer: W,
    journal: PartialCommitJournal<J>,
}

impl<W, J> PartialCommitCoordinator<W, J> {
    #[must_use]
    pub fn new(writer: W, journal: PartialCommitJournal<J>) -> Self {
        Self { writer, journal }
    }
}

impl<W: CommitWriter, J: JournalStore> PartialCommitCoordinator<W, J> {
    #[allow(clippy::result_large_err)]
    pub async fn commit(
        &self,
        target: git_domain::SessionTarget,
        operation: OperationId,
        transaction: &mut PartialCommitTransaction,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        let mut record = self.journal.record_intent(operation, transaction).await?;
        let _tree = transaction.begin_commit()?;
        record.phase = JournalPhase::CommitInProgress;
        self.journal.store.replace(record.clone()).await?;
        match self
            .writer
            .commit(target, transaction.standard_request()?, context)
            .await
        {
            Ok(outcome) => {
                transaction.commit_observed(outcome.commit)?;
                transaction.observe_head_after_commit(Some(outcome.commit))?;
                record.phase = JournalPhase::CommitObserved;
                record.observed_commit = Some(outcome.commit);
                record.observed_parents = outcome.parents.clone();
                record.result = Some(JournalResult::Completed);
                self.journal.store.replace(record).await?;
                Ok(outcome)
            }
            Err(error) => {
                record.phase = JournalPhase::RecoveryRequired;
                record.result = Some(JournalResult::RecoveryRequired {
                    reason: error.to_string(),
                });
                let _ = self.journal.store.replace(record).await;
                transaction.phase = PartialCommitPhase::RecoveryRequired;
                Err(error)
            }
        }
    }
}

impl<J> PartialCommitJournal<J> {
    #[must_use]
    pub fn new(store: J, scope: RecoveryScope) -> Self {
        Self { store, scope }
    }
}

impl<J: JournalStore> PartialCommitJournal<J> {
    #[allow(clippy::result_large_err)]
    pub async fn record_intent(
        &self,
        operation: OperationId,
        transaction: &PartialCommitTransaction,
    ) -> GitResult<JournalRecord> {
        let target = transaction.request.selections[0].target();
        let mut record = JournalRecord::intent(
            operation,
            GitCommand::CommitSelection {
                target,
                request: transaction.request.clone(),
            },
            self.scope.clone(),
        );
        record.expected_head = transaction.expected_head;
        record.expected_index = transaction.expected_index;
        record.candidate_tree = transaction.candidate_tree;
        record.phase = phase_to_journal(transaction.phase);
        self.store.append(record.clone()).await?;
        Ok(record)
    }

    #[allow(clippy::result_large_err)]
    pub async fn replace_phase(
        &self,
        mut record: JournalRecord,
        phase: PartialCommitPhase,
        result: Option<JournalResult>,
    ) -> GitResult<()> {
        record.phase = phase_to_journal(phase);
        record.result = result;
        self.store.replace(record).await
    }
}

fn phase_to_journal(phase: PartialCommitPhase) -> JournalPhase {
    match phase {
        PartialCommitPhase::IntentRecorded => JournalPhase::IntentRecorded,
        PartialCommitPhase::SnapshotCaptured => JournalPhase::SnapshotCaptured,
        PartialCommitPhase::TempIndexPrepared => JournalPhase::TempIndexPrepared,
        PartialCommitPhase::PreviewReady => JournalPhase::PreviewReady,
        PartialCommitPhase::PreflightRevalidated => JournalPhase::PreflightRevalidated,
        PartialCommitPhase::CommitInProgress => JournalPhase::CommitInProgress,
        PartialCommitPhase::CommitObserved => JournalPhase::CommitObserved,
        PartialCommitPhase::ReconcilingIndex => JournalPhase::ReconcilingIndex,
        PartialCommitPhase::Completed => JournalPhase::Completed,
        PartialCommitPhase::RecoveryRequired => JournalPhase::RecoveryRequired,
    }
}

impl PartialCommitTransaction {
    #[allow(clippy::result_large_err)]
    fn standard_request(&self) -> GitResult<git_domain::StandardCommitRequest> {
        Ok(git_domain::StandardCommitRequest {
            options: self.request.options.clone(),
            expected_revision: self.request.expected_revision,
            expected_head: self.expected_head,
            expected_index: self.expected_index,
        })
    }
    #[allow(clippy::result_large_err)]
    pub fn begin(request: SelectionCommitRequest) -> GitResult<Self> {
        if request.selections.is_empty() {
            return Err(GitError::new(
                GitErrorKind::InvalidPath,
                "partial commit requires at least one selection",
            ));
        }
        if request.selections.len() != request.fingerprints.len() {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "every partial commit selection requires one fingerprint",
            ));
        }
        let target = request.selections[0].target();
        if request
            .selections
            .iter()
            .any(|selection| selection.target() != target)
        {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "partial commit selections must share one session target",
            ));
        }
        if request
            .fingerprints
            .iter()
            .any(|fingerprint| fingerprint.expected_head != request.fingerprints[0].expected_head)
        {
            return Err(GitError::new(
                GitErrorKind::HeadChangedExternally,
                "partial commit fingerprints disagree about expected HEAD",
            ));
        }
        Ok(Self {
            request,
            expected_head: None,
            expected_index: None,
            candidate_tree: None,
            observed_commit: None,
            phase: PartialCommitPhase::IntentRecorded,
        })
    }

    #[allow(clippy::result_large_err)]
    pub fn temp_index_prepared(
        &mut self,
        expected_head: Option<ObjectId>,
        expected_index: Option<ObjectId>,
    ) -> GitResult<()> {
        if self.phase != PartialCommitPhase::IntentRecorded {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "temporary Index can only be prepared once",
            ));
        }
        if self.request.fingerprints.iter().any(|fingerprint| {
            fingerprint.expected_head != expected_head
                || fingerprint.expected_index != expected_index
        }) {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "selection identity changed before temporary Index preparation",
            ));
        }
        self.expected_head = expected_head;
        self.expected_index = expected_index;
        self.phase = PartialCommitPhase::TempIndexPrepared;
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    pub fn candidate_ready(&mut self, tree: ObjectId) -> GitResult<()> {
        if !matches!(
            self.phase,
            PartialCommitPhase::TempIndexPrepared | PartialCommitPhase::PreviewReady
        ) {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "candidate tree is not valid in the current transaction phase",
            ));
        }
        self.candidate_tree = Some(tree);
        self.phase = PartialCommitPhase::PreviewReady;
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    pub fn revalidate_before_commit(
        &mut self,
        head: Option<ObjectId>,
        index: Option<ObjectId>,
    ) -> GitResult<()> {
        if self.phase != PartialCommitPhase::PreviewReady {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "commit confirmation requires a ready preview",
            ));
        }
        if self.expected_head != head {
            self.phase = PartialCommitPhase::RecoveryRequired;
            return Err(GitError::new(
                GitErrorKind::HeadChangedExternally,
                "HEAD changed after partial commit preview",
            ));
        }
        if self.expected_index != index {
            self.phase = PartialCommitPhase::RecoveryRequired;
            return Err(GitError::new(
                GitErrorKind::IndexChangedExternally,
                "Index changed after partial commit preview",
            ));
        }
        self.phase = PartialCommitPhase::PreflightRevalidated;
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    pub fn begin_commit(&mut self) -> GitResult<ObjectId> {
        if self.phase != PartialCommitPhase::PreflightRevalidated {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "commit cannot start before preflight revalidation",
            ));
        }
        let tree = self
            .candidate_tree
            .ok_or_else(|| GitError::new(GitErrorKind::Internal, "commit has no candidate tree"))?;
        self.phase = PartialCommitPhase::CommitInProgress;
        Ok(tree)
    }

    #[allow(clippy::result_large_err)]
    pub fn commit_observed(&mut self, commit: ObjectId) -> GitResult<()> {
        if self.phase != PartialCommitPhase::CommitInProgress {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "observed commit is not expected in the current phase",
            ));
        }
        self.observed_commit = Some(commit);
        self.phase = PartialCommitPhase::CommitObserved;
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    pub fn observe_head_after_commit(&mut self, head: Option<ObjectId>) -> GitResult<()> {
        if self.phase != PartialCommitPhase::CommitObserved {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "HEAD observation is not valid before a commit result is observed",
            ));
        }
        let observed = self.observed_commit.ok_or_else(|| {
            GitError::new(
                GitErrorKind::Internal,
                "commit observation has no commit identity",
            )
        })?;
        if head != Some(observed) {
            self.phase = PartialCommitPhase::RecoveryRequired;
            return Err(GitError::new(
                GitErrorKind::HeadChangedExternally,
                "observed HEAD does not match the commit result",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{
        ChangeSelection, FileIdentity, FileKind, RepoPath, RepositoryId, SelectionFingerprint,
        SessionTarget, SnapshotRevision, WorktreeId,
    };

    fn request() -> SelectionCommitRequest {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let file = FileIdentity {
            target,
            path: RepoPath::try_from("file.txt").expect("path"),
            previous_path: None,
            kind: FileKind::Regular,
        };
        SelectionCommitRequest {
            options: git_domain::CommitOptions {
                message: b"partial".to_vec(),
                amend: false,
                allow_empty: false,
                signoff: false,
                author: None,
                committer: None,
                signing: Default::default(),
            },
            selections: vec![ChangeSelection::WholeFile { file }],
            fingerprints: vec![SelectionFingerprint {
                expected_head: None,
                expected_index: None,
                diff_generation: SnapshotRevision::new(1),
                content_digest: vec![1],
            }],
            expected_revision: SnapshotRevision::new(1),
        }
    }

    #[test]
    fn rejects_mismatched_selection_fingerprints() {
        let mut value = request();
        value.fingerprints.clear();
        assert_eq!(
            PartialCommitTransaction::begin(value).unwrap_err().kind,
            GitErrorKind::StaleSelection
        );
    }

    #[test]
    fn candidate_tree_is_only_accepted_after_temp_index_phase() {
        let mut transaction = PartialCommitTransaction::begin(request()).expect("valid request");
        let tree: ObjectId = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
            .parse()
            .expect("tree");
        assert_eq!(
            transaction.candidate_ready(tree).unwrap_err().kind,
            GitErrorKind::Internal
        );
    }

    #[test]
    fn stale_head_moves_transaction_to_recovery_required() {
        let mut transaction = PartialCommitTransaction::begin(request()).expect("valid request");
        transaction
            .temp_index_prepared(None, None)
            .expect("prepare");
        let tree: ObjectId = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
            .parse()
            .expect("tree");
        transaction.candidate_ready(tree).expect("candidate");
        let changed: ObjectId = "1111111111111111111111111111111111111111"
            .parse()
            .expect("head");
        assert_eq!(
            transaction
                .revalidate_before_commit(Some(changed), None)
                .unwrap_err()
                .kind,
            GitErrorKind::HeadChangedExternally
        );
        assert_eq!(transaction.phase, PartialCommitPhase::RecoveryRequired);
    }

    #[test]
    fn commit_result_requires_matching_observed_head() {
        let mut transaction = PartialCommitTransaction::begin(request()).expect("valid request");
        transaction
            .temp_index_prepared(None, None)
            .expect("prepare");
        let tree: ObjectId = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
            .parse()
            .expect("tree");
        transaction.candidate_ready(tree).expect("candidate");
        transaction
            .revalidate_before_commit(None, None)
            .expect("revalidate");
        transaction.begin_commit().expect("begin commit");
        let commit: ObjectId = "1111111111111111111111111111111111111111"
            .parse()
            .expect("commit");
        transaction.commit_observed(commit).expect("observe");
        assert!(transaction.observe_head_after_commit(Some(commit)).is_ok());
    }
}
