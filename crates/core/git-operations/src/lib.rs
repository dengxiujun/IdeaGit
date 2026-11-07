//! Operation coordination for typed Git commands.
//!
//! CORE-010 supplies the bounded queue and lifecycle contract. It delegates the actual Git side
//! effect to an injected executor; resource locks, journaling, and recovery are later tasks.

mod index;
mod lock;
mod recovery;

pub use index::CoordinatedIndexWriter;
pub use lock::{
    LockError, ResourceKey, ResourceKind, ResourceLock, ResourceLockManager, ResourceScope,
};
pub use recovery::{RecoveryDisposition, RecoveryObservation, classify as classify_recovery};

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use git_domain::{GitCommand, GitError, GitErrorKind, GitResult, OperationId, SessionTarget};
use git_ports::{ExecutionContext, ProcessProgress};
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
    use git_domain::{RepoPath, RepositoryId, SessionTarget, WorktreeId};
    use std::sync::Arc;
    use tokio::sync::Notify;

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
}
