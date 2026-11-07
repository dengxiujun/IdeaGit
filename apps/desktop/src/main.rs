//! Composition root for application services, adapters, platform, shell, and feature views.
//!
//! UI-001 initializes GPUI-Kit and opens only the minimal shell window. Git services and the
//! production workbench remain intentionally uninitialized until their tracked tasks.

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
                    while let Some(model) = receiver.recv().await {
                        cx.update(|cx| controller.set_repository_model(model, cx));
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
    sender: tokio::sync::mpsc::UnboundedSender<git_ui_model::RepositoryWorkbenchModel>,
) -> Result<(), String> {
    use git_ports::RepositoryReader as _;

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
    let result = loop {
        let model =
            git_ui_model::RepositoryWorkbenchModel::single_session(&session, &refresh.borrow());
        if sender.send(model).is_err() {
            break Ok(());
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
    use tempfile::TempDir;

    use super::*;

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
                let model = receiver
                    .recv()
                    .await
                    .expect("bridge should publish a model");
                if model.chrome.status.message == "Ready" {
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
                let model = receiver.recv().await.expect("bridge should remain live");
                let untracked = model
                    .changes
                    .group(git_ui_model::ChangeGroup::Untracked)
                    .expect("fixed group should exist");
                if untracked
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
}
