//! Operation coordination for typed Git commands.
//!
//! CORE-010 supplies the bounded queue and lifecycle contract. It delegates the actual Git side
//! effect to an injected executor; resource locks, journaling, and recovery are later tasks.

mod branch;
mod commit;
mod index;
mod lock;
mod recovery;

pub use branch::{CoordinatedBranchWriter, JournaledBranchWriter, ReconciledBranchWriter};
pub use commit::{
    CommitPreflightState, CommitStateReader, CoordinatedCommitWriter, JournaledCommitWriter,
    PreflightCommitWriter, ReconciledCommitWriter,
};
pub use index::{
    CoordinatedIndexWriter, JournaledIndexWriter, ReconciledIndexWriter, SnapshotRebuilder,
};
pub use lock::{
    LockError, ResourceKey, ResourceKind, ResourceLock, ResourceLockManager, ResourceScope,
};
pub use recovery::{RecoveryDisposition, RecoveryObservation, classify as classify_recovery};

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use git_domain::{GitCommand, GitError, GitErrorKind, GitResult, OperationId, SessionTarget};
use git_ports::{
    BranchWriter, CommitWriter, ExecutionContext, IndexMutation, IndexWriter, ProcessProgress,
};
use thiserror::Error;
use tokio::sync::{Mutex, mpsc, watch};
use tokio_util::sync::CancellationToken;

/// Lifecycle phases published by the coordinator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationPhase {
    Queued,
    ResourceWait,
    Preflight,
    Running,
    Reconciling,
    Finished,
}

/// Explicit cancellation result; cancellation never implies rollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelOutcome {
    NotStarted,
    StoppedAndChecked,
    NeedsRecovery,
}

/// Terminal business outcome for one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationOutcome {
    Success,
    Failed(GitError),
    Cancelled(CancelOutcome),
    RecoveryRequired(GitError),
}

/// Latest observable state for one submitted command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationSnapshot {
    pub id: OperationId,
    pub target: SessionTarget,
    pub command: GitCommand,
    pub phase: OperationPhase,
    pub outcome: Option<OperationOutcome>,
}

impl OperationSnapshot {
    fn queued(id: OperationId, command: GitCommand) -> Self {
        Self {
            id,
            target: command.target(),
            command,
            phase: OperationPhase::Queued,
            outcome: None,
        }
    }
}

/// Executor boundary implemented by Git adapter/application integration.
#[async_trait]
pub trait OperationExecutor: Send + Sync + 'static {
    async fn execute(&self, command: GitCommand, context: ExecutionContext) -> GitResult<()>;
}

/// Routes the complete M2 write-command set into independently composed writer stacks.
///
/// The composition root remains responsible for wrapping each writer with the required resource
/// lock, Journal, preflight, and reconciliation decorators before constructing this executor.
#[derive(Debug)]
pub struct M2OperationExecutor<I, C, B> {
    index: I,
    commit: C,
    branch: B,
}

impl<I, C, B> M2OperationExecutor<I, C, B> {
    #[must_use]
    pub fn new(index: I, commit: C, branch: B) -> Self {
        Self {
            index,
            commit,
            branch,
        }
    }
}

#[async_trait]
impl<I, C, B> OperationExecutor for M2OperationExecutor<I, C, B>
where
    I: IndexWriter + Send + Sync + 'static,
    C: CommitWriter + Send + Sync + 'static,
    B: BranchWriter + Send + Sync + 'static,
{
    async fn execute(&self, command: GitCommand, context: ExecutionContext) -> GitResult<()> {
        match command {
            GitCommand::StageFiles { target, paths } => {
                self.index
                    .apply(IndexMutation::Stage { target, paths }, context)
                    .await
            }
            GitCommand::UnstageFiles { target, paths } => {
                self.index
                    .apply(IndexMutation::Unstage { target, paths }, context)
                    .await
            }
            GitCommand::CommitIndex { target, request } => self
                .commit
                .commit(target, request, context)
                .await
                .map(|_| ()),
            GitCommand::Branch { target, command } => {
                self.branch.apply(target, command, context).await
            }
            other => Err(GitError::new(
                GitErrorKind::UnsupportedFeature,
                format!("M2 operation executor does not support {other:?}"),
            )),
        }
    }
}

