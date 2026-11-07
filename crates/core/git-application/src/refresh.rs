//! Snapshot refresh orchestration and coalescing scheduler.

use std::{sync::Arc, time::Duration};

use git_domain::{
    GitError, GitErrorKind, GitResult, OperationId, RepositorySnapshot, SessionTarget,
    SnapshotBuilder, SnapshotRead, SnapshotRevision,
};
use git_operations::SnapshotRebuilder;
use git_ports::{
    BranchReader, ExecutionContext, HeadReader, RepositoryLocation, StatusOptions, StatusReader,
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, MissedTickBehavior},
};
use tokio_util::sync::CancellationToken;

/// Why a repository refresh was requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshTrigger {
    Initial,
    FilesystemChanged,
    RescanRequired,
    Manual,
}

/// Whether the last attempted authoritative read succeeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshHealth {
    Pending,
    Ready,
    Degraded,
}

/// Current publishable state for one session generation.
#[derive(Clone, Debug)]
pub struct RefreshState {
    pub target: SessionTarget,
    pub health: RefreshHealth,
    pub snapshot: Option<Arc<RepositorySnapshot>>,
    pub last_error: Option<GitError>,
}

impl RefreshState {
    fn pending(target: SessionTarget) -> Self {
        Self {
            target,
            health: RefreshHealth::Pending,
            snapshot: None,
            last_error: None,
        }
    }
}

/// Timing and buffering limits for a refresh scheduler.
#[derive(Clone, Copy, Debug)]
pub struct RefreshSchedulerConfig {
    pub debounce: Duration,
    pub verification_interval: Duration,
    pub trigger_capacity: usize,
}

impl Default for RefreshSchedulerConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(200),
            verification_interval: Duration::from_secs(30),
            trigger_capacity: 32,
        }
    }
}

/// Reads authoritative Git state and builds one immutable snapshot.
#[derive(Debug)]
pub struct SnapshotRefreshService<H, S> {
    head: H,
    status: S,
}

impl<H, S> SnapshotRefreshService<H, S> {
    #[must_use]
    pub const fn new(head: H, status: S) -> Self {
        Self { head, status }
    }
}

impl<H, S> SnapshotRefreshService<H, S>
where
    H: HeadReader + BranchReader,
    S: StatusReader,
{
    /// Executes one guarded read. It performs no retry and never mutates repository state.
    #[allow(clippy::result_large_err)]
    pub async fn refresh(
        &self,
        target: SessionTarget,
        location: &RepositoryLocation,
        previous_revision: SnapshotRevision,
        cancellation: CancellationToken,
    ) -> GitResult<RepositorySnapshot> {
        let (progress, _receiver) = mpsc::channel(16);
        let context = ExecutionContext::new(OperationId::new(), cancellation, progress);
        let head_before = self.head.read_head(location, context.clone()).await?;
        let read = if location.is_bare() {
            let tracking = self.head.read_tracking(location, context.clone()).await?;
            let head_after = self.head.read_head(location, context).await?;
            SnapshotRead::bare(head_before, tracking, head_after)
        } else {
            let status = self
                .status
                .status(location, StatusOptions::default(), context.clone())
                .await?;
            let tracking = self.head.read_tracking(location, context.clone()).await?;
            let head_after = self.head.read_head(location, context).await?;
            SnapshotRead::worktree(head_before, status, tracking, head_after)
        };
        SnapshotBuilder::new(target, previous_revision)
            .build(read)
            .map_err(|error| {
                GitError::new(
                    GitErrorKind::Internal,
                    "repository snapshot could not be built",
                )
                .with_diagnostic(error.to_string())
                .with_target(target)
            })
    }
}

/// Handle for a running coalescing refresh loop.
#[derive(Debug)]
pub struct RefreshScheduler {
    trigger: mpsc::Sender<RefreshRequest>,
    state: watch::Receiver<RefreshState>,
    task: JoinHandle<()>,
}

