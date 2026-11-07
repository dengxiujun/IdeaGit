//! Composition root for application services, adapters, platform, shell, and feature views.
//!
//! UI-001 initializes GPUI-Kit and opens only the minimal shell window. Git services and the
//! production workbench remain intentionally uninitialized until their tracked tasks.

use git_operations::{OperationCoordinator, OperationExecutor, OperationPhase, OperationSnapshot};
use git_ui_model::{ActionDispatcher, ActionRequest, to_git_command};
use gpui_kit::QuitMode;
use std::{
    env::VarError,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const SMOKE_DURATION_ENV: &str = "IDEAGIT_UI_SMOKE_DURATION_MS";
const OPEN_REPOSITORY_ENV: &str = "IDEAGIT_OPEN_REPOSITORY";

enum DesktopUiUpdate {
    Repository(git_ui_model::RepositoryWorkbenchModel),
    Log(git_domain::CommitPage),
    LogFailed(String),
    RecoveryBlocked(String),
    WriteReady {
        dispatcher: Arc<dyn ActionDispatcher>,
        updates: tokio::sync::mpsc::UnboundedReceiver<OperationSnapshot>,
    },
}

/// Composition-root adapter from the Git CLI probe to the Core commit preflight contract.
#[derive(Debug)]
pub struct DesktopCommitStateReader<R> {
    target: git_domain::SessionTarget,
    probe: git_cli::GitCliCommitStateProbe<R>,
}

impl<R> DesktopCommitStateReader<R> {
    #[must_use]
    pub fn new(
        target: git_domain::SessionTarget,
        probe: git_cli::GitCliCommitStateProbe<R>,
    ) -> Self {
        Self { target, probe }
    }
}

#[async_trait::async_trait]
impl<R: git_ports::ProcessRunner> git_operations::CommitStateReader
    for DesktopCommitStateReader<R>
{
    async fn read(
        &self,
        target: git_domain::SessionTarget,
        context: git_ports::ExecutionContext,
    ) -> git_domain::GitResult<git_operations::CommitPreflightState> {
        if target != self.target {
            return Err(git_domain::GitError::new(
                git_domain::GitErrorKind::InvalidRepository,
                "commit preflight target does not match its repository session",
            )
            .with_target(target)
            .with_operation(context.operation_id));
        }
        self.probe
            .read(context)
            .await
            .map(|state| git_operations::CommitPreflightState {
                head: state.head,
                index: Some(state.index),
            })
    }
}

#[async_trait::async_trait]
impl<R: git_ports::ProcessRunner> git_operations::RecoveryStateReader
    for DesktopCommitStateReader<R>
{
    async fn read(
        &self,
        target: git_domain::SessionTarget,
        context: git_ports::ExecutionContext,
    ) -> git_domain::GitResult<git_operations::RecoveryObservation> {
        git_operations::CommitStateReader::read(self, target, context)
            .await
            .map(|state| git_operations::RecoveryObservation {
                head: state.head,
                index: state.index,
            })
    }
}

/// Complete M2 Index writer stack assembled by the desktop composition root.
pub type DesktopIndexWriter = git_operations::ReconciledIndexWriter<
    git_operations::CoordinatedIndexWriter<
        git_operations::JournaledIndexWriter<
            git_cli::GitCliIndexWriter<git_cli::TokioProcessRunner>,
            persistence::JsonJournalStore,
            DesktopCommitStateReader<git_cli::TokioProcessRunner>,
        >,
    >,
    git_application::ScheduledSnapshotRebuilder,
>;

/// Complete M2 Commit writer stack assembled by the desktop composition root.
pub type DesktopCommitWriter = git_operations::ReconciledCommitWriter<
    git_operations::CoordinatedCommitWriter<
        git_operations::PreflightCommitWriter<
            git_operations::JournaledCommitWriter<
                git_cli::GitCliCommitWriter<git_cli::TokioProcessRunner>,
                persistence::JsonJournalStore,
            >,
            DesktopCommitStateReader<git_cli::TokioProcessRunner>,
        >,
    >,
    git_application::ScheduledSnapshotRebuilder,
>;

/// Complete M2 Branch writer stack assembled by the desktop composition root.
pub type DesktopBranchWriter = git_operations::ReconciledBranchWriter<
    git_operations::CoordinatedBranchWriter<
        git_operations::JournaledBranchWriter<
            git_cli::GitCliBranchWriter<git_cli::TokioProcessRunner>,
            persistence::JsonJournalStore,
        >,
    >,
    git_application::ScheduledSnapshotRebuilder,
>;

/// Complete Stage/Commit/Branch executor for one discovered repository session.
pub type DesktopM2Executor = git_operations::M2OperationExecutor<
    DesktopIndexWriter,
    DesktopCommitWriter,
    DesktopBranchWriter,
>;

/// Builds the production M2 writer stack without opening any GUI write capability by itself.
#[allow(clippy::result_large_err)]
pub fn build_m2_executor(
    target: git_domain::SessionTarget,
    location: git_ports::RepositoryLocation,
    journal_path: PathBuf,
    rebuilder: git_application::ScheduledSnapshotRebuilder,
) -> git_domain::GitResult<DesktopM2Executor> {
    let recovery_scope = location.recovery_scope().map_err(|error| {
        git_domain::GitError::new(
            git_domain::GitErrorKind::InvalidRepository,
            "repository has no usable persistent recovery scope",
        )
        .with_diagnostic(error.to_string())
        .with_target(target)
    })?;
    let locks = git_operations::ResourceLockManager::new();
    let journal = persistence::JsonJournalStore::new(journal_path);
    let index = git_operations::ReconciledIndexWriter::new(
        git_operations::CoordinatedIndexWriter::new(
            git_operations::JournaledIndexWriter::new(
                git_cli::GitCliIndexWriter::new(git_cli::TokioProcessRunner, location.clone()),
                journal.clone(),
                recovery_scope.clone(),
                DesktopCommitStateReader::new(
                    target,
                    git_cli::GitCliCommitStateProbe::new(
                        git_cli::TokioProcessRunner,
                        location.clone(),
                    ),
                ),
            ),
            locks.clone(),
        ),
        rebuilder.clone(),
    );
    let commit = git_operations::ReconciledCommitWriter::new(
        git_operations::CoordinatedCommitWriter::new(
            git_operations::PreflightCommitWriter::new(
                git_operations::JournaledCommitWriter::new(
                    git_cli::GitCliCommitWriter::new(git_cli::TokioProcessRunner, location.clone()),
                    journal.clone(),
                    recovery_scope.clone(),
                ),
                DesktopCommitStateReader::new(
                    target,
                    git_cli::GitCliCommitStateProbe::new(
                        git_cli::TokioProcessRunner,
                        location.clone(),
                    ),
                ),
            ),
            locks.clone(),
        ),
        rebuilder.clone(),
    );
    let branch = git_operations::ReconciledBranchWriter::new(
        git_operations::CoordinatedBranchWriter::new(
            git_operations::JournaledBranchWriter::new(
                git_cli::GitCliBranchWriter::new(git_cli::TokioProcessRunner, location),
                journal,
                recovery_scope,
            ),
            locks,
        ),
        rebuilder,
    );
    Ok(git_operations::M2OperationExecutor::new(
        target, index, commit, branch,
    ))
}

/// Composition-root dispatcher that queues typed UI actions without blocking GPUI.
pub struct DesktopActionDispatcher<E: OperationExecutor> {
    coordinator: Arc<OperationCoordinator<E>>,
    runtime: tokio::runtime::Handle,
    updates: tokio::sync::mpsc::UnboundedSender<OperationSnapshot>,
}

impl<E: OperationExecutor> DesktopActionDispatcher<E> {
    #[must_use]
    pub fn new(
        coordinator: Arc<OperationCoordinator<E>>,
        runtime: tokio::runtime::Handle,
    ) -> (
        Self,
        tokio::sync::mpsc::UnboundedReceiver<OperationSnapshot>,
    ) {
        let (updates, receiver) = tokio::sync::mpsc::unbounded_channel();
        (
            Self {
                coordinator,
                runtime,
                updates,
            },
            receiver,
        )
    }
}

impl<E: OperationExecutor> ActionDispatcher for DesktopActionDispatcher<E> {
    fn dispatch(&self, request: ActionRequest) -> Result<(), String> {
        let command = to_git_command(&request)?;
        let failed_command = command.clone();
        let coordinator = Arc::clone(&self.coordinator);
        let updates = self.updates.clone();
        self.runtime.spawn(async move {
            let id = match coordinator.submit(command).await {
                Ok(id) => id,
                Err(error) => {
                    tracing::error!(%error, "failed to queue desktop Git action");
                    let _ = updates.send(queue_failure_snapshot(failed_command, error));
                    return;
                }
            };
            let Some(mut snapshots) = coordinator.subscribe(id).await else {
                tracing::error!(%id, "queued desktop Git action has no snapshot stream");
                let error = git_domain::GitError::new(
                    git_domain::GitErrorKind::Internal,
                    "queued Git action has no operation state",
                )
                .with_target(failed_command.target())
                .with_operation(id);
                let _ = updates.send(OperationSnapshot {
                    id,
                    target: failed_command.target(),
                    command: failed_command,
                    phase: OperationPhase::Finished,
                    outcome: Some(git_operations::OperationOutcome::Failed(error)),
                });
                return;
            };
            loop {
                let snapshot = snapshots.borrow().clone();
                let finished = snapshot.phase == OperationPhase::Finished;
                if updates.send(snapshot).is_err() || finished {
                    break;
                }
                if snapshots.changed().await.is_err() {
                    break;
                }
            }
        });
        Ok(())
    }
}

fn queue_failure_snapshot(
    command: git_domain::GitCommand,
    error: git_operations::CoordinatorError,
) -> OperationSnapshot {
    let id = git_domain::OperationId::new();
    let kind = match error {
        git_operations::CoordinatorError::QueueFull => {
            git_domain::GitErrorKind::OperationAlreadyInProgress
        }
        git_operations::CoordinatorError::InvalidCommand(_) => {
            git_domain::GitErrorKind::StaleSelection
        }
        git_operations::CoordinatorError::ZeroCapacity
        | git_operations::CoordinatorError::UnknownOperation(_) => {
            git_domain::GitErrorKind::Internal
        }
    };
    let target = command.target();
    let error = git_domain::GitError::new(kind, error.to_string())
        .with_target(target)
        .with_operation(id);
    OperationSnapshot {
        id,
        target,
        command,
        phase: OperationPhase::Finished,
        outcome: Some(git_operations::OperationOutcome::Failed(error)),
    }
}

/// Installs one desktop dispatcher and forwards its coordinator snapshots onto the GPUI thread.
///
/// The runtime handle is captured explicitly because GPUI action callbacks do not run inside the
/// Tokio runtime. Dropping the workbench closes the UI task without retrying or repeating a Git
/// command.
pub fn install_action_dispatcher<E: OperationExecutor>(
    coordinator: Arc<OperationCoordinator<E>>,
    runtime: tokio::runtime::Handle,
    controller: &idea_shell::WorkbenchController,
    cx: &mut gpui_kit::App,
) -> Arc<DesktopActionDispatcher<E>> {
    let (dispatcher, updates) = DesktopActionDispatcher::new(coordinator, runtime);
    let dispatcher = Arc::new(dispatcher);
    install_prebuilt_action_dispatcher(dispatcher.clone(), updates, controller, cx);
    dispatcher
}

fn install_prebuilt_action_dispatcher(
    dispatcher: Arc<dyn ActionDispatcher>,
    mut updates: tokio::sync::mpsc::UnboundedReceiver<OperationSnapshot>,
    controller: &idea_shell::WorkbenchController,
    cx: &mut gpui_kit::App,
) {
    controller.set_action_dispatcher(dispatcher);
    let operation_sink = controller.operation_sink();
    cx.spawn(async move |cx| {
        while let Some(snapshot) = updates.recv().await {
            if !cx.update(|cx| operation_sink.set_operation_snapshot(&snapshot, cx)) {
                break;
            }
        }
    })
    .detach();
}

fn main() -> ExitCode {
    let smoke_duration = match smoke_duration_from_environment() {
        Ok(duration) => duration,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let startup_failed = Arc::new(AtomicBool::new(false));
    let startup_failed_in_app = Arc::clone(&startup_failed);
    let layout_path = desktop_platform::workbench_layout_path();
    let initial_layout = layout_path.as_deref().and_then(load_layout);
    let initial_repository = std::env::var_os(OPEN_REPOSITORY_ENV).map(PathBuf::from);
    let journal_path = desktop_platform::operation_journal_path();

    gpui_kit::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx| {
            gpui_kit::init(cx);

            let layout_io = idea_shell::WorkbenchLayoutIo::new(initial_layout, move |layout| {
                let Some(path) = layout_path.clone() else {
                    return;
                };
                std::thread::spawn(move || {
                    if let Err(error) = save_layout(&path, &layout) {
                        eprintln!("failed to persist IdeaGit workbench layout: {error}");
                    }
                });
            });
            let opened = match idea_shell::open_workbench_window(cx, layout_io) {
                Ok(opened) => opened,
                Err(error) => {
                    startup_failed_in_app.store(true, Ordering::Relaxed);
                    eprintln!("failed to open IdeaGit window: {error}");
                    cx.quit();
                    return;
                }
            };
            let window = opened.window;

            if let Some(path) = initial_repository {
                let controller = opened.controller;
                let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
                std::thread::spawn(move || {
                    let result = tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| format!("failed to start Git runtime: {error}"))
                        .and_then(|runtime| {
                            runtime.block_on(run_repository_bridge(path, journal_path, sender))
                        });
                    if let Err(error) = result {
                        eprintln!("IdeaGit repository bridge stopped: {error}");
                    }
                });
                cx.spawn(async move |cx| {
                    while let Some(update) = receiver.recv().await {
                        cx.update(|cx| match update {
                            DesktopUiUpdate::Repository(model) => {
                                controller.set_repository_model(model, cx);
                            }
                            DesktopUiUpdate::Log(page) => controller.set_log_page(page, cx),
                            DesktopUiUpdate::LogFailed(error) => {
                                controller.set_operation_feedback(
                                    idea_shell::OperationFeedback::Failed(format!(
                                        "读取 Git Log 失败：{error}"
                                    )),
                                    cx,
                                );
                            }
                            DesktopUiUpdate::RecoveryBlocked(reason) => {
                                controller.set_operation_feedback(
                                    idea_shell::OperationFeedback::Failed(format!(
                                        "写操作已禁用：{reason}"
                                    )),
                                    cx,
                                );
                            }
                            DesktopUiUpdate::WriteReady {
                                dispatcher,
                                updates,
                            } => install_prebuilt_action_dispatcher(
                                dispatcher,
                                updates,
                                &controller,
                                cx,
                            ),
                        });
                    }
                })
                .detach();
            }

            eprintln!("IdeaGit native window opened");
            cx.activate(true);

            if let Some(duration) = smoke_duration {
                let smoke_failed = Arc::clone(&startup_failed_in_app);
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(duration).await;
                    if let Err(error) = window.update(cx, |_, window, cx| {
                        window.draw(cx).clear(cx);
                        window.remove_window();
                        cx.quit();
                    }) {
                        smoke_failed.store(true, Ordering::Relaxed);
                        eprintln!("failed to close IdeaGit smoke window: {error}");
                        cx.update(|cx| cx.quit());
                    } else {
                        eprintln!("IdeaGit native window rendered and closed after smoke duration");
                    }
                })
                .detach();
            }
        });

    if startup_failed.load(Ordering::Relaxed) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