/// Bridges file-based Stage/Unstage commands to the coordinated Index writer stack.
#[derive(Debug)]
pub struct IndexOperationExecutor<W> {
    writer: W,
}

impl<W> IndexOperationExecutor<W> {
    #[must_use]
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

#[async_trait]
impl<W: IndexWriter + Send + Sync + 'static> OperationExecutor for IndexOperationExecutor<W> {
    async fn execute(&self, command: GitCommand, context: ExecutionContext) -> GitResult<()> {
        let mutation = match command {
            GitCommand::StageFiles { target, paths } => IndexMutation::Stage { target, paths },
            GitCommand::UnstageFiles { target, paths } => IndexMutation::Unstage { target, paths },
            other => {
                return Err(GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    format!("operation executor does not support {other:?}"),
                ));
            }
        };
        self.writer.apply(mutation, context).await
    }
}

/// Bridges standard Index commits to the coordinated Commit writer stack.
#[derive(Debug)]
pub struct CommitOperationExecutor<W> {
    writer: W,
}

impl<W> CommitOperationExecutor<W> {
    #[must_use]
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

#[async_trait]
impl<W: CommitWriter + Send + Sync + 'static> OperationExecutor for CommitOperationExecutor<W> {
    async fn execute(&self, command: GitCommand, context: ExecutionContext) -> GitResult<()> {
        let (target, request) = match command {
            GitCommand::CommitIndex { target, request } => (target, request),
            other => {
                return Err(GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    format!("commit executor does not support {other:?}"),
                ));
            }
        };
        self.writer
            .commit(target, request, context)
            .await
            .map(|_| ())
    }
}

/// Bridges typed branch commands to the coordinated Branch writer stack.
#[derive(Debug)]
pub struct BranchOperationExecutor<W> {
    writer: W,
}

impl<W> BranchOperationExecutor<W> {
    #[must_use]
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

#[async_trait]
impl<W: BranchWriter + Send + Sync + 'static> OperationExecutor for BranchOperationExecutor<W> {
    async fn execute(&self, command: GitCommand, context: ExecutionContext) -> GitResult<()> {
        let (target, branch) = match command {
            GitCommand::Branch { target, command } => (target, command),
            other => {
                return Err(GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    format!("branch executor does not support {other:?}"),
                ));
            }
        };
        self.writer.apply(target, branch, context).await
    }
}

struct OperationEntry {
    snapshot: OperationSnapshot,
    updates: watch::Sender<OperationSnapshot>,
    cancellation: CancellationToken,
}

struct CoordinatorState {
    operations: HashMap<OperationId, OperationEntry>,
}

struct Submission {
    id: OperationId,
    command: GitCommand,
    cancellation: CancellationToken,
}

/// Bounded, serial operation coordinator.
pub struct OperationCoordinator<E> {
    submissions: mpsc::Sender<Submission>,
    state: Arc<Mutex<CoordinatorState>>,
    worker: tokio::task::JoinHandle<()>,
    _executor: std::marker::PhantomData<E>,
}

impl<E: OperationExecutor> OperationCoordinator<E> {
    /// Starts a coordinator with a bounded submission queue.
    pub fn new(executor: E, capacity: usize) -> Result<Self, CoordinatorError> {
        if capacity == 0 {
            return Err(CoordinatorError::ZeroCapacity);
        }
        let (submissions, receiver) = mpsc::channel(capacity);
        let state = Arc::new(Mutex::new(CoordinatorState {
            operations: HashMap::new(),
        }));
        let worker_state = Arc::clone(&state);
        let worker = tokio::spawn(run_worker(executor, receiver, worker_state));
        Ok(Self {
            submissions,
            state,
            worker,
            _executor: std::marker::PhantomData,
        })
    }

    /// Validates and queues one command without executing on the caller task.
    pub async fn submit(&self, command: GitCommand) -> Result<OperationId, CoordinatorError> {
        command
            .validate()
            .map_err(CoordinatorError::InvalidCommand)?;
        let id = OperationId::new();
        let cancellation = CancellationToken::new();
        let snapshot = OperationSnapshot::queued(id, command.clone());
        let (updates, _) = watch::channel(snapshot.clone());
        self.state.lock().await.operations.insert(
            id,
            OperationEntry {
                snapshot,
                updates,
                cancellation: cancellation.clone(),
            },
        );
        if self
            .submissions
            .try_send(Submission {
                id,
                command,
                cancellation,
            })
            .is_err()
        {
            self.state.lock().await.operations.remove(&id);
            return Err(CoordinatorError::QueueFull);
        }
        Ok(id)
    }

