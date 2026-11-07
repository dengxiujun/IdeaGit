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

#[derive(Debug)]
enum DesktopUiUpdate {
    Repository(git_ui_model::RepositoryWorkbenchModel),
    Log(git_domain::CommitPage),
    LogFailed(String),
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
        let coordinator = Arc::clone(&self.coordinator);
        let updates = self.updates.clone();
        self.runtime.spawn(async move {
            let id = match coordinator.submit(command).await {
                Ok(id) => id,
                Err(error) => {
                    tracing::error!(%error, "failed to queue desktop Git action");
                    return;
                }
            };
            let Some(mut snapshots) = coordinator.subscribe(id).await else {
                tracing::error!(%id, "queued desktop Git action has no snapshot stream");
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
    let (dispatcher, mut updates) = DesktopActionDispatcher::new(coordinator, runtime);
    let dispatcher = Arc::new(dispatcher);
    controller.set_action_dispatcher(dispatcher.clone());
    let operation_sink = controller.operation_sink();
    cx.spawn(async move |cx| {
        while let Some(snapshot) = updates.recv().await {
            if !cx.update(|cx| operation_sink.set_operation_snapshot(&snapshot, cx)) {
                break;
            }
        }
    })
    .detach();
    dispatcher
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
                        .and_then(|runtime| runtime.block_on(run_repository_bridge(path, sender)));
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

    #[tokio::test]
    async fn commit_state_reader_maps_probe_and_rejects_a_foreign_session() {
        use git_operations::CommitStateReader as _;

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
        let state = reader
            .read(
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

        let foreign = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let (progress, _receiver) = tokio::sync::mpsc::channel(4);
        let error = reader
            .read(
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

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let bridge = tokio::spawn(run_repository_bridge(repository.worktree.clone(), sender));
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
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let bridge = tokio::spawn(run_repository_bridge(repository.worktree.clone(), sender));

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