async fn run_repository_bridge(
    path: PathBuf,
    journal_path: Option<PathBuf>,
    sender: tokio::sync::mpsc::UnboundedSender<DesktopUiUpdate>,
) -> Result<(), String> {
    use git_ports::{HistoryReader as _, RepositoryReader as _};

    let discovery_cancellation = tokio_util::sync::CancellationToken::new();
    let (progress, _progress_receiver) = tokio::sync::mpsc::channel(16);
    let context = git_ports::ExecutionContext::new(
        git_domain::OperationId::new(),
        discovery_cancellation,
        progress,
    );
    let location = git_cli::GitCliRepositoryReader::new(git_cli::TokioProcessRunner)
        .discover(path, context)
        .await
        .map_err(|error| error.to_string())?;
    let registry = git_application::RepositoryRegistry::new();
    let handle = registry
        .open(location.clone())
        .map_err(|error| error.to_string())?;
    let session = registry
        .session(handle.target())
        .map_err(|error| error.to_string())?;
    let cancellation = registry
        .cancellation_token(handle.target())
        .map_err(|error| error.to_string())?;
    let recovery = inspect_startup_recovery(
        handle.target(),
        &location,
        journal_path.as_deref(),
        cancellation.clone(),
    )
    .await;
    let recovery_block = match recovery {
        Ok(report) if report.blocks_writes() => Some(format!(
            "检测到 {} 条需人工处理的旧日志和 {} 条未完成操作",
            report.orphaned.len(),
            report.inspections.len()
        )),
        Ok(_) => None,
        Err(error) => Some(error),
    };
    let write_enabled = recovery_block.is_none();
    if let Some(reason) = recovery_block
        && sender
            .send(DesktopUiUpdate::RecoveryBlocked(reason))
            .is_err()
    {
        return Ok(());
    }
    let scheduler = git_application::RefreshScheduler::start(
        git_application::SnapshotRefreshService::new(
            git_cli::GitCliHeadReader::new(git_cli::TokioProcessRunner),
            git_cli::GitCliStatusReader::new(git_cli::TokioProcessRunner),
        ),
        handle.target(),
        location.clone(),
        cancellation.clone(),
        git_application::RefreshSchedulerConfig::default(),
    )
    .map_err(|error| error.to_string())?;
    let mut refresh = scheduler.subscribe();
    if write_enabled {
        let journal_path = journal_path
            .clone()
            .ok_or_else(|| "operation Journal path disappeared after recovery".to_owned())?;
        let executor = build_m2_executor(
            handle.target(),
            location.clone(),
            journal_path,
            scheduler.rebuilder(),
        )
        .map_err(|error| error.to_string())?;
        let coordinator = Arc::new(
            OperationCoordinator::new(executor, 32)
                .map_err(|error| format!("failed to start operation coordinator: {error}"))?,
        );
        let (dispatcher, updates) =
            DesktopActionDispatcher::new(coordinator, tokio::runtime::Handle::current());
        if sender
            .send(DesktopUiUpdate::WriteReady {
                dispatcher: Arc::new(dispatcher),
                updates,
            })
            .is_err()
        {
            cancellation.cancel();
            scheduler
                .join()
                .await
                .map_err(|error| format!("repository refresh task failed: {error}"))?;
            drop(handle);
            return Ok(());
        }
    }
    let mut watcher = repository_watcher::RepositoryWatch::start(&location, 32)
        .map_err(|error| error.to_string())?;
    let history = git_cli::GitCliHistoryReader::new(git_cli::TokioProcessRunner, location);
    let mut last_log_head = None;
    let result = loop {
        let state = refresh.borrow().clone();
        let model = git_ui_model::RepositoryWorkbenchModel::single_session(&session, &state);
        if sender.send(DesktopUiUpdate::Repository(model)).is_err() {
            break Ok(());
        }
        if let Some(snapshot) = state.snapshot.as_ref()
            && last_log_head.as_ref() != Some(&snapshot.head)
        {
            last_log_head = Some(snapshot.head.clone());
            let page = if matches!(snapshot.head, git_domain::HeadState::Unborn { .. }) {
                Ok(git_domain::CommitPage {
                    target: handle.target(),
                    commits: Vec::new(),
                    has_more: false,
                })
            } else {
                let (progress, _progress_receiver) = tokio::sync::mpsc::channel(16);
                history
                    .log(
                        handle.target(),
                        git_domain::LogQuery::default(),
                        git_ports::ExecutionContext::new(
                            git_domain::OperationId::new(),
                            cancellation.clone(),
                            progress,
                        ),
                    )
                    .await
                    .map_err(|error| error.to_string())
            };
            let update = match page {
                Ok(page) => DesktopUiUpdate::Log(page),
                Err(error) => DesktopUiUpdate::LogFailed(error),
            };
            if sender.send(update).is_err() {
                break Ok(());
            }
        }
        tokio::select! {
            changed = refresh.changed() => {
                if changed.is_err() {
                    break Ok(());
                }
            }
            signal = watcher.recv() => match signal {
                Some(repository_watcher::WatchSignal::Changed) => {
                    scheduler.request(git_application::RefreshTrigger::FilesystemChanged);
                }
                Some(repository_watcher::WatchSignal::RescanRequired) | None => {
                    scheduler.request(git_application::RefreshTrigger::RescanRequired);
                }
            },
            () = sender.closed() => break Ok(()),
            () = cancellation.cancelled() => break Ok(()),
        }
    };
    cancellation.cancel();
    scheduler
        .join()
        .await
        .map_err(|error| format!("repository refresh task failed: {error}"))?;
    drop(handle);
    result
}