    /// Returns the current latest-only operation state.
    pub async fn operation(&self, id: OperationId) -> Option<OperationSnapshot> {
        self.state
            .lock()
            .await
            .operations
            .get(&id)
            .map(|entry| entry.snapshot.clone())
    }

    /// Subscribes to the latest-only state stream for an operation.
    pub async fn subscribe(&self, id: OperationId) -> Option<watch::Receiver<OperationSnapshot>> {
        self.state
            .lock()
            .await
            .operations
            .get(&id)
            .map(|entry| entry.updates.subscribe())
    }

    /// Requests cancellation. The executor still determines whether side effects occurred.
    pub async fn cancel(&self, id: OperationId) -> Result<(), CoordinatorError> {
        let state = self.state.lock().await;
        let entry = state
            .operations
            .get(&id)
            .ok_or(CoordinatorError::UnknownOperation(id))?;
        entry.cancellation.cancel();
        Ok(())
    }

    /// Waits for the worker after the coordinator's submission sender is closed.
    pub async fn join(self) -> Result<(), tokio::task::JoinError> {
        drop(self.submissions);
        self.worker.await
    }
}

async fn run_worker<E: OperationExecutor>(
    executor: E,
    mut submissions: mpsc::Receiver<Submission>,
    state: Arc<Mutex<CoordinatorState>>,
) {
    while let Some(submission) = submissions.recv().await {
        set_phase(&state, submission.id, OperationPhase::ResourceWait).await;
        if submission.cancellation.is_cancelled() {
            finish(
                &state,
                submission.id,
                OperationOutcome::Cancelled(CancelOutcome::NotStarted),
            )
            .await;
            continue;
        }
        set_phase(&state, submission.id, OperationPhase::Preflight).await;
        set_phase(&state, submission.id, OperationPhase::Running).await;
        let (progress, mut progress_receiver) = mpsc::channel::<ProcessProgress>(16);
        let context =
            ExecutionContext::new(submission.id, submission.cancellation.clone(), progress);
        let progress_drain =
            tokio::spawn(async move { while progress_receiver.recv().await.is_some() {} });
        let execution = executor.execute(submission.command, context).await;
        let _ = progress_drain.await;
        set_phase(&state, submission.id, OperationPhase::Reconciling).await;
        let outcome = match execution {
            Ok(()) => OperationOutcome::Success,
            Err(error) if error.kind == GitErrorKind::Cancelled => {
                OperationOutcome::Cancelled(CancelOutcome::NeedsRecovery)
            }
            Err(error) => OperationOutcome::Failed(error),
        };
        finish(&state, submission.id, outcome).await;
    }
}

async fn set_phase(state: &Arc<Mutex<CoordinatorState>>, id: OperationId, phase: OperationPhase) {
    let mut state = state.lock().await;
    if let Some(entry) = state.operations.get_mut(&id) {
        entry.snapshot.phase = phase;
        let _ = entry.updates.send(entry.snapshot.clone());
    }
}

async fn finish(state: &Arc<Mutex<CoordinatorState>>, id: OperationId, outcome: OperationOutcome) {
    let mut state = state.lock().await;
    if let Some(entry) = state.operations.get_mut(&id) {
        entry.snapshot.phase = OperationPhase::Finished;
        entry.snapshot.outcome = Some(outcome);
        let _ = entry.updates.send(entry.snapshot.clone());
    }
}

