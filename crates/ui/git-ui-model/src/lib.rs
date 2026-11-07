//! Shared UI projections, selections, actions, and asynchronous request identities.

use git_application::{RefreshHealth, RefreshState, RepositorySessionView};
use git_domain::{
    BranchCommand, ChangeKind, GitCommand, HeadState, RepoPath, RepositorySnapshot, SessionTarget,
    SnapshotRevision, StandardCommitRequest,
};

/// Immutable source selected by a read-only change-tree entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeContentSource {
    /// The Index differs from `HEAD`.
    Index,
    /// The worktree differs from the Index.
    Worktree,
}

/// A top-level section of the read-only Changes tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeGroup {
    Staged,
    Unstaged,
    Untracked,
}

impl ChangeGroup {
    /// Localized section title rendered by the first workbench slice.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Staged => "Staged",
            Self::Unstaged => "Unstaged",
            Self::Untracked => "Untracked",
        }
    }
}

/// One file entry in a Changes tree section.
///
/// The raw [`RepoPath`] is retained so later selection and action slices do not have to recover
/// repository identity from a lossy display string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeTreeEntry {
    pub path: RepoPath,
    pub original_path: Option<RepoPath>,
    pub source: ChangeContentSource,
    pub change_kind: ChangeKind,
    pub conflicted: bool,
}

/// One stable group and its Git-output-order entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeTreeGroup {
    pub kind: ChangeGroup,
    pub entries: Vec<ChangeTreeEntry>,
}

/// Read-only projection of one immutable repository snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangesTreeModel {
    pub target: Option<SessionTarget>,
    pub revision: Option<SnapshotRevision>,
    pub groups: Vec<ChangeTreeGroup>,
}

impl Default for ChangesTreeModel {
    fn default() -> Self {
        Self {
            target: None,
            revision: None,
            groups: [
                ChangeGroup::Staged,
                ChangeGroup::Unstaged,
                ChangeGroup::Untracked,
            ]
            .into_iter()
            .map(|kind| ChangeTreeGroup {
                kind,
                entries: Vec::new(),
            })
            .collect(),
        }
    }
}

impl ChangesTreeModel {
    /// Projects Index, worktree, and untracked state without changing any Git state.
    ///
    /// A path modified on both sides deliberately occurs in both the Staged and Unstaged groups:
    /// they compare different Git content sources and are not interchangeable check states.
    #[must_use]
    pub fn from_snapshot(target: SessionTarget, snapshot: &RepositorySnapshot) -> Self {
        let mut model = Self {
            target: Some(target),
            revision: Some(snapshot.revision),
            ..Self::default()
        };
        for change in &snapshot.changes {
            if change.is_staged()
                && let Some(group) = model.group_mut(ChangeGroup::Staged)
            {
                group.entries.push(ChangeTreeEntry {
                    path: change.path.clone(),
                    original_path: change.original_path.clone(),
                    source: ChangeContentSource::Index,
                    change_kind: change.index_status,
                    conflicted: change.is_conflicted(),
                });
            }
            if change.is_unstaged()
                && let Some(group) = model.group_mut(ChangeGroup::Unstaged)
            {
                group.entries.push(ChangeTreeEntry {
                    path: change.path.clone(),
                    original_path: change.original_path.clone(),
                    source: ChangeContentSource::Worktree,
                    change_kind: change.worktree_status,
                    conflicted: change.is_conflicted(),
                });
            }
        }
        if let Some(untracked) = model.group_mut(ChangeGroup::Untracked) {
            untracked
                .entries
                .extend(
                    snapshot
                        .untracked
                        .iter()
                        .cloned()
                        .map(|path| ChangeTreeEntry {
                            path,
                            original_path: None,
                            source: ChangeContentSource::Worktree,
                            change_kind: ChangeKind::Added,
                            conflicted: false,
                        }),
                );
        }
        model
    }

    #[must_use]
    pub fn group(&self, kind: ChangeGroup) -> Option<&ChangeTreeGroup> {
        self.groups.iter().find(|group| group.kind == kind)
    }

    fn group_mut(&mut self, kind: ChangeGroup) -> Option<&mut ChangeTreeGroup> {
        self.groups.iter_mut().find(|group| group.kind == kind)
    }
}

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

/// All first-slice repository data consumed by the Workbench window.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RepositoryWorkbenchModel {
    pub chrome: RepositoryChromeModel,
    pub changes: ChangesTreeModel,
}

/// User intent routed through the workbench action registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionId {
    Stage,
    Unstage,
    Commit,
    CreateBranch,
    CheckoutBranch,
}

/// UI panel identity used by action policy without coupling the projection crate to GPUI panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionPanel {
    RepositoryExplorer,
    GitLog,
    Commit,
    Diff,
    Console,
}

