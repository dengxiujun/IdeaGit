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
use git_ports::{
    CommitOutcome, CommitSideEffectObserver, ExecutionContext, IndexReconciliationOutcome,
    IndexReconciliationPublisher, IndexReconciliationRequest, JournalStore, PartialCommitWriter,
    TemporaryIndexLease,
};

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
    pub original_index_digest: Option<[u8; 32]>,
    pub candidate_index: Option<Vec<u8>>,
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
pub struct PartialCommitCoordinator<W, J, O, P> {
    writer: W,
    journal: PartialCommitJournal<J>,
    observer: O,
    publisher: P,
}

impl<W, J, O, P> PartialCommitCoordinator<W, J, O, P> {
    #[must_use]
    pub fn new(writer: W, journal: PartialCommitJournal<J>, observer: O, publisher: P) -> Self {
        Self {
            writer,
            journal,
            observer,
            publisher,
        }
    }
}

impl<
    W: PartialCommitWriter,
    J: JournalStore,
    O: CommitSideEffectObserver,
    P: IndexReconciliationPublisher,
> PartialCommitCoordinator<W, J, O, P>
{
    #[allow(clippy::result_large_err)]
    pub async fn commit(
        &self,
        target: git_domain::SessionTarget,
        operation: OperationId,
        transaction: &mut PartialCommitTransaction,
        temporary_index: &TemporaryIndexLease,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        let mut record = self.journal.record_intent(operation, transaction).await?;
        let before = self.observer.capture(target, context.clone()).await?;
        let _tree = transaction.begin_commit()?;
        record.phase = JournalPhase::CommitInProgress;
        self.journal.store.replace(record.clone()).await?;
        match self
            .writer
            .commit(
                target,
                operation,
                transaction.standard_request()?,
                temporary_index,
                context.clone(),
            )
            .await
        {
            Ok(outcome) => {
                let after = self.observer.observe(target, context.clone()).await?;
                if before.worktree_digest != after.worktree_digest
                    || before.refs_digest != after.refs_digest
                {
                    transaction.phase = PartialCommitPhase::RecoveryRequired;
                    record.phase = JournalPhase::RecoveryRequired;
                    record.result = Some(JournalResult::RecoveryRequired {
                        reason: "commit side effects changed observed Worktree or Refs".to_owned(),
                    });
                    let _ = self.journal.store.replace(record).await;
                    return Err(GitError::new(
                        GitErrorKind::HooksMutatedRepository,
                        "commit side effects require recovery",
                    ));
                }
                transaction.commit_observed(outcome.commit)?;
                transaction.observe_head_after_commit(Some(outcome.commit))?;
                record.phase = JournalPhase::ReconcilingIndex;
                record.observed_commit = Some(outcome.commit);
                record.observed_parents = outcome.parents.clone();
                self.journal.store.replace(record.clone()).await?;
                let (Some(expected_index_digest), Some(candidate_index)) = (
                    transaction.original_index_digest,
                    transaction.candidate_index.clone(),
                ) else {
                    return self
                        .recovery_required(
                            transaction,
                            record,
                            "partial commit has no verified candidate Index",
                        )
                        .await;
                };
                let reconciliation = self
                    .publisher
                    .publish(
                        IndexReconciliationRequest {
                            target,
                            expected_index_digest,
                            candidate_index,
                        },
                        context.clone(),
                    )
                    .await;
                let reconciliation = match reconciliation {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        transaction.phase = PartialCommitPhase::RecoveryRequired;
                        record.phase = JournalPhase::RecoveryRequired;
                        record.result = Some(JournalResult::RecoveryRequired {
                            reason: error.to_string(),
                        });
                        let _ = self.journal.store.replace(record).await;
                        return Err(error);
                    }
                };
                match reconciliation {
                    IndexReconciliationOutcome::Published => {
                        transaction.phase = PartialCommitPhase::Completed;
                        record.phase = JournalPhase::Completed;
                        record.result = Some(JournalResult::Completed);
                        self.journal.store.replace(record).await?;
                        Ok(outcome)
                    }
                    IndexReconciliationOutcome::Conflict => {
                        self.recovery_required(
                            transaction,
                            record,
                            "real Index changed before reconciliation",
                        )
                        .await
                    }
                    IndexReconciliationOutcome::Unknown => {
                        self.recovery_required(
                            transaction,
                            record,
                            "real Index reconciliation result is unknown",
                        )
                        .await
                    }
                }
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

    #[allow(clippy::result_large_err)]
    async fn recovery_required(
        &self,
        transaction: &mut PartialCommitTransaction,
        mut record: JournalRecord,
        reason: &'static str,
    ) -> GitResult<CommitOutcome> {
        transaction.phase = PartialCommitPhase::RecoveryRequired;
        record.phase = JournalPhase::RecoveryRequired;
        record.result = Some(JournalResult::RecoveryRequired {
            reason: reason.to_owned(),
        });
        self.journal.store.replace(record).await?;
        Err(GitError::new(GitErrorKind::IndexChangedExternally, reason))
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
            original_index_digest: None,
            candidate_index: None,
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
    pub fn reconciliation_candidate(
        &mut self,
        original_index_digest: [u8; 32],
        candidate_index: Vec<u8>,
    ) -> GitResult<()> {
        if !matches!(
            self.phase,
            PartialCommitPhase::TempIndexPrepared | PartialCommitPhase::PreviewReady
        ) {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "Index reconciliation candidate is not valid in the current transaction phase",
            ));
        }
        self.original_index_digest = Some(original_index_digest);
        self.candidate_index = Some(candidate_index);
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
    use async_trait::async_trait;
    use git_domain::{
        ChangeSelection, FileIdentity, FileKind, RecoveryPath, RepoPath, RepositoryId,
        SelectionFingerprint, SessionTarget, SnapshotRevision, WorktreeId,
    };
    use git_ports::{
        CommitSideEffectSnapshot, IndexReconciliationOutcome, IndexReconciliationPublisher,
        IndexReconciliationRequest, PendingJournalRecords, ProcessProgress,
    };
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    #[derive(Clone, Default)]
    struct TestJournalStore {
        record: Arc<Mutex<Option<JournalRecord>>>,
    }

    #[async_trait]
    impl JournalStore for TestJournalStore {
        async fn append(&self, record: JournalRecord) -> GitResult<()> {
            *self.record.lock().expect("journal lock") = Some(record);
            Ok(())
        }

        async fn replace(&self, record: JournalRecord) -> GitResult<()> {
            *self.record.lock().expect("journal lock") = Some(record);
            Ok(())
        }

        async fn get(&self, _operation_id: OperationId) -> GitResult<Option<JournalRecord>> {
            Ok(self.record.lock().expect("journal lock").clone())
        }

        async fn pending(&self, _scope: &RecoveryScope) -> GitResult<PendingJournalRecords> {
            Ok(PendingJournalRecords::default())
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct TestWriter;

    #[async_trait]
    impl PartialCommitWriter for TestWriter {
        async fn commit(
            &self,
            _target: SessionTarget,
            _operation: OperationId,
            _request: git_domain::StandardCommitRequest,
            _temporary_index: &TemporaryIndexLease,
            _context: ExecutionContext,
        ) -> GitResult<CommitOutcome> {
            Ok(CommitOutcome {
                commit: "1111111111111111111111111111111111111111"
                    .parse()
                    .expect("commit oid"),
                parents: Vec::new(),
            })
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct MutatingObserver;

    #[async_trait]
    impl CommitSideEffectObserver for MutatingObserver {
        async fn capture(
            &self,
            _target: SessionTarget,
            _context: ExecutionContext,
        ) -> GitResult<CommitSideEffectSnapshot> {
            Ok(CommitSideEffectSnapshot {
                head: None,
                worktree_digest: [1; 32],
                refs_digest: [2; 32],
            })
        }

        async fn observe(
            &self,
            _target: SessionTarget,
            _context: ExecutionContext,
        ) -> GitResult<CommitSideEffectSnapshot> {
            Ok(CommitSideEffectSnapshot {
                head: None,
                worktree_digest: [3; 32],
                refs_digest: [2; 32],
            })
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct StableObserver;

    #[async_trait]
    impl CommitSideEffectObserver for StableObserver {
        async fn capture(
            &self,
            _target: SessionTarget,
            _context: ExecutionContext,
        ) -> GitResult<CommitSideEffectSnapshot> {
            Ok(CommitSideEffectSnapshot {
                head: None,
                worktree_digest: [1; 32],
                refs_digest: [2; 32],
            })
        }

        async fn observe(
            &self,
            _target: SessionTarget,
            _context: ExecutionContext,
        ) -> GitResult<CommitSideEffectSnapshot> {
            self.capture(_target, _context).await
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct TestPublisher;

    #[async_trait]
    impl IndexReconciliationPublisher for TestPublisher {
        async fn publish(
            &self,
            _request: IndexReconciliationRequest,
            _context: ExecutionContext,
        ) -> GitResult<IndexReconciliationOutcome> {
            Ok(IndexReconciliationOutcome::Published)
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct FailingPublisher;

    #[async_trait]
    impl IndexReconciliationPublisher for FailingPublisher {
        async fn publish(
            &self,
            _request: IndexReconciliationRequest,
            _context: ExecutionContext,
        ) -> GitResult<IndexReconciliationOutcome> {
            Err(GitError::new(
                GitErrorKind::PermissionDenied,
                "simulated Index publication failure",
            ))
        }
    }

    fn test_context(operation: OperationId) -> ExecutionContext {
        let (progress, _receiver) = tokio::sync::mpsc::channel::<ProcessProgress>(4);
        ExecutionContext::new(operation, CancellationToken::new(), progress)
    }

    fn test_temporary_index(operation: OperationId) -> TemporaryIndexLease {
        TemporaryIndexLease {
            path: std::path::PathBuf::from("/tmp/ideagit-partial-commit-test-index"),
            transaction_id: operation,
        }
    }

    #[tokio::test]
    async fn coordinator_persists_recovery_when_observer_detects_side_effect() {
        let store = TestJournalStore::default();
        let scope = RecoveryScope::worktree(
            RecoveryPath::unix(b"/tmp/ideagit-common".to_vec()).expect("common path"),
            RecoveryPath::unix(b"/tmp/ideagit-git".to_vec()).expect("git path"),
        );
        let journal = PartialCommitJournal::new(store.clone(), scope);
        let coordinator =
            PartialCommitCoordinator::new(TestWriter, journal, MutatingObserver, TestPublisher);
        let mut transaction = PartialCommitTransaction::begin(request()).expect("transaction");
        transaction
            .temp_index_prepared(None, None)
            .expect("temporary Index");
        transaction
            .candidate_ready(
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
                    .parse()
                    .expect("tree"),
            )
            .expect("candidate");
        transaction
            .revalidate_before_commit(None, None)
            .expect("revalidation");
        let target = transaction.request.selections[0].target();

        let operation = OperationId::new();
        let temporary_index = test_temporary_index(operation);
        let error = coordinator
            .commit(
                target,
                operation,
                &mut transaction,
                &temporary_index,
                test_context(operation),
            )
            .await
            .expect_err("side effects must not report success");

        assert_eq!(error.kind, GitErrorKind::HooksMutatedRepository);
        assert_eq!(transaction.phase, PartialCommitPhase::RecoveryRequired);
        let record = store
            .record
            .lock()
            .expect("journal lock")
            .clone()
            .expect("journal record");
        assert_eq!(record.phase, JournalPhase::RecoveryRequired);
        assert!(matches!(
            record.result,
            Some(JournalResult::RecoveryRequired { .. })
        ));
    }

    #[tokio::test]
    async fn coordinator_completes_only_after_index_reconciliation_publishes() {
        let store = TestJournalStore::default();
        let scope = RecoveryScope::worktree(
            RecoveryPath::unix(b"/tmp/ideagit-common".to_vec()).expect("common path"),
            RecoveryPath::unix(b"/tmp/ideagit-git".to_vec()).expect("git path"),
        );
        let journal = PartialCommitJournal::new(store.clone(), scope);
        let coordinator =
            PartialCommitCoordinator::new(TestWriter, journal, StableObserver, TestPublisher);
        let mut transaction = PartialCommitTransaction::begin(request()).expect("transaction");
        transaction
            .temp_index_prepared(None, None)
            .expect("temporary Index");
        transaction
            .candidate_ready(
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
                    .parse()
                    .expect("tree"),
            )
            .expect("candidate");
        transaction
            .reconciliation_candidate([7; 32], b"verified-index".to_vec())
            .expect("reconciliation candidate");
        transaction
            .revalidate_before_commit(None, None)
            .expect("revalidation");
        let target = transaction.request.selections[0].target();
        let operation = OperationId::new();
        let temporary_index = test_temporary_index(operation);
        let outcome = coordinator
            .commit(
                target,
                operation,
                &mut transaction,
                &temporary_index,
                test_context(operation),
            )
            .await
            .expect("reconciled commit");

        assert_eq!(
            outcome.commit.to_string(),
            "1111111111111111111111111111111111111111"
        );
        assert_eq!(transaction.phase, PartialCommitPhase::Completed);
        let record = store
            .record
            .lock()
            .expect("journal lock")
            .clone()
            .expect("journal record");
        assert_eq!(record.phase, JournalPhase::Completed);
        assert_eq!(record.result, Some(JournalResult::Completed));
    }

    #[tokio::test]
    async fn coordinator_persists_recovery_when_index_reconciliation_fails() {
        let store = TestJournalStore::default();
        let scope = RecoveryScope::worktree(
            RecoveryPath::unix(b"/tmp/ideagit-common".to_vec()).expect("common path"),
            RecoveryPath::unix(b"/tmp/ideagit-git".to_vec()).expect("git path"),
        );
        let journal = PartialCommitJournal::new(store.clone(), scope);
        let coordinator =
            PartialCommitCoordinator::new(TestWriter, journal, StableObserver, FailingPublisher);
        let mut transaction = PartialCommitTransaction::begin(request()).expect("transaction");
        transaction
            .temp_index_prepared(None, None)
            .expect("temporary Index");
        transaction
            .candidate_ready(
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
                    .parse()
                    .expect("tree"),
            )
            .expect("candidate");
        transaction
            .reconciliation_candidate([7; 32], b"verified-index".to_vec())
            .expect("reconciliation candidate");
        transaction
            .revalidate_before_commit(None, None)
            .expect("revalidation");
        let target = transaction.request.selections[0].target();

        let operation = OperationId::new();
        let temporary_index = test_temporary_index(operation);
        let error = coordinator
            .commit(
                target,
                operation,
                &mut transaction,
                &temporary_index,
                test_context(operation),
            )
            .await
            .expect_err("publication failure must require recovery");

        assert_eq!(error.kind, GitErrorKind::PermissionDenied);
        assert_eq!(transaction.phase, PartialCommitPhase::RecoveryRequired);
        let record = store
            .record
            .lock()
            .expect("journal lock")
            .clone()
            .expect("journal record");
        assert_eq!(record.phase, JournalPhase::RecoveryRequired);
        assert!(matches!(
            record.result,
            Some(JournalResult::RecoveryRequired { .. })
        ));
    }

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

    #[test]
    fn mixed_staged_and_unstaged_content_cannot_bypass_index_identity() {
        let mut transaction = PartialCommitTransaction::begin(request()).expect("valid request");
        transaction
            .temp_index_prepared(None, None)
            .expect("prepare");
        let tree: ObjectId = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
            .parse()
            .expect("tree");
        transaction.candidate_ready(tree).expect("candidate");
        let changed_index: ObjectId = "2222222222222222222222222222222222222222"
            .parse()
            .expect("index");
        let error = transaction
            .revalidate_before_commit(None, Some(changed_index))
            .expect_err("changed Index must reject the mixed-state commit");
        assert_eq!(error.kind, GitErrorKind::IndexChangedExternally);
        assert_eq!(transaction.phase, PartialCommitPhase::RecoveryRequired);
    }

    #[test]
    fn commit_observation_mismatch_enters_recovery_without_claiming_success() {
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
        transaction.begin_commit().expect("begin");
        let committed: ObjectId = "1111111111111111111111111111111111111111"
            .parse()
            .expect("commit");
        let external: ObjectId = "2222222222222222222222222222222222222222"
            .parse()
            .expect("external");
        transaction.commit_observed(committed).expect("observe");
        let error = transaction
            .observe_head_after_commit(Some(external))
            .expect_err("external HEAD must not be reported as success");
        assert_eq!(error.kind, GitErrorKind::HeadChangedExternally);
        assert_eq!(transaction.phase, PartialCommitPhase::RecoveryRequired);
    }

    #[test]
    fn real_repository_external_commit_with_index_change_requires_recovery() {
        use std::fs;

        use git_fixture::TestRepository;

        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("tracked.txt", b"base\n")
            .expect("initial commit");
        let index_path = fixture.worktree.join(".git/index");
        let original_index = fs::read(&index_path).expect("index");
        let original_head: ObjectId = fixture
            .head_oid()
            .expect("head")
            .expect("initial head")
            .parse()
            .expect("valid head");
        fixture
            .write_file("tracked.txt", b"external worktree\n")
            .expect("worktree edit");
        fixture
            .commit_file("other.txt", b"external commit\n")
            .expect("external commit");
        let observed_head: ObjectId = fixture
            .head_oid()
            .expect("head")
            .expect("updated head")
            .parse()
            .expect("valid head");
        let plan = RealIndexReconciler::plan(
            RealIndexReconciler::digest(&original_index),
            &fs::read(&index_path).expect("unchanged index"),
            Some(original_head),
            Some(observed_head),
            Some(original_index.clone()),
        )
        .expect("plan");
        assert!(matches!(plan, ReconciliationPlan::RecoveryRequired { .. }));
    }
}
