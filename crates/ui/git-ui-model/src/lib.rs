//! Shared UI projections, selections, actions, and asynchronous request identities.

use git_application::{RefreshHealth, RefreshState, RepositorySessionView};
use git_domain::{HeadState, SessionTarget, SnapshotRevision};

/// One repository/worktree tab rendered by the workbench shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryTabModel {
    pub target: SessionTarget,
    pub title: String,
    pub branch: String,
    pub active: bool,
    pub health: RefreshHealth,
}

/// Compact Git state shown in the bottom status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryStatusModel {
    pub branch: String,
    pub sync: String,
    pub revision: Option<SnapshotRevision>,
    pub message: String,
    pub health: RefreshHealth,
}

/// Current repository projection for a single workbench window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryChromeModel {
    pub tabs: Vec<RepositoryTabModel>,
    pub active_target: Option<SessionTarget>,
    pub status: RepositoryStatusModel,
}

impl Default for RepositoryChromeModel {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_target: None,
            status: RepositoryStatusModel {
                branch: "No branch".to_owned(),
                sync: String::new(),
                revision: None,
                message: "Ready".to_owned(),
                health: RefreshHealth::Pending,
            },
        }
    }
}

impl RepositoryChromeModel {
    /// Projects one live session and its latest-only refresh state into window chrome.
    #[must_use]
    pub fn single_session(session: &RepositorySessionView, refresh: &RefreshState) -> Self {
        let target = session.target();
        let title = session
            .location()
            .worktree_root()
            .unwrap_or_else(|| session.location().git_dir())
            .file_name()
            .map_or_else(
                || "Repository".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
        let (branch, sync, revision) = refresh.snapshot.as_ref().map_or_else(
            || ("Loading…".to_owned(), String::new(), None),
            |snapshot| {
                (
                    head_label(&snapshot.head),
                    sync_label(snapshot.ahead, snapshot.behind),
                    Some(snapshot.revision),
                )
            },
        );
        let message = match refresh.health {
            RefreshHealth::Pending => "Refreshing…",
            RefreshHealth::Ready => "Ready",
            RefreshHealth::Degraded => "Refresh failed",
        }
        .to_owned();
        Self {
            tabs: vec![RepositoryTabModel {
                target,
                title,
                branch: branch.clone(),
                active: true,
                health: refresh.health,
            }],
            active_target: Some(target),
            status: RepositoryStatusModel {
                branch,
                sync,
                revision,
                message,
                health: refresh.health,
            },
        }
    }
}

fn head_label(head: &HeadState) -> String {
    match head {
        HeadState::Unborn { branch } | HeadState::Attached { branch, .. } => {
            branch.display_lossy().into_owned()
        }
        HeadState::Detached { oid } => format!("Detached @ {oid}"),
    }
}

fn sync_label(ahead: Option<u64>, behind: Option<u64>) -> String {
    match (ahead, behind) {
        (Some(ahead), Some(behind)) => format!("↑{ahead} ↓{behind}"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use git_application::RefreshState;
    use git_application::RepositoryRegistry;
    use git_domain::{
        ObjectFormat, ObjectId, RefName, RepositorySnapshot, SnapshotConsistency,
        WorktreeSnapshotState,
    };
    use git_ports::RepositoryLocation;

    use super::*;

    fn oid() -> ObjectId {
        ObjectId::from_bytes(ObjectFormat::Sha1, &[1; 20]).expect("test OID should be valid")
    }

    #[test]
    fn attached_snapshot_projects_tab_branch_sync_and_revision() {
        let registry = RepositoryRegistry::new();
        let handle = registry
            .open(RepositoryLocation::worktree(
                "/repo/project",
                "/repo/project/.git",
                "/repo/project/.git",
            ))
            .expect("session should open");
        let session = registry
            .session(handle.target())
            .expect("session should exist");
        let snapshot = RepositorySnapshot {
            repository: handle.target().repository(),
            worktree: handle.target().worktree_id(),
            revision: SnapshotRevision::new(3),
            head: HeadState::Attached {
                branch: RefName::new(b"main".to_vec()).expect("branch should be valid"),
                oid: oid(),
            },
            operation: None,
            changes: Vec::new(),
            untracked: Vec::new(),
            ahead: Some(2),
            behind: Some(1),
            worktree_state: WorktreeSnapshotState::Available,
            consistency: SnapshotConsistency::Consistent,
        };
        let refresh = RefreshState {
            target: handle.target(),
            health: RefreshHealth::Ready,
            snapshot: Some(Arc::new(snapshot)),
            last_error: None,
        };

        let model = RepositoryChromeModel::single_session(&session, &refresh);
        assert_eq!(model.tabs[0].title, "project");
        assert_eq!(model.tabs[0].branch, "main");
        assert_eq!(model.status.sync, "↑2 ↓1");
        assert_eq!(model.status.revision, Some(SnapshotRevision::new(3)));
    }

    #[test]
    fn pending_and_degraded_states_never_claim_ready() {
        let registry = RepositoryRegistry::new();
        let handle = registry
            .open(RepositoryLocation::bare("repo.git", "repo.git"))
            .expect("bare session should open");
        let session = registry
            .session(handle.target())
            .expect("session should exist");
        for (health, expected) in [
            (RefreshHealth::Pending, "Refreshing…"),
            (RefreshHealth::Degraded, "Refresh failed"),
        ] {
            let refresh = RefreshState {
                target: handle.target(),
                health,
                snapshot: None,
                last_error: None,
            };
            assert_eq!(
                RepositoryChromeModel::single_session(&session, &refresh)
                    .status
                    .message,
                expected
            );
        }
    }
}