struct RefreshRequest {
    _trigger: RefreshTrigger,
    completion: Option<oneshot::Sender<GitResult<RepositorySnapshot>>>,
}

/// Operation-scoped handle that completes only after its requested authoritative refresh.
#[derive(Clone, Debug)]
pub struct ScheduledSnapshotRebuilder {
    target: SessionTarget,
    trigger: mpsc::Sender<RefreshRequest>,
}

#[async_trait::async_trait]
impl SnapshotRebuilder for ScheduledSnapshotRebuilder {
    #[allow(clippy::result_large_err)]
    async fn rebuild(
        &self,
        target: SessionTarget,
        context: ExecutionContext,
    ) -> GitResult<RepositorySnapshot> {
        if target != self.target {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "snapshot rebuilder target does not match its repository session",
            )
            .with_target(target)
            .with_operation(context.operation_id));
        }
        let (completion, result) = oneshot::channel();
        let request = RefreshRequest {
            _trigger: RefreshTrigger::Manual,
            completion: Some(completion),
        };
        tokio::select! {
            () = context.cancellation.cancelled() => {
                return Err(rebuild_cancelled(target, context.operation_id));
            }
            queued = self.trigger.send(request) => {
                if queued.is_err() {
                    return Err(GitError::new(
                        GitErrorKind::Internal,
                        "repository refresh scheduler is unavailable",
                    )
                    .with_target(target)
                    .with_operation(context.operation_id));
                }
            }
        }
        tokio::select! {
            () = context.cancellation.cancelled() => {
                Err(rebuild_cancelled(target, context.operation_id))
            }
            completed = result => {
                completed.unwrap_or_else(|_| {
                    Err(GitError::new(
                        GitErrorKind::Internal,
                        "repository refresh scheduler stopped before reconciliation",
                    )
                    .with_target(target)
                    .with_operation(context.operation_id))
                })
                .map_err(|error| {
                    error
                        .with_target(target)
                        .with_operation(context.operation_id)
                })
            }
        }
    }
}

fn rebuild_cancelled(target: SessionTarget, operation: OperationId) -> GitError {
    GitError::new(
        GitErrorKind::Cancelled,
        "repository reconciliation was cancelled",
    )
    .with_target(target)
    .with_operation(operation)
}

impl RefreshScheduler {
    /// Starts a scheduler for one session. An initial refresh is queued automatically.
    #[allow(clippy::result_large_err)]
    pub fn start<H, S>(
        service: SnapshotRefreshService<H, S>,
        target: SessionTarget,
        location: RepositoryLocation,
        cancellation: CancellationToken,
        config: RefreshSchedulerConfig,
    ) -> Result<Self, GitError>
    where
        H: HeadReader + BranchReader + Send + Sync + 'static,
        S: StatusReader + Send + Sync + 'static,
    {
        if config.trigger_capacity == 0
            || config.debounce.is_zero()
            || config.verification_interval.is_zero()
        {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "invalid repository refresh scheduler configuration",
            ));
        }
        let (trigger, receiver) = mpsc::channel(config.trigger_capacity);
        let (state_sender, state) = watch::channel(RefreshState::pending(target));
        let task_trigger = trigger.clone();
        let task = tokio::spawn(run_scheduler(
            service,
            target,
            location,
            cancellation,
            config,
            receiver,
            state_sender,
        ));
        task_trigger
            .try_send(RefreshRequest {
                _trigger: RefreshTrigger::Initial,
                completion: None,
            })
            .map_err(|_| {
                GitError::new(
                    GitErrorKind::Internal,
                    "initial repository refresh could not be queued",
                )
            })?;
        Ok(Self {
            trigger,
            state,
            task,
        })
    }

    /// Queues a possible-change signal. A full channel is collapsed into the already queued work.
    pub fn request(&self, trigger: RefreshTrigger) {
        let _ = self.trigger.try_send(RefreshRequest {
            _trigger: trigger,
            completion: None,
        });
    }

    /// Subscribes to the latest state only; this is not an event log.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<RefreshState> {
        self.state.clone()
    }

    /// Creates an operation-scoped writer reconciliation handle for this repository session.
    #[must_use]
    pub fn rebuilder(&self) -> ScheduledSnapshotRebuilder {
        ScheduledSnapshotRebuilder {
            target: self.state.borrow().target,
            trigger: self.trigger.clone(),
        }
    }

    /// Waits for the scheduler task after its session cancellation token is cancelled.
    pub async fn join(self) -> Result<(), tokio::task::JoinError> {
        self.task.await
    }
}