async fn inspect_startup_recovery(
    target: git_domain::SessionTarget,
    location: &git_ports::RepositoryLocation,
    journal_path: Option<&Path>,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<git_operations::StartupRecoveryReport, String> {
    let journal_path = journal_path.ok_or_else(|| "操作日志路径不可用".to_owned())?;
    let scope = location
        .recovery_scope()
        .map_err(|error| format!("无法构造恢复作用域：{error}"))?;
    let inspector = git_operations::StartupRecoveryInspector::new(
        persistence::JsonJournalStore::new(journal_path),
        DesktopCommitStateReader::new(
            target,
            git_cli::GitCliCommitStateProbe::new(git_cli::TokioProcessRunner, location.clone()),
        ),
    );
    let (progress, _progress_receiver) = tokio::sync::mpsc::channel(16);
    inspector
        .inspect(
            target,
            &scope,
            git_ports::ExecutionContext::new(
                git_domain::OperationId::new(),
                cancellation,
                progress,
            ),
        )
        .await
        .map_err(|error| format!("启动恢复检查失败：{error}"))
}

fn load_layout(path: &Path) -> Option<git_workbench::WorkbenchLayout> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            eprintln!("failed to read IdeaGit workbench layout: {error}");
            return None;
        }
    };
    match serde_json::from_slice::<git_workbench::WorkbenchLayout>(&bytes) {
        Ok(layout) if layout.validate().is_ok() => Some(layout),
        Ok(_) => {
            eprintln!("ignored incompatible IdeaGit workbench layout");
            None
        }
        Err(error) => {
            eprintln!("ignored invalid IdeaGit workbench layout: {error}");
            None
        }
    }
}