/// Coordinator construction and submission failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoordinatorError {
    #[error("operation coordinator capacity must be greater than zero")]
    ZeroCapacity,
    #[error("operation queue is full")]
    QueueFull,
    #[error("invalid Git command: {0}")]
    InvalidCommand(#[from] git_domain::CommandValidationError),
    #[error("unknown operation: {0}")]
    UnknownOperation(OperationId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{
        BranchCommand, CommitOptions, RefName, RepoPath, RepositoryId, SessionTarget, SigningMode,
        SnapshotRevision, StandardCommitRequest, WorktreeId,
    };
    use git_ports::IndexMutation;
    use std::sync::Arc;
    use tokio::sync::Notify;

    struct RecordingIndexWriter {
        seen: Arc<std::sync::Mutex<Vec<IndexMutation>>>,
    }

    #[async_trait]
    impl IndexWriter for RecordingIndexWriter {
        async fn apply(
            &self,
            mutation: IndexMutation,
            _context: ExecutionContext,
        ) -> GitResult<()> {
            self.seen
                .lock()
                .expect("test mutex should not poison")
                .push(mutation);
            Ok(())
        }
    }

    struct RecordingCommitWriter {
        seen: Arc<std::sync::Mutex<usize>>,
    }

    #[async_trait]
    impl CommitWriter for RecordingCommitWriter {
        async fn commit(
            &self,
            _target: SessionTarget,
            _request: StandardCommitRequest,
            _context: ExecutionContext,
        ) -> GitResult<git_ports::CommitOutcome> {
            *self.seen.lock().expect("test mutex should not poison") += 1;
            Ok(git_ports::CommitOutcome {
                commit: git_domain::ObjectId::from_bytes(git_domain::ObjectFormat::Sha1, &[2; 20])
                    .expect("test OID should be valid"),
                parents: Vec::new(),
            })
        }
    }

    struct RecordingBranchWriter {
        seen: Arc<std::sync::Mutex<usize>>,
    }

    #[derive(Clone)]
    struct FailingRebuilder {
        calls: Arc<std::sync::Mutex<usize>>,
    }

    #[async_trait]
    impl SnapshotRebuilder for FailingRebuilder {
        async fn rebuild(
            &self,
            _target: SessionTarget,
            _context: ExecutionContext,
        ) -> GitResult<git_domain::RepositorySnapshot> {
            *self.calls.lock().expect("test mutex should not poison") += 1;
            Err(GitError::new(
                GitErrorKind::Internal,
                "authoritative rebuild failed",
            ))
        }
    }

    #[async_trait]
    impl BranchWriter for RecordingBranchWriter {
        async fn apply(
            &self,
            _target: SessionTarget,
            _command: BranchCommand,
            _context: ExecutionContext,
        ) -> GitResult<()> {
            *self.seen.lock().expect("test mutex should not poison") += 1;
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FakeExecutor {
        started: Arc<Notify>,
        release: Arc<Notify>,
        fail: bool,
    }

    #[async_trait]
    impl OperationExecutor for FakeExecutor {
        async fn execute(&self, _command: GitCommand, context: ExecutionContext) -> GitResult<()> {
            self.started.notify_one();
            self.release.notified().await;
            if context.cancellation.is_cancelled() {
                return Err(GitError::new(GitErrorKind::Cancelled, "cancelled"));
            }
            if self.fail {
                return Err(GitError::new(GitErrorKind::HookRejected, "hook rejected"));
            }
            Ok(())
        }
    }

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn command() -> GitCommand {
        GitCommand::StageFiles {
            target: target(),
            paths: vec![RepoPath::try_from("file.txt").expect("valid test path")],
        }
    }

    #[tokio::test]
    async fn publishes_lifecycle_and_success() {
        let executor = FakeExecutor {
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            fail: false,
        };
        let started = Arc::clone(&executor.started);
        let release = Arc::clone(&executor.release);
        let coordinator = OperationCoordinator::new(executor, 2).expect("valid capacity");
        let id = coordinator
            .submit(command())
            .await
            .expect("submit should queue");
        let mut updates = coordinator.subscribe(id).await.expect("operation exists");
        assert_eq!(updates.borrow().phase, OperationPhase::Queued);
        started.notified().await;
        release.notify_one();
        while updates.borrow().phase != OperationPhase::Finished {
            updates.changed().await.expect("worker should publish");
        }
        assert_eq!(updates.borrow().outcome, Some(OperationOutcome::Success));
        coordinator.join().await.expect("worker should stop");
    }

    #[tokio::test]
    async fn cancellation_before_execution_is_not_success() {
        let executor = FakeExecutor {
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            fail: false,
        };
        let coordinator = OperationCoordinator::new(executor, 1).expect("valid capacity");
        let id = coordinator
            .submit(command())
            .await
            .expect("submit should queue");
        coordinator
            .cancel(id)
            .await
            .expect("cancel should be accepted");
        let mut updates = coordinator.subscribe(id).await.expect("operation exists");
        while updates.borrow().phase != OperationPhase::Finished {
            updates.changed().await.expect("worker should publish");
        }
        assert_eq!(
            updates.borrow().outcome,
            Some(OperationOutcome::Cancelled(CancelOutcome::NotStarted))
        );
        coordinator.join().await.expect("worker should stop");
    }

    #[tokio::test]
    async fn executor_failure_is_structured() {
        let executor = FakeExecutor {
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            fail: true,
        };
        let started = Arc::clone(&executor.started);
        let release = Arc::clone(&executor.release);
        let coordinator = OperationCoordinator::new(executor, 1).expect("valid capacity");
        let id = coordinator
            .submit(command())
            .await
            .expect("submit should queue");
        started.notified().await;
        release.notify_one();
        let mut updates = coordinator.subscribe(id).await.expect("operation exists");
        while updates.borrow().phase != OperationPhase::Finished {
            updates.changed().await.expect("worker should publish");
        }
        assert!(matches!(
            updates.borrow().outcome,
            Some(OperationOutcome::Failed(_))
        ));
        coordinator.join().await.expect("worker should stop");
    }

    #[tokio::test]
    async fn index_executor_maps_stage_and_rejects_other_commands() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = RecordingIndexWriter {
            seen: Arc::clone(&seen),
        };
        let executor = IndexOperationExecutor::new(writer);
        let target = target();
        let path = RepoPath::try_from("file.txt").expect("valid path");
        let (progress, _) = mpsc::channel(2);
        executor
            .execute(
                GitCommand::StageFiles {
                    target,
                    paths: vec![path],
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("stage should map");
        assert_eq!(seen.lock().expect("test mutex should not poison").len(), 1);
        let (progress, _) = mpsc::channel(2);
        let error = executor
            .execute(
                GitCommand::Remote {
                    target,
                    command: git_domain::RemoteCommand::Fetch { remote: None },
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect_err("unsupported command should fail");
        assert_eq!(error.kind, GitErrorKind::UnsupportedFeature);
    }

    #[tokio::test]
    async fn commit_executor_maps_standard_commit() {
        let seen = Arc::new(std::sync::Mutex::new(0));
        let executor = CommitOperationExecutor::new(RecordingCommitWriter {
            seen: Arc::clone(&seen),
        });
        let (progress, _) = mpsc::channel(2);
        executor
            .execute(
                GitCommand::CommitIndex {
                    target: target(),
                    request: StandardCommitRequest {
                        options: CommitOptions {
                            message: b"message".to_vec(),
                            amend: false,
                            allow_empty: false,
                            signoff: false,
                            author: None,
                            committer: None,
                            signing: SigningMode::UseGitConfig,
                        },
                        expected_revision: SnapshotRevision::new(1),
                        expected_head: None,
                        expected_index: None,
                    },
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("commit should map");
        assert_eq!(*seen.lock().expect("test mutex should not poison"), 1);
    }

    #[tokio::test]
    async fn branch_executor_maps_typed_branch_command() {
        let seen = Arc::new(std::sync::Mutex::new(0));
        let executor = BranchOperationExecutor::new(RecordingBranchWriter {
            seen: Arc::clone(&seen),
        });
        let (progress, _) = mpsc::channel(2);
        executor
            .execute(
                GitCommand::Branch {
                    target: target(),
                    command: BranchCommand::Create {
                        name: RefName::new(b"feature".to_vec()).expect("valid branch name"),
                    },
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("branch should map");
        assert_eq!(*seen.lock().expect("test mutex should not poison"), 1);
    }

    #[tokio::test]
    async fn m2_executor_routes_index_commit_and_branch_but_rejects_remote() {
        let index_seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let commit_seen = Arc::new(std::sync::Mutex::new(0));
        let branch_seen = Arc::new(std::sync::Mutex::new(0));
        let executor = M2OperationExecutor::new(
            RecordingIndexWriter {
                seen: Arc::clone(&index_seen),
            },
            RecordingCommitWriter {
                seen: Arc::clone(&commit_seen),
            },
            RecordingBranchWriter {
                seen: Arc::clone(&branch_seen),
            },
        );
        let target = target();
        let path = RepoPath::try_from("file.txt").expect("valid path");

        executor
            .execute(
                GitCommand::StageFiles {
                    target,
                    paths: vec![path],
                },
                context(),
            )
            .await
            .expect("stage should route");
        executor
            .execute(
                GitCommand::CommitIndex {
                    target,
                    request: StandardCommitRequest {
                        options: CommitOptions {
                            message: b"message".to_vec(),
                            amend: false,
                            allow_empty: false,
                            signoff: false,
                            author: None,
                            committer: None,
                            signing: SigningMode::UseGitConfig,
                        },
                        expected_revision: SnapshotRevision::new(1),
                        expected_head: None,
                        expected_index: None,
                    },
                },
                context(),
            )
            .await
            .expect("commit should route");
        executor
            .execute(
                GitCommand::Branch {
                    target,
                    command: BranchCommand::Checkout {
                        name: RefName::new(b"main".to_vec()).expect("valid branch"),
                    },
                },
                context(),
            )
            .await
            .expect("branch should route");

        assert_eq!(
            index_seen
                .lock()
                .expect("test mutex should not poison")
                .len(),
            1
        );
        assert_eq!(
            *commit_seen.lock().expect("test mutex should not poison"),
            1
        );
        assert_eq!(
            *branch_seen.lock().expect("test mutex should not poison"),
            1
        );

        let error = executor
            .execute(
                GitCommand::Remote {
                    target,
                    command: git_domain::RemoteCommand::Fetch { remote: None },
                },
                context(),
            )
            .await
            .expect_err("M3 command should remain unavailable");
        assert_eq!(error.kind, GitErrorKind::UnsupportedFeature);
    }

    #[tokio::test]
    async fn m2_reconciled_writers_never_report_success_when_rebuild_fails() {
        let index_seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let commit_seen = Arc::new(std::sync::Mutex::new(0));
        let branch_seen = Arc::new(std::sync::Mutex::new(0));
        let rebuilds = Arc::new(std::sync::Mutex::new(0));
        let rebuilder = FailingRebuilder {
            calls: Arc::clone(&rebuilds),
        };
        let executor = M2OperationExecutor::new(
            ReconciledIndexWriter::new(
                RecordingIndexWriter {
                    seen: Arc::clone(&index_seen),
                },
                rebuilder.clone(),
            ),
            ReconciledCommitWriter::new(
                RecordingCommitWriter {
                    seen: Arc::clone(&commit_seen),
                },
                rebuilder.clone(),
            ),
            ReconciledBranchWriter::new(
                RecordingBranchWriter {
                    seen: Arc::clone(&branch_seen),
                },
                rebuilder,
            ),
        );
        let target = target();

        let index_error = executor
            .execute(
                GitCommand::StageFiles {
                    target,
                    paths: vec![RepoPath::try_from("file.txt").expect("valid path")],
                },
                context(),
            )
            .await
            .expect_err("failed Index rebuild must fail the operation");
        assert_eq!(index_error.kind, GitErrorKind::Internal);

        let commit_error = executor
            .execute(
                GitCommand::CommitIndex {
                    target,
                    request: StandardCommitRequest {
                        options: CommitOptions {
                            message: b"message".to_vec(),
                            amend: false,
                            allow_empty: false,
                            signoff: false,
                            author: None,
                            committer: None,
                            signing: SigningMode::UseGitConfig,
                        },
                        expected_revision: SnapshotRevision::new(1),
                        expected_head: None,
                        expected_index: None,
                    },
                },
                context(),
            )
            .await
            .expect_err("failed Commit rebuild must fail the operation");
        assert_eq!(commit_error.kind, GitErrorKind::Internal);

        let branch_error = executor
            .execute(
                GitCommand::Branch {
                    target,
                    command: BranchCommand::Create {
                        name: RefName::new(b"feature".to_vec()).expect("valid branch"),
                    },
                },
                context(),
            )
            .await
            .expect_err("failed Branch rebuild must fail the operation");
        assert_eq!(branch_error.kind, GitErrorKind::Internal);

        assert_eq!(
            index_seen
                .lock()
                .expect("test mutex should not poison")
                .len(),
            1
        );
        assert_eq!(
            *commit_seen.lock().expect("test mutex should not poison"),
            1
        );
        assert_eq!(
            *branch_seen.lock().expect("test mutex should not poison"),
            1
        );
        assert_eq!(*rebuilds.lock().expect("test mutex should not poison"), 3);
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel(2);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }
}
