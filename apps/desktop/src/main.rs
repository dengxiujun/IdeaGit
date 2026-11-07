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
                let (sender, receiver) = tokio::sync::oneshot::channel();
                std::thread::spawn(move || {
                    let result = tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| format!("failed to start Git runtime: {error}"))
                        .and_then(|runtime| runtime.block_on(load_repository_chrome(path)));
                    let _ = sender.send(result);
                });
                cx.spawn(async move |cx| match receiver.await {
                    Ok(Ok(model)) => {
                        cx.update(|cx| controller.set_repository_chrome(model, cx));
                    }
                    Ok(Err(error)) => eprintln!("failed to open IdeaGit repository: {error}"),
                    Err(_) => eprintln!("IdeaGit repository loader stopped unexpectedly"),
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

async fn load_repository_chrome(
    path: PathBuf,
) -> Result<git_ui_model::RepositoryChromeModel, String> {
    use git_ports::RepositoryReader as _;

    let cancellation = tokio_util::sync::CancellationToken::new();
    let (progress, _progress_receiver) = tokio::sync::mpsc::channel(16);
    let context = git_ports::ExecutionContext::new(
        git_domain::OperationId::new(),
        cancellation.clone(),
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
    let snapshot = git_application::SnapshotRefreshService::new(
        git_cli::GitCliHeadReader::new(git_cli::TokioProcessRunner),
        git_cli::GitCliStatusReader::new(git_cli::TokioProcessRunner),
    )
    .refresh(
        handle.target(),
        &location,
        git_domain::SnapshotRevision::INITIAL,
        cancellation,
    )
    .await
    .map_err(|error| error.to_string())?;
    let refresh = git_application::RefreshState {
        target: handle.target(),
        health: git_application::RefreshHealth::Ready,
        snapshot: Some(Arc::new(snapshot)),
        last_error: None,
    };
    Ok(git_ui_model::RepositoryChromeModel::single_session(
        &session, &refresh,
    ))
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
    async fn real_repository_projects_current_branch_into_workbench_chrome() {
        let repository =
            git_fixture::TestRepository::init().expect("real Git fixture should initialize");
        repository
            .commit_file("tracked.txt", b"content\n")
            .expect("fixture commit should succeed");

        let model = load_repository_chrome(repository.worktree.clone())
            .await
            .expect("repository should open and refresh");
        assert_eq!(model.tabs.len(), 1);
        assert_eq!(model.tabs[0].title, "worktree");
        assert_eq!(model.tabs[0].branch, "main");
        assert_eq!(model.status.message, "Ready");
        assert!(model.status.revision.is_some());
    }
}