/// Snapshot of the selection and target captured immediately before dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionContext {
    pub target: Option<SessionTarget>,
    pub selected_files: Vec<RepoPath>,
    pub selected_commits: Vec<git_domain::ObjectId>,
    pub selected_branch: Option<git_domain::RefName>,
    pub active_panel: ActionPanel,
}

impl Default for ActionContext {
    fn default() -> Self {
        Self {
            target: None,
            selected_files: Vec::new(),
            selected_commits: Vec::new(),
            selected_branch: None,
            active_panel: ActionPanel::RepositoryExplorer,
        }
    }
}

/// Whether an action may be presented and invoked for the current context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionAvailability {
    Enabled,
    Disabled(String),
    Hidden,
}

/// Policy shared by buttons, menus, and shortcuts.
pub trait ActionPolicy {
    fn evaluate(&self, action: ActionId, context: &ActionContext) -> ActionAvailability;
}

/// Validated intent handed from UI controls to the application composition root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRequest {
    pub action: ActionId,
    pub context: ActionContext,
    pub payload: ActionPayload,
}

/// Typed parameters required by actions whose intent cannot be inferred from selection alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionPayload {
    None,
    Commit(StandardCommitRequest),
    Branch(BranchCommand),
}

/// Application-owned boundary for queuing a typed action.
pub trait ActionDispatcher: Send + Sync {
    fn dispatch(&self, request: ActionRequest) -> Result<(), String>;
}

/// Small composition-root adapter for forwarding actions to a background consumer.
#[derive(Clone)]
pub struct ChannelActionDispatcher {
    sender: std::sync::mpsc::Sender<ActionRequest>,
}

impl ChannelActionDispatcher {
    #[must_use]
    pub fn new(sender: std::sync::mpsc::Sender<ActionRequest>) -> Self {
        Self { sender }
    }
}

impl ActionDispatcher for ChannelActionDispatcher {
    fn dispatch(&self, request: ActionRequest) -> Result<(), String> {
        self.sender
            .send(request)
            .map_err(|_| "action consumer is unavailable".to_owned())
    }
}

/// Converts the file-based first-slice actions into the shared Git command contract.
pub fn to_git_command(request: &ActionRequest) -> Result<GitCommand, String> {
    let target = request
        .context
        .target
        .ok_or_else(|| "action target is missing".to_owned())?;
    if request.context.selected_files.is_empty() {
        return Err("action requires at least one selected file".to_owned());
    }
    match request.action {
        ActionId::Stage => Ok(GitCommand::StageFiles {
            target,
            paths: request.context.selected_files.clone(),
        }),
        ActionId::Unstage => Ok(GitCommand::UnstageFiles {
            target,
            paths: request.context.selected_files.clone(),
        }),
        ActionId::Commit => match &request.payload {
            ActionPayload::Commit(commit) => Ok(GitCommand::CommitIndex {
                target,
                request: commit.clone(),
            }),
            _ => Err("commit action requires typed commit parameters".to_owned()),
        },
        ActionId::CreateBranch | ActionId::CheckoutBranch => match &request.payload {
            ActionPayload::Branch(command) => Ok(GitCommand::Branch {
                target,
                command: command.clone(),
            }),
            _ => Err("branch action requires a typed branch command".to_owned()),
        },
    }
}

/// Conservative default policy for the first write-action slice.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultActionPolicy;

impl ActionPolicy for DefaultActionPolicy {
    fn evaluate(&self, action: ActionId, context: &ActionContext) -> ActionAvailability {
        if context.target.is_none() {
            return ActionAvailability::Disabled("打开仓库后才能执行此操作".to_owned());
        }
        match action {
            ActionId::Stage | ActionId::Unstage => {
                if context.selected_files.is_empty() {
                    ActionAvailability::Disabled("请选择至少一个文件".to_owned())
                } else {
                    ActionAvailability::Enabled
                }
            }
            ActionId::Commit => {
                if context.selected_files.is_empty() {
                    ActionAvailability::Disabled("没有可提交的文件".to_owned())
                } else {
                    ActionAvailability::Enabled
                }
            }
            ActionId::CreateBranch => ActionAvailability::Enabled,
            ActionId::CheckoutBranch => {
                if context.selected_branch.is_some() {
                    ActionAvailability::Enabled
                } else {
                    ActionAvailability::Disabled("请选择目标分支".to_owned())
                }
            }
        }
    }
}

impl RepositoryWorkbenchModel {
    /// Projects one session's latest refresh state into all currently visible read-only regions.
    #[must_use]
    pub fn single_session(session: &RepositorySessionView, refresh: &RefreshState) -> Self {
        let target = session.target();
        let changes = refresh
            .snapshot
            .as_ref()
            .map_or_else(ChangesTreeModel::default, |snapshot| {
                ChangesTreeModel::from_snapshot(target, snapshot)
            });
        Self {
            chrome: RepositoryChromeModel::single_session(session, refresh),
            changes,
        }
    }
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
        ChangeKind, FileChange, ObjectFormat, ObjectId, RefName, RepoPath, RepositorySnapshot,
        SnapshotConsistency, SubmoduleState, WorktreeSnapshotState,
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