fn save_layout(path: &Path, layout: &git_workbench::WorkbenchLayout) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "layout path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec_pretty(layout).map_err(std::io::Error::other)?;
    let temporary = temporary_layout_path(path);
    fs::write(&temporary, bytes)?;
    #[cfg(target_os = "windows")]
    if path.exists() {
        fs::remove_file(path)?;
    }
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn temporary_layout_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(value)
}

fn smoke_duration_from_environment() -> Result<Option<Duration>, String> {
    match std::env::var(SMOKE_DURATION_ENV) {
        Ok(value) => parse_smoke_duration(Some(&value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(format!("{SMOKE_DURATION_ENV} must be valid UTF-8")),
    }
}

fn parse_smoke_duration(value: Option<&str>) -> Result<Option<Duration>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let millis = value
        .parse::<u64>()
        .map_err(|_| format!("{SMOKE_DURATION_ENV} must be a positive integer"))?;
    if millis == 0 {
        return Err(format!("{SMOKE_DURATION_ENV} must be greater than zero"));
    }
    Ok(Some(Duration::from_millis(millis)))
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use git_domain::{GitResult, RepoPath, RepositoryId, SessionTarget, WorktreeId};
    use git_ports::ExecutionContext;
    use git_ports::JournalStore as _;
    use git_ui_model::{ActionContext, ActionId, ActionPanel, ActionPayload};
    use tempfile::TempDir;

    use super::*;

    struct ImmediateExecutor;

    #[async_trait]
    impl OperationExecutor for ImmediateExecutor {
        async fn execute(
            &self,
            _command: git_domain::GitCommand,
            _context: ExecutionContext,
        ) -> GitResult<()> {
            Ok(())
        }
    }

    struct BlockingExecutor {
        started: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl OperationExecutor for BlockingExecutor {
        async fn execute(
            &self,
            _command: git_domain::GitCommand,
            _context: ExecutionContext,
        ) -> GitResult<()> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(())
        }
    }

    fn stage_request() -> ActionRequest {
        ActionRequest {
            action: ActionId::Stage,
            context: ActionContext {
                target: Some(SessionTarget::worktree(
                    RepositoryId::new(),
                    WorktreeId::new(),
                    1,
                )),
                selected_files: vec![RepoPath::try_from("file.txt").expect("valid test path")],
                selected_commits: Vec::new(),
                selected_branch: None,
                active_panel: ActionPanel::Commit,
            },
            payload: ActionPayload::None,
        }
    }

    #[test]
    fn parses_smoke_duration() {
        assert_eq!(
            parse_smoke_duration(Some("1500")),
            Ok(Some(Duration::from_millis(1500)))
        );
        assert_eq!(parse_smoke_duration(None), Ok(None));
    }

    #[test]
    fn rejects_invalid_smoke_duration() {
        assert!(parse_smoke_duration(Some("0")).is_err());
        assert!(parse_smoke_duration(Some("not-a-number")).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dispatcher_uses_injected_runtime_outside_tokio_context_and_forwards_terminal_state() {
        let coordinator = Arc::new(
            OperationCoordinator::new(ImmediateExecutor, 1).expect("valid coordinator capacity"),
        );
        let (dispatcher, mut updates) = DesktopActionDispatcher::new(
            Arc::clone(&coordinator),
            tokio::runtime::Handle::current(),
        );
        let dispatched = std::thread::spawn(move || dispatcher.dispatch(stage_request()))
            .join()
            .expect("UI-like caller thread should not panic");
        assert_eq!(dispatched, Ok(()));

        let terminal = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = updates
                    .recv()
                    .await
                    .expect("dispatcher should forward coordinator snapshots");
                if snapshot.phase == OperationPhase::Finished {
                    break snapshot;
                }
            }
        })
        .await
        .expect("terminal snapshot should be forwarded");
        assert_eq!(
            terminal.outcome,
            Some(git_operations::OperationOutcome::Success)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dispatcher_reports_bounded_queue_rejection_as_terminal_failure() {
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let coordinator = Arc::new(
            OperationCoordinator::new(
                BlockingExecutor {
                    started: Arc::clone(&started),
                    release: Arc::clone(&release),
                },
                1,
            )
            .expect("valid coordinator capacity"),
        );
        let (dispatcher, mut updates) =
            DesktopActionDispatcher::new(coordinator, tokio::runtime::Handle::current());
        dispatcher
            .dispatch(stage_request())
            .expect("first action should spawn submission");
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("first action should enter the executor");
        dispatcher
            .dispatch(stage_request())
            .expect("second action should spawn submission");
        dispatcher
            .dispatch(stage_request())
            .expect("third action should report asynchronously");

        let rejected = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = updates
                    .recv()
                    .await
                    .expect("dispatcher update stream should remain live");
                if let Some(git_operations::OperationOutcome::Failed(ref error)) = snapshot.outcome
                    && error.kind == git_domain::GitErrorKind::OperationAlreadyInProgress
                {
                    break snapshot;
                }
            }
        })
        .await
        .expect("queue rejection should reach the UI feedback stream");
        assert_eq!(rejected.phase, OperationPhase::Finished);
        release.notify_waiters();
    }

    #[tokio::test]
    async fn commit_state_reader_maps_probe_and_rejects_a_foreign_session() {
        let repository =
            git_fixture::TestRepository::init().expect("real Git fixture should initialize");
        repository
            .commit_file("tracked.txt", b"content\n")
            .expect("fixture commit should succeed");
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let reader = DesktopCommitStateReader::new(
            target,
            git_cli::GitCliCommitStateProbe::new(
                git_cli::TokioProcessRunner,
                git_ports::RepositoryLocation::worktree(
                    &repository.worktree,
                    repository.worktree.join(".git"),
                    repository.worktree.join(".git"),
                ),
            ),
        );
        let (progress, _receiver) = tokio::sync::mpsc::channel(4);
        let state = git_operations::CommitStateReader::read(
            &reader,
            target,
            ExecutionContext::new(
                git_domain::OperationId::new(),
                tokio_util::sync::CancellationToken::new(),
                progress,
            ),
        )
        .await
        .expect("matching session should be readable");
        assert!(state.head.is_some());
        assert!(state.index.is_some());

        let (progress, _receiver) = tokio::sync::mpsc::channel(4);
        let recovery = git_operations::RecoveryStateReader::read(
            &reader,
            target,
            ExecutionContext::new(
                git_domain::OperationId::new(),
                tokio_util::sync::CancellationToken::new(),
                progress,
            ),
        )
        .await
        .expect("recovery state should reuse the same authoritative probe");
        assert_eq!(recovery.head, state.head);
        assert_eq!(recovery.index, state.index);

        let foreign = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let (progress, _receiver) = tokio::sync::mpsc::channel(4);
        let error = git_operations::CommitStateReader::read(
            &reader,
            foreign,
            ExecutionContext::new(
                git_domain::OperationId::new(),
                tokio_util::sync::CancellationToken::new(),
                progress,
            ),
        )
        .await
        .expect_err("foreign session must be rejected before probing Git");
        assert_eq!(error.kind, git_domain::GitErrorKind::InvalidRepository);
        assert_eq!(error.target, Some(foreign));
    }

    #[tokio::test]
    async fn startup_recovery_matches_scope_after_registry_restart() {
        use git_operations::CommitStateReader as _;

        let repository =
            git_fixture::TestRepository::init().expect("real Git fixture should initialize");
        repository
            .commit_file("tracked.txt", b"content\n")
            .expect("fixture commit should succeed");
        let location = git_ports::RepositoryLocation::worktree(
            &repository.worktree,
            repository.worktree.join(".git"),
            repository.worktree.join(".git"),
        );
        let first_registry = git_application::RepositoryRegistry::new();
        let first_handle = first_registry
            .open(location.clone())
            .expect("first session should open");
        let old_target = first_handle.target();
        let reader = DesktopCommitStateReader::new(
            old_target,
            git_cli::GitCliCommitStateProbe::new(git_cli::TokioProcessRunner, location.clone()),
        );
        let (progress, _receiver) = tokio::sync::mpsc::channel(4);
        let state = reader
            .read(
                old_target,
                ExecutionContext::new(
                    git_domain::OperationId::new(),
                    tokio_util::sync::CancellationToken::new(),
                    progress,
                ),
            )
            .await
            .expect("pre-crash state should be readable");
        drop(first_handle);
        drop(first_registry);

        let temporary = TempDir::new().expect("journal directory");
        let journal_path = temporary.path().join("journal.json");
        let scope = location.recovery_scope().expect("recovery scope");
        let mut record = git_domain::JournalRecord::intent(
            git_domain::OperationId::new(),
            git_domain::GitCommand::StageFiles {
                target: old_target,
                paths: vec![RepoPath::try_from("tracked.txt").expect("fixture path")],
            },
            scope,
        );
        record.expected_head = state.head;
        record.expected_index = state.index;
        persistence::JsonJournalStore::new(&journal_path)
            .append(record)
            .await
            .expect("pending intent should persist");

        let second_registry = git_application::RepositoryRegistry::new();
        let second_handle = second_registry
            .open(location.clone())
            .expect("second session should open");
        let current_target = second_handle.target();
        assert_ne!(current_target, old_target);
        let report = inspect_startup_recovery(
            current_target,
            &location,
            Some(&journal_path),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("restart inspection should succeed");
        assert_eq!(report.inspections.len(), 1);
        assert_eq!(report.inspections[0].record.target, old_target);
        assert_eq!(report.inspections[0].current_target, current_target);
        assert_eq!(
            report.inspections[0].disposition,
            git_operations::RecoveryDisposition::NotExecuted
        );
        assert!(!report.blocks_writes());

        repository
            .write_file("external.txt", b"external\n")
            .expect("external file should be written");
        repository
            .stage_file("external.txt")
            .expect("external Index mutation should succeed");
        let changed = inspect_startup_recovery(
            current_target,
            &location,
            Some(&journal_path),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("changed-state inspection should succeed");
        assert_eq!(
            changed.inspections[0].disposition,
            git_operations::RecoveryDisposition::Unknown
        );
        assert!(changed.blocks_writes());
    }

    #[tokio::test]
    async fn startup_recovery_blocks_when_journal_is_missing_or_damaged() {
        let repository =
            git_fixture::TestRepository::init().expect("real Git fixture should initialize");
        let location = git_ports::RepositoryLocation::worktree(
            &repository.worktree,
            repository.worktree.join(".git"),
            repository.worktree.join(".git"),
        );
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let missing = inspect_startup_recovery(
            target,
            &location,
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect_err("missing durable path must block writes");
        assert!(missing.contains("路径不可用"));

        let temporary = TempDir::new().expect("journal directory");
        let journal_path = temporary.path().join("journal.json");
        fs::write(&journal_path, b"truncated").expect("damaged Journal fixture should be written");
        let damaged = inspect_startup_recovery(
            target,
            &location,
            Some(&journal_path),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect_err("damaged Journal must block writes");
        assert!(damaged.contains("启动恢复检查失败"));
    }

    #[tokio::test]
    async fn repository_bridge_reports_recovery_block_and_keeps_read_only_refresh_alive() {
        let repository =
            git_fixture::TestRepository::init().expect("real Git fixture should initialize");
        let temporary = TempDir::new().expect("journal directory");
        let journal_path = temporary.path().join("journal.json");
        fs::write(&journal_path, b"truncated").expect("damaged Journal fixture should be written");
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let bridge = tokio::spawn(run_repository_bridge(
            repository.worktree.clone(),
            Some(journal_path),
            sender,
        ));
        let (blocked, ready) = tokio::time::timeout(Duration::from_secs(10), async {
            let mut blocked = false;
            let mut ready = false;
            while !blocked || !ready {
                match receiver.recv().await.expect("bridge should remain live") {
                    DesktopUiUpdate::RecoveryBlocked(reason) => {
                        assert!(reason.contains("启动恢复检查失败"));
                        blocked = true;
                    }
                    DesktopUiUpdate::Repository(model)
                        if model.chrome.status.message == "Ready" =>
                    {
                        ready = true;
                    }
                    _ => {}
                }
            }
            (blocked, ready)
        })
        .await
        .expect("recovery feedback and read-only snapshot should arrive");
        assert!(blocked);
        assert!(ready);

        drop(receiver);
        tokio::time::timeout(Duration::from_secs(10), bridge)
            .await
            .expect("bridge should stop after UI receiver closes")
            .expect("bridge task should not panic")
            .expect("bridge should stop cleanly");
    }

    #[tokio::test]
    async fn repository_bridge_dispatches_a_real_stage_only_after_recovery_is_ready() {
        let repository =
            git_fixture::TestRepository::init().expect("real Git fixture should initialize");
        repository
            .commit_file("base.txt", b"base\n")
            .expect("base commit should succeed");
        repository
            .write_file("ui-stage.txt", b"stage through ActionDispatcher\n")
            .expect("worktree file should be written");
        let temporary = TempDir::new().expect("journal directory");
        let journal_path = temporary.path().join("journal.json");
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let bridge = tokio::spawn(run_repository_bridge(
            repository.worktree.clone(),
            Some(journal_path.clone()),
            sender,
        ));

        let (dispatcher, mut operation_updates, target) =
            tokio::time::timeout(Duration::from_secs(10), async {
                let mut dispatcher = None;
                let mut operation_updates = None;
                let mut target = None;
                loop {
                    match receiver.recv().await.expect("bridge should remain live") {
                        DesktopUiUpdate::WriteReady {
                            dispatcher: ready_dispatcher,
                            updates,
                        } => {
                            dispatcher = Some(ready_dispatcher);
                            operation_updates = Some(updates);
                        }
                        DesktopUiUpdate::Repository(model)
                            if model.chrome.status.message == "Ready" =>
                        {
                            target = model.chrome.tabs.first().map(|tab| tab.target);
                        }
                        DesktopUiUpdate::RecoveryBlocked(reason) => {
                            panic!("clean repository should not be recovery-blocked: {reason}")
                        }
                        _ => {}
                    }
                    match (dispatcher.take(), operation_updates.take(), target) {
                        (Some(dispatcher), Some(updates), Some(target)) => {
                            break (dispatcher, updates, target);
                        }
                        (pending_dispatcher, pending_updates, _) => {
                            dispatcher = pending_dispatcher;
                            operation_updates = pending_updates;
                        }
                    }
                }
            })
            .await
            .expect("write dispatcher and repository model should become ready");

        dispatcher
            .dispatch(ActionRequest {
                action: ActionId::Stage,
                context: ActionContext {
                    target: Some(target),
                    selected_files: vec![RepoPath::try_from("ui-stage.txt").expect("fixture path")],
                    selected_commits: Vec::new(),
                    selected_branch: None,
                    active_panel: ActionPanel::Commit,
                },
                payload: ActionPayload::None,
            })
            .expect("Stage action should queue");
        let terminal = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let snapshot = operation_updates
                    .recv()
                    .await
                    .expect("operation stream should remain live");
                if snapshot.phase == OperationPhase::Finished {
                    break snapshot;
                }
            }
        })
        .await
        .expect("Stage action should reach a terminal state");
        assert_eq!(
            terminal.outcome,
            Some(git_operations::OperationOutcome::Success)
        );

        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let DesktopUiUpdate::Repository(model) =
                    receiver.recv().await.expect("bridge should remain live")
                    && model
                        .changes
                        .group(git_ui_model::ChangeGroup::Staged)
                        .expect("staged group")
                        .entries
                        .iter()
                        .any(|entry| entry.path.display_lossy() == "ui-stage.txt")
                {
                    break;
                }
            }
        })
        .await
        .expect("reconciled staged state should reach the UI model");
        let record = persistence::JsonJournalStore::new(journal_path)
            .get(terminal.id)
            .await
            .expect("Journal should remain readable")
            .expect("operation should be journaled");
        assert_eq!(record.result, Some(git_domain::JournalResult::Completed));

        drop(dispatcher);
        drop(operation_updates);
        drop(receiver);
        tokio::time::timeout(Duration::from_secs(10), bridge)
            .await
            .expect("bridge should stop after UI receiver closes")
            .expect("bridge task should not panic")
            .expect("bridge should stop cleanly");
    }

    #[tokio::test]
    async fn production_m2_executor_stages_journals_and_reconciles_a_real_repository() {
        use git_operations::OperationExecutor as _;

        let repository =
            git_fixture::TestRepository::init().expect("real Git fixture should initialize");
        repository
            .write_file("stage.txt", b"staged content\n")
            .expect("fixture file should be written");
        let location = git_ports::RepositoryLocation::worktree(
            &repository.worktree,
            repository.worktree.join(".git"),
            repository.worktree.join(".git"),
        );
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let cancellation = tokio_util::sync::CancellationToken::new();
        let scheduler = git_application::RefreshScheduler::start(
            git_application::SnapshotRefreshService::new(
                git_cli::GitCliHeadReader::new(git_cli::TokioProcessRunner),
                git_cli::GitCliStatusReader::new(git_cli::TokioProcessRunner),
            ),
            target,
            location.clone(),
            cancellation.clone(),
            git_application::RefreshSchedulerConfig::default(),
        )
        .expect("refresh scheduler should start");
        let mut refresh = scheduler.subscribe();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if refresh.borrow().health == git_application::RefreshHealth::Ready {
                    break;
                }
                refresh
                    .changed()
                    .await
                    .expect("refresh scheduler should remain live");
            }
        })
        .await
        .expect("initial snapshot should become ready");

        let temporary = TempDir::new().expect("temporary directory should be created");
        let journal_path = temporary.path().join("operation-journal.json");
        let executor = build_m2_executor(
            target,
            location,
            journal_path.clone(),
            scheduler.rebuilder(),
        )
        .expect("fixture should have a recovery scope");
        let operation_id = git_domain::OperationId::new();
        let (progress, _receiver) = tokio::sync::mpsc::channel(8);
        executor
            .execute(
                git_domain::GitCommand::StageFiles {
                    target,
                    paths: vec![RepoPath::try_from("stage.txt").expect("valid fixture path")],
                },
                ExecutionContext::new(
                    operation_id,
                    tokio_util::sync::CancellationToken::new(),
                    progress,
                ),
            )
            .await
            .expect("full writer stack should stage and reconcile");

        let snapshot = refresh
            .borrow()
            .snapshot
            .clone()
            .expect("reconcile should publish a snapshot");
        assert!(
            snapshot
                .changes
                .iter()
                .any(|change| { change.path.display_lossy() == "stage.txt" && change.is_staged() })
        );
        let record = persistence::JsonJournalStore::new(journal_path)
            .get(operation_id)
            .await
            .expect("Journal should remain readable")
            .expect("operation record should exist");
        assert_eq!(record.phase, git_domain::JournalPhase::Completed);
        assert_eq!(record.result, Some(git_domain::JournalResult::Completed));
        assert!(record.expected_head.is_none());
        assert!(record.expected_index.is_some());

        cancellation.cancel();
        scheduler
            .join()
            .await
            .expect("refresh scheduler should stop cleanly");
    }

    #[test]
    fn layout_storage_round_trips_and_replaces_previous_state() {
        let temporary = TempDir::new().expect("temporary directory should be created");
        let path = temporary.path().join("nested/workbench.json");
        let first = git_workbench::WorkbenchLayout::new("main", serde_json::json!({"value": 1}));
        save_layout(&path, &first).expect("first layout should save");
        assert_eq!(load_layout(&path), Some(first));

        let second = git_workbench::WorkbenchLayout::new("main", serde_json::json!({"value": 2}));
        save_layout(&path, &second).expect("replacement layout should save");
        assert_eq!(load_layout(&path), Some(second));
        assert!(!temporary_layout_path(&path).exists());
    }

    #[test]
    fn invalid_or_missing_layout_falls_back_without_panicking() {
        let temporary = TempDir::new().expect("temporary directory should be created");
        let path = temporary.path().join("workbench.json");
        assert_eq!(load_layout(&path), None);
        fs::write(&path, b"not json").expect("invalid fixture should be written");
        assert_eq!(load_layout(&path), None);
    }

    #[tokio::test]
    async fn external_git_edit_refreshes_the_workbench_changes_model() {
        let repository =
            git_fixture::TestRepository::init().expect("real Git fixture should initialize");
        repository
            .commit_file("tracked.txt", b"content\n")
            .expect("fixture commit should succeed");
        repository
            .write_file("unstaged.txt", b"worktree\n")
            .expect("worktree file should be written");
        repository
            .stage_file("unstaged.txt")
            .expect("new file should be staged");
        repository
            .write_file("mixed.txt", b"index version\n")
            .expect("mixed file should be written");
        repository
            .stage_file("mixed.txt")
            .expect("mixed file should be staged");
        repository
            .write_file("mixed.txt", b"worktree version\n")
            .expect("mixed file should change after staging");
        repository
            .write_file("untracked.txt", b"untracked\n")
            .expect("untracked file should be written");

        let journal = TempDir::new().expect("journal directory");
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let bridge = tokio::spawn(run_repository_bridge(
            repository.worktree.clone(),
            Some(journal.path().join("journal.json")),
            sender,
        ));
        let model = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let update = receiver
                    .recv()
                    .await
                    .expect("bridge should publish a model");
                if let DesktopUiUpdate::Repository(model) = update
                    && model.chrome.status.message == "Ready"
                {
                    break model;
                }
            }
        })
        .await
        .expect("initial snapshot should reach the workbench");
        assert_eq!(model.chrome.tabs.len(), 1);
        assert_eq!(model.chrome.tabs[0].title, "worktree");
        assert_eq!(model.chrome.tabs[0].branch, "main");
        assert_eq!(model.chrome.status.message, "Ready");
        assert!(model.chrome.status.revision.is_some());
        let paths = |group| {
            model
                .changes
                .group(group)
                .expect("fixed group should exist")
                .entries
                .iter()
                .map(|entry| entry.path.display_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            paths(git_ui_model::ChangeGroup::Staged),
            vec!["mixed.txt", "unstaged.txt"]
        );
        assert_eq!(
            paths(git_ui_model::ChangeGroup::Unstaged),
            vec!["mixed.txt"]
        );
        assert_eq!(
            paths(git_ui_model::ChangeGroup::Untracked),
            vec!["untracked.txt"]
        );
        repository
            .write_file("external.txt", b"external edit\n")
            .expect("external Git edit should be written");
        let refreshed = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let update = receiver.recv().await.expect("bridge should remain live");
                if let DesktopUiUpdate::Repository(model) = update
                    && model
                        .changes
                        .group(git_ui_model::ChangeGroup::Untracked)
                        .expect("fixed group should exist")
                        .entries
                        .iter()
                        .any(|entry| entry.path.display_lossy() == "external.txt")
                {
                    break model;
                }
            }
        })
        .await
        .expect("watcher refresh should reach the workbench");
        assert_eq!(refreshed.chrome.status.message, "Ready");
        drop(receiver);
        tokio::time::timeout(Duration::from_secs(10), bridge)
            .await
            .expect("bridge should stop after UI receiver closes")
            .expect("bridge task should not panic")
            .expect("bridge should stop cleanly");
    }

    #[tokio::test]
    async fn repository_bridge_publishes_log_again_after_external_commit() {
        let repository =
            git_fixture::TestRepository::init().expect("real Git fixture should initialize");
        repository
            .commit_file("one.txt", b"one\n")
            .expect("first fixture commit should succeed");
        let journal = TempDir::new().expect("journal directory");
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let bridge = tokio::spawn(run_repository_bridge(
            repository.worktree.clone(),
            Some(journal.path().join("journal.json")),
            sender,
        ));

        let first = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match receiver.recv().await.expect("bridge should remain live") {
                    DesktopUiUpdate::Log(page) if page.commits.len() == 1 => break page,
                    DesktopUiUpdate::LogFailed(error) => panic!("log should load: {error}"),
                    _ => {}
                }
            }
        })
        .await
        .expect("initial log should reach the workbench");
        assert_eq!(first.commits[0].subject, b"fixture commit");
        let first_commit = first.commits[0].id;

        repository
            .commit_file("two.txt", b"two\n")
            .expect("external fixture commit should succeed");
        let refreshed = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match receiver.recv().await.expect("bridge should remain live") {
                    DesktopUiUpdate::Log(page) if page.commits.len() == 2 => break page,
                    DesktopUiUpdate::LogFailed(error) => {
                        panic!("refreshed log should load: {error}")
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("updated log should reach the workbench");
        assert_ne!(refreshed.commits[0].id, first_commit);
        assert_eq!(refreshed.commits[1].id, first_commit);

        drop(receiver);
        tokio::time::timeout(Duration::from_secs(10), bridge)
            .await
            .expect("bridge should stop after UI receiver closes")
            .expect("bridge task should not panic")
            .expect("bridge should stop cleanly");
    }
}