async fn run_scheduler<H, S>(
    service: SnapshotRefreshService<H, S>,
    target: SessionTarget,
    location: RepositoryLocation,
    cancellation: CancellationToken,
    config: RefreshSchedulerConfig,
    mut triggers: mpsc::Receiver<RefreshRequest>,
    state: watch::Sender<RefreshState>,
) where
    H: HeadReader + BranchReader,
    S: StatusReader,
{
    let mut revision = SnapshotRevision::INITIAL;
    let mut current = None;
    let mut verification = tokio::time::interval_at(
        Instant::now() + config.verification_interval,
        config.verification_interval,
    );
    verification.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        let mut completions = Vec::new();
        tokio::select! {
            () = cancellation.cancelled() => break,
            _ = verification.tick() => {},
            request = triggers.recv() => {
                let Some(request) = request else { break; };
                if let Some(completion) = request.completion {
                    completions.push(completion);
                }
                let cancelled = tokio::select! {
                    () = cancellation.cancelled() => true,
                    () = tokio::time::sleep(config.debounce) => false,
                };
                if cancelled {
                    let error = rebuild_cancelled(target, OperationId::new());
                    for completion in completions {
                        let _ = completion.send(Err(error.clone()));
                    }
                    break;
                }
                while let Ok(request) = triggers.try_recv() {
                    if let Some(completion) = request.completion {
                        completions.push(completion);
                    }
                }
            }
        }

        match service
            .refresh(target, &location, revision, cancellation.child_token())
            .await
        {
            Ok(snapshot) => {
                revision = snapshot.revision;
                for completion in completions {
                    let _ = completion.send(Ok(snapshot.clone()));
                }
                current = Some(Arc::new(snapshot));
                state.send_replace(RefreshState {
                    target,
                    health: RefreshHealth::Ready,
                    snapshot: current.clone(),
                    last_error: None,
                });
            }
            Err(error) if error.kind == GitErrorKind::Cancelled && cancellation.is_cancelled() => {
                for completion in completions {
                    let _ = completion.send(Err(error.clone()));
                }
                break;
            }
            Err(error) => {
                for completion in completions {
                    let _ = completion.send(Err(error.clone()));
                }
                state.send_replace(RefreshState {
                    target,
                    health: RefreshHealth::Degraded,
                    snapshot: current.clone(),
                    last_error: Some(error),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use git_domain::{
        BranchHead, BranchOid, BranchStatus, BranchTracking, HeadState, ObjectFormat, ObjectId,
        RefName, RepositoryId, StatusData, WorktreeId,
    };

    use super::*;

    #[derive(Clone)]
    struct FakeHead {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    #[async_trait]
    impl HeadReader for FakeHead {
        async fn read_head(
            &self,
            _location: &RepositoryLocation,
            _context: ExecutionContext,
        ) -> GitResult<HeadState> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(GitError::new(
                    GitErrorKind::InvalidRepository,
                    "head unavailable",
                ));
            }
            Ok(head())
        }
    }

    #[async_trait]
    impl BranchReader for FakeHead {
        async fn read_tracking(
            &self,
            _location: &RepositoryLocation,
            _context: ExecutionContext,
        ) -> GitResult<Option<BranchTracking>> {
            Ok(None)
        }
    }

    #[derive(Clone)]
    struct FakeStatus {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl StatusReader for FakeStatus {
        async fn status(
            &self,
            _location: &RepositoryLocation,
            _options: StatusOptions,
            _context: ExecutionContext,
        ) -> GitResult<StatusData> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(StatusData {
                branch: BranchStatus {
                    oid: Some(BranchOid::Object(oid())),
                    head: Some(BranchHead::Named(branch())),
                    ..BranchStatus::default()
                },
                entries: Vec::new(),
            })
        }
    }

    fn oid() -> ObjectId {
        ObjectId::from_bytes(ObjectFormat::Sha1, &[1; 20]).expect("test OID should be valid")
    }

    fn branch() -> RefName {
        RefName::new(b"main".to_vec()).expect("test branch should be valid")
    }

    fn head() -> HeadState {
        HeadState::Attached {
            branch: branch(),
            oid: oid(),
        }
    }

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn config() -> RefreshSchedulerConfig {
        RefreshSchedulerConfig {
            debounce: Duration::from_millis(10),
            verification_interval: Duration::from_secs(60),
            trigger_capacity: 4,
        }
    }

    #[tokio::test]
    async fn guarded_refresh_builds_the_next_snapshot() {
        let head_calls = Arc::new(AtomicUsize::new(0));
        let status_calls = Arc::new(AtomicUsize::new(0));
        let service = SnapshotRefreshService::new(
            FakeHead {
                calls: Arc::clone(&head_calls),
                fail: false,
            },
            FakeStatus {
                calls: Arc::clone(&status_calls),
            },
        );
        let snapshot = service
            .refresh(
                target(),
                &RepositoryLocation::worktree("repo", "repo/.git", "repo/.git"),
                SnapshotRevision::INITIAL,
                CancellationToken::new(),
            )
            .await
            .expect("refresh should succeed");

        assert_eq!(snapshot.revision, SnapshotRevision::new(1));
        assert_eq!(head_calls.load(Ordering::SeqCst), 2);
        assert_eq!(status_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scheduler_coalesces_bursts_and_cancels_with_the_session() {
        let head_calls = Arc::new(AtomicUsize::new(0));
        let cancellation = CancellationToken::new();
        let scheduler = RefreshScheduler::start(
            SnapshotRefreshService::new(
                FakeHead {
                    calls: Arc::clone(&head_calls),
                    fail: false,
                },
                FakeStatus {
                    calls: Arc::new(AtomicUsize::new(0)),
                },
            ),
            target(),
            RepositoryLocation::worktree("repo", "repo/.git", "repo/.git"),
            cancellation.clone(),
            config(),
        )
        .expect("scheduler should start");
        let mut state = scheduler.subscribe();
        scheduler.request(RefreshTrigger::FilesystemChanged);
        scheduler.request(RefreshTrigger::FilesystemChanged);
        scheduler.request(RefreshTrigger::RescanRequired);

        tokio::time::timeout(Duration::from_secs(1), state.changed())
            .await
            .expect("refresh should finish")
            .expect("state sender should remain open");
        assert_eq!(state.borrow().health, RefreshHealth::Ready);
        assert_eq!(
            state.borrow().snapshot.as_ref().map(|value| value.revision),
            Some(SnapshotRevision::new(1))
        );
        assert_eq!(head_calls.load(Ordering::SeqCst), 2);

        cancellation.cancel();
        scheduler
            .join()
            .await
            .expect("scheduler should stop cleanly");
    }

    #[tokio::test]
    async fn failed_refresh_enters_degraded_state_without_inventing_a_snapshot() {
        let cancellation = CancellationToken::new();
        let scheduler = RefreshScheduler::start(
            SnapshotRefreshService::new(
                FakeHead {
                    calls: Arc::new(AtomicUsize::new(0)),
                    fail: true,
                },
                FakeStatus {
                    calls: Arc::new(AtomicUsize::new(0)),
                },
            ),
            target(),
            RepositoryLocation::worktree("repo", "repo/.git", "repo/.git"),
            cancellation.clone(),
            config(),
        )
        .expect("scheduler should start");
        let mut state = scheduler.subscribe();

        tokio::time::timeout(Duration::from_secs(1), state.changed())
            .await
            .expect("failed refresh should publish")
            .expect("state sender should remain open");
        assert_eq!(state.borrow().health, RefreshHealth::Degraded);
        assert!(state.borrow().snapshot.is_none());
        assert_eq!(
            state.borrow().last_error.as_ref().map(|error| error.kind),
            Some(GitErrorKind::InvalidRepository)
        );

        cancellation.cancel();
        scheduler
            .join()
            .await
            .expect("scheduler should stop cleanly");
    }

    #[tokio::test]
    async fn scheduled_rebuilder_waits_for_its_own_authoritative_refresh() {
        let cancellation = CancellationToken::new();
        let target = target();
        let scheduler = RefreshScheduler::start(
            SnapshotRefreshService::new(
                FakeHead {
                    calls: Arc::new(AtomicUsize::new(0)),
                    fail: false,
                },
                FakeStatus {
                    calls: Arc::new(AtomicUsize::new(0)),
                },
            ),
            target,
            RepositoryLocation::worktree("repo", "repo/.git", "repo/.git"),
            cancellation.clone(),
            config(),
        )
        .expect("scheduler should start");
        let mut state = scheduler.subscribe();
        tokio::time::timeout(Duration::from_secs(1), state.changed())
            .await
            .expect("initial refresh should finish")
            .expect("state sender should remain open");
        assert_eq!(
            state.borrow().snapshot.as_ref().map(|value| value.revision),
            Some(SnapshotRevision::new(1))
        );

        let (progress, _receiver) = mpsc::channel(1);
        let rebuilt = scheduler
            .rebuilder()
            .rebuild(
                target,
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("scheduled reconciliation should succeed");
        assert_eq!(rebuilt.revision, SnapshotRevision::new(2));

        cancellation.cancel();
        scheduler
            .join()
            .await
            .expect("scheduler should stop cleanly");
    }

    #[tokio::test]
    async fn scheduled_rebuilder_preserves_refresh_failure() {
        let cancellation = CancellationToken::new();
        let target = target();
        let scheduler = RefreshScheduler::start(
            SnapshotRefreshService::new(
                FakeHead {
                    calls: Arc::new(AtomicUsize::new(0)),
                    fail: true,
                },
                FakeStatus {
                    calls: Arc::new(AtomicUsize::new(0)),
                },
            ),
            target,
            RepositoryLocation::worktree("repo", "repo/.git", "repo/.git"),
            cancellation.clone(),
            config(),
        )
        .expect("scheduler should start");
        let (progress, _receiver) = mpsc::channel(1);
        let error = scheduler
            .rebuilder()
            .rebuild(
                target,
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect_err("failed authoritative refresh must fail reconciliation");
        assert_eq!(error.kind, GitErrorKind::InvalidRepository);

        cancellation.cancel();
        scheduler
            .join()
            .await
            .expect("scheduler should stop cleanly");
    }

    #[test]
    fn invalid_scheduler_limits_are_rejected() {
        let result = RefreshScheduler::start(
            SnapshotRefreshService::new(
                FakeHead {
                    calls: Arc::new(AtomicUsize::new(0)),
                    fail: false,
                },
                FakeStatus {
                    calls: Arc::new(AtomicUsize::new(0)),
                },
            ),
            target(),
            RepositoryLocation::bare("repo.git", "repo.git"),
            CancellationToken::new(),
            RefreshSchedulerConfig {
                trigger_capacity: 0,
                ..config()
            },
        );
        assert!(matches!(result, Err(error) if error.kind == GitErrorKind::Internal));
    }
}