    #[test]
    fn changes_tree_keeps_mixed_paths_in_both_content_source_groups() {
        let target = SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        );
        let mixed = RepoPath::try_from("mixed.txt").expect("valid repository path");
        let unstaged = RepoPath::try_from("unstaged.txt").expect("valid repository path");
        let untracked = RepoPath::try_from("untracked.txt").expect("valid repository path");
        let change = |path, index_status, worktree_status| FileChange {
            path,
            original_path: None,
            index_status,
            worktree_status,
            head_oid: None,
            index_oid: None,
            head_mode: None,
            index_mode: None,
            worktree_mode: None,
            submodule: SubmoduleState::NOT_SUBMODULE,
            similarity: None,
            conflict: None,
        };
        let snapshot = RepositorySnapshot {
            repository: target.repository(),
            worktree: target.worktree_id(),
            revision: SnapshotRevision::new(7),
            head: HeadState::Detached { oid: oid() },
            operation: None,
            changes: vec![
                change(mixed.clone(), ChangeKind::Modified, ChangeKind::Modified),
                change(unstaged, ChangeKind::Unmodified, ChangeKind::Deleted),
            ],
            untracked: vec![untracked],
            ahead: None,
            behind: None,
            worktree_state: WorktreeSnapshotState::Available,
            consistency: SnapshotConsistency::Consistent,
        };

        let model = ChangesTreeModel::from_snapshot(target, &snapshot);
        let paths = |group| {
            model
                .group(group)
                .expect("fixed group should exist")
                .entries
                .iter()
                .map(|entry| entry.path.display_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(ChangeGroup::Staged), vec!["mixed.txt"]);
        assert_eq!(
            paths(ChangeGroup::Unstaged),
            vec!["mixed.txt", "unstaged.txt"]
        );
        assert_eq!(paths(ChangeGroup::Untracked), vec!["untracked.txt"]);
        assert_eq!(
            model
                .group(ChangeGroup::Staged)
                .expect("fixed group should exist")
                .entries[0]
                .source,
            ChangeContentSource::Index
        );
    }

    #[test]
    fn default_action_policy_requires_target_and_selection() {
        let policy = DefaultActionPolicy;
        let empty = ActionContext::default();
        assert_eq!(
            policy.evaluate(ActionId::Stage, &empty),
            ActionAvailability::Disabled("打开仓库后才能执行此操作".to_owned())
        );

        let target = SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        );
        let context = ActionContext {
            target: Some(target),
            ..Default::default()
        };
        assert_eq!(
            policy.evaluate(ActionId::Stage, &context),
            ActionAvailability::Disabled("请选择至少一个文件".to_owned())
        );
        assert_eq!(
            policy.evaluate(ActionId::CreateBranch, &context),
            ActionAvailability::Enabled
        );
    }

    #[test]
    fn checkout_requires_a_branch_but_commit_requires_files() {
        let policy = DefaultActionPolicy;
        let target = SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        );
        let path = RepoPath::try_from("src/lib.rs").expect("valid repository path");
        let context = ActionContext {
            target: Some(target),
            selected_files: vec![path],
            ..Default::default()
        };
        assert_eq!(
            policy.evaluate(ActionId::Commit, &context),
            ActionAvailability::Enabled
        );
        assert!(matches!(
            policy.evaluate(ActionId::CheckoutBranch, &context),
            ActionAvailability::Disabled(_)
        ));
    }

    #[test]
    fn file_actions_convert_to_shared_git_commands() {
        let target = SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        );
        let request = ActionRequest {
            action: ActionId::Stage,
            context: ActionContext {
                target: Some(target),
                selected_files: vec![RepoPath::try_from("a.txt").expect("valid path")],
                ..Default::default()
            },
            payload: ActionPayload::None,
        };
        assert!(matches!(
            to_git_command(&request),
            Ok(GitCommand::StageFiles { target: actual, .. }) if actual == target
        ));
    }

    #[test]
    fn channel_dispatcher_reports_closed_consumer() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let dispatcher = ChannelActionDispatcher::new(sender);
        let request = ActionRequest {
            action: ActionId::Stage,
            context: ActionContext::default(),
            payload: ActionPayload::None,
        };
        assert!(dispatcher.dispatch(request.clone()).is_ok());
        assert_eq!(receiver.recv().expect("request should arrive"), request);
        drop(receiver);
        assert_eq!(
            dispatcher
                .dispatch(request)
                .expect_err("closed channel should fail"),
            "action consumer is unavailable"
        );
    }
}
