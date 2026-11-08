//! Shared UI projections, selections, actions, and asynchronous request identities.

use std::{collections::HashMap, time::SystemTime};

use git_application::{RefreshHealth, RefreshState, RepositorySessionView};
use git_changelist::ChangelistId;
use git_domain::{
    BranchCommand, ChangeKind, ChangeSelection, GitCommand, HeadState, InteractiveRebasePlan,
    ObjectId, OperationId, RebaseAction, RebasePlanValidationError, RebaseTodoEntry, RefName,
    RepoPath, RepositorySnapshot, SelectionCommitRequest, SessionTarget, SnapshotRevision,
    StandardCommitRequest,
};
use git_shelf::{ShelfEntry, ShelfId, UnshelveDecision, unshelve_decision};

/// Which saved-change collection is visible in the Shelf/Stash panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedChangesView {
    Shelf,
    Stash,
}

/// A read-only row projected from an application-owned Shelf entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfRowViewModel {
    pub id: ShelfId,
    pub name: String,
    pub created_at: i64,
    pub base_head: Option<ObjectId>,
    pub file_count: usize,
    pub selected: bool,
    pub unshelve_decision: UnshelveDecision,
}

/// Pure state for the Shelf/Stash view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfStashViewModel {
    pub target: SessionTarget,
    pub view: SavedChangesView,
    pub search_query: String,
    pub selected: Option<ShelfId>,
    pub rows: Vec<ShelfRowViewModel>,
    pub worktree_dirty: bool,
    pub current_head: Option<ObjectId>,
}

impl ShelfStashViewModel {
    #[must_use]
    pub fn from_entries(
        target: SessionTarget,
        entries: &[ShelfEntry],
        current_head: Option<ObjectId>,
        worktree_dirty: bool,
    ) -> Self {
        Self {
            target,
            view: SavedChangesView::Shelf,
            search_query: String::new(),
            selected: None,
            rows: entries
                .iter()
                .map(|entry| Self::row(entry, None, current_head, worktree_dirty))
                .collect(),
            worktree_dirty,
            current_head,
        }
    }

    fn row(
        entry: &ShelfEntry,
        selected: Option<ShelfId>,
        current_head: Option<ObjectId>,
        worktree_dirty: bool,
    ) -> ShelfRowViewModel {
        ShelfRowViewModel {
            id: entry.id,
            name: entry.name.clone(),
            created_at: entry.created_at,
            base_head: entry.base_head,
            file_count: entry.bundle.files.len(),
            selected: selected == Some(entry.id),
            unshelve_decision: unshelve_decision(entry, current_head, worktree_dirty),
        }
    }

    pub fn select(&mut self, id: Option<ShelfId>) {
        self.selected = id;
        for row in &mut self.rows {
            row.selected = Some(row.id) == id;
        }
    }

    pub fn set_search_query(&mut self, query: impl Into<String>) {
        self.search_query = query.into();
    }

    #[must_use]
    pub fn visible_rows(&self) -> Vec<ShelfRowViewModel> {
        let query = self.search_query.trim().to_lowercase();
        self.rows
            .iter()
            .filter(|row| query.is_empty() || row.name.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    #[must_use]
    pub fn selected_row(&self) -> Option<&ShelfRowViewModel> {
        self.rows.iter().find(|row| Some(row.id) == self.selected)
    }

    #[must_use]
    pub fn can_unshelve_selected(&self) -> bool {
        self.selected_row()
            .is_some_and(|row| row.unshelve_decision == UnshelveDecision::Apply)
    }
}

/// Lifecycle state shown by the Interactive Rebase dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebaseDialogPhase {
    Editing,
    Running,
    Conflict,
    Completed,
    Failed(String),
}

/// One editable row in the Interactive Rebase dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebaseTodoRowModel {
    pub commit: ObjectId,
    pub action: RebaseAction,
    pub subject: Vec<u8>,
    pub message_override: Option<String>,
}

/// Pure UI state for an Interactive Rebase dialog.
///
/// The model only edits a validated domain plan. Execution and conflict recovery remain owned by
/// the application and Git ports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebaseDialogViewModel {
    pub target: SessionTarget,
    pub upstream: ObjectId,
    pub rows: Vec<RebaseTodoRowModel>,
    pub selected_row: Option<usize>,
    pub phase: RebaseDialogPhase,
}

impl RebaseDialogViewModel {
    #[must_use]
    pub fn from_plan(plan: &InteractiveRebasePlan, subjects: &[(ObjectId, Vec<u8>)]) -> Self {
        let rows = plan
            .entries
            .iter()
            .map(|entry| RebaseTodoRowModel {
                commit: entry.commit,
                action: entry.action,
                subject: subjects
                    .iter()
                    .find(|(commit, _)| *commit == entry.commit)
                    .map_or_else(Vec::new, |(_, subject)| subject.clone()),
                message_override: entry.message_override.clone(),
            })
            .collect();
        Self {
            target: plan.target,
            upstream: plan.upstream,
            rows,
            selected_row: None,
            phase: RebaseDialogPhase::Editing,
        }
    }

    pub fn move_row(&mut self, from: usize, to: usize) -> bool {
        if from >= self.rows.len() || to >= self.rows.len() || from == to {
            return false;
        }
        let row = self.rows.remove(from);
        self.rows.insert(to, row);
        self.selected_row = Some(to);
        true
    }

    pub fn set_action(&mut self, row: usize, action: RebaseAction) -> bool {
        let Some(entry) = self.rows.get_mut(row) else {
            return false;
        };
        entry.action = action;
        if action == RebaseAction::Drop {
            entry.message_override = None;
        }
        true
    }

    pub fn set_message_override(&mut self, row: usize, message: Option<String>) -> bool {
        let Some(entry) = self.rows.get_mut(row) else {
            return false;
        };
        if entry.action == RebaseAction::Drop {
            return false;
        }
        entry.message_override = message.filter(|value| !value.trim().is_empty());
        true
    }

    pub fn plan(&self) -> Result<InteractiveRebasePlan, RebasePlanValidationError> {
        let plan = InteractiveRebasePlan {
            target: self.target,
            upstream: self.upstream,
            entries: self
                .rows
                .iter()
                .map(|row| RebaseTodoEntry {
                    commit: row.commit,
                    action: row.action,
                    message_override: row.message_override.clone(),
                })
                .collect(),
        };
        plan.validate().map(|()| plan)
    }

    #[must_use]
    pub fn validation_error(&self) -> Option<RebasePlanValidationError> {
        self.plan().err()
    }
}

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

/// Submission lifecycle rendered by the Commit Panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitSubmissionState {
    Idle,
    Validating,
    Previewing,
    AwaitingConfirmation,
    Running(OperationId),
    CommittedAndReconciled(ObjectId),
    CommittedButNeedsRecovery {
        oid: ObjectId,
        journal_id: OperationId,
    },
    NoCommit(String),
    Indeterminate(OperationId),
}

/// Commit semantics selected by the Commit Panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitMode {
    /// Git Index is the authoritative selection source.
    Staging,
    /// Application-owned Changelist selections are committed through the partial-commit flow.
    Changelist,
}

/// Repository-scoped editable state that must survive refreshes and failed submissions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitDraft {
    pub target: SessionTarget,
    pub draft_revision: u64,
    pub message: String,
    pub amend: bool,
    pub selected_changes: Vec<ChangeSelection>,
    pub updated_at: SystemTime,
}

impl CommitDraft {
    fn empty(target: SessionTarget) -> Self {
        Self {
            target,
            draft_revision: 0,
            message: String::new(),
            amend: false,
            selected_changes: Vec::new(),
            updated_at: SystemTime::now(),
        }
    }

    fn touch(&mut self) {
        self.draft_revision = self.draft_revision.saturating_add(1);
        self.updated_at = SystemTime::now();
    }
}

/// Per-window draft storage keyed by the complete session target.
#[derive(Clone, Debug, Default)]
pub struct CommitDraftStore {
    drafts: HashMap<SessionTarget, CommitDraft>,
}

impl CommitDraftStore {
    #[must_use]
    pub fn draft(&self, target: SessionTarget) -> Option<&CommitDraft> {
        self.drafts.get(&target)
    }

    pub fn set_message(&mut self, target: SessionTarget, message: impl Into<String>) {
        let draft = self
            .drafts
            .entry(target)
            .or_insert_with(|| CommitDraft::empty(target));
        let message = message.into();
        if draft.message != message {
            draft.message = message;
            draft.touch();
        }
    }

    pub fn set_amend(&mut self, target: SessionTarget, amend: bool) {
        let draft = self
            .drafts
            .entry(target)
            .or_insert_with(|| CommitDraft::empty(target));
        if draft.amend != amend {
            draft.amend = amend;
            draft.touch();
        }
    }

    pub fn set_selection(
        &mut self,
        target: SessionTarget,
        selected_changes: Vec<ChangeSelection>,
    ) -> Result<(), String> {
        if selected_changes
            .iter()
            .any(|selection| selection.target() != target)
        {
            return Err("commit selection belongs to another repository session".to_owned());
        }
        let draft = self
            .drafts
            .entry(target)
            .or_insert_with(|| CommitDraft::empty(target));
        if draft.selected_changes != selected_changes {
            draft.selected_changes = selected_changes;
            draft.touch();
        }
        Ok(())
    }
}

/// Snapshot projection plus the repository-scoped editable Commit Panel state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitPanelViewModel {
    pub target: SessionTarget,
    pub revision: SnapshotRevision,
    pub changes: Vec<ChangeTreeGroup>,
    pub selected: Vec<ChangeSelection>,
    pub message: String,
    pub amend: bool,
    pub mode: CommitMode,
    pub changelist: Option<ChangelistId>,
    pub draft_revision: u64,
    pub expected_head: Option<ObjectId>,
    pub expected_index: Option<ObjectId>,
    pub submission: CommitSubmissionState,
}

impl CommitPanelViewModel {
    #[must_use]
    pub fn from_changes(
        changes: &ChangesTreeModel,
        drafts: &CommitDraftStore,
        expected_head: Option<ObjectId>,
        expected_index: Option<ObjectId>,
    ) -> Option<Self> {
        let target = changes.target?;
        let revision = changes.revision?;
        let draft = drafts
            .draft(target)
            .cloned()
            .unwrap_or_else(|| CommitDraft::empty(target));
        Some(Self {
            target,
            revision,
            changes: changes.groups.clone(),
            selected: draft.selected_changes,
            message: draft.message,
            amend: draft.amend,
            mode: CommitMode::Staging,
            changelist: None,
            draft_revision: draft.draft_revision,
            expected_head,
            expected_index,
            submission: CommitSubmissionState::Idle,
        })
    }

    /// Switches the visible commit mode without staging, unstaging, or mutating Git state.
    pub fn set_mode(&mut self, mode: CommitMode) {
        self.mode = mode;
        if mode == CommitMode::Staging {
            self.changelist = None;
        }
    }

    /// Binds a Changelist identity before the Changelist commit mode can be submitted.
    pub fn bind_changelist(&mut self, changelist: ChangelistId) {
        self.mode = CommitMode::Changelist;
        self.changelist = Some(changelist);
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
    pub expected_head: Option<ObjectId>,
    pub expected_index: Option<ObjectId>,
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

/// A branch row rendered by the Branch Popup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchRowViewModel {
    pub reference: RefName,
    pub display_name: String,
    pub is_current: bool,
    pub is_favorite: bool,
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
}

/// One named branch section (Recent, Local, or Remote).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchSectionViewModel {
    pub title: String,
    pub rows: Vec<BranchRowViewModel>,
}

/// State owned by one repository-scoped Branch Popup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchPopupViewModel {
    pub target: SessionTarget,
    pub search_query: String,
    pub selected_branch: Option<RefName>,
    pub sections: Vec<BranchSectionViewModel>,
    pub loading: bool,
    pub error: Option<String>,
}

impl BranchPopupViewModel {
    #[must_use]
    pub fn visible_sections(&self) -> Vec<BranchSectionViewModel> {
        let query = self.search_query.trim().to_lowercase();
        self.sections
            .iter()
            .map(|section| BranchSectionViewModel {
                title: section.title.clone(),
                rows: section
                    .rows
                    .iter()
                    .filter(|row| {
                        query.is_empty()
                            || row.display_name.to_lowercase().contains(&query)
                            || row
                                .reference
                                .display_lossy()
                                .to_lowercase()
                                .contains(&query)
                    })
                    .cloned()
                    .collect(),
            })
            .filter(|section| !section.rows.is_empty())
            .collect()
    }

    pub fn select(&mut self, branch: Option<RefName>) {
        self.selected_branch = branch;
    }

    #[must_use]
    pub fn checkout_request(&self) -> Option<ActionRequest> {
        let branch = self.selected_branch.clone()?;
        Some(ActionRequest {
            action: ActionId::CheckoutBranch,
            context: ActionContext {
                target: Some(self.target),
                selected_branch: Some(branch.clone()),
                active_panel: ActionPanel::RepositoryExplorer,
                ..ActionContext::default()
            },
            payload: ActionPayload::Branch(BranchCommand::Checkout { name: branch }),
        })
    }

    #[must_use]
    pub fn create_request(&self, branch: RefName) -> ActionRequest {
        ActionRequest {
            action: ActionId::CreateBranch,
            context: ActionContext {
                target: Some(self.target),
                active_panel: ActionPanel::RepositoryExplorer,
                ..ActionContext::default()
            },
            payload: ActionPayload::Branch(BranchCommand::Create { name: branch }),
        }
    }
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
    /// A confirmed Changelist request; it must retain selection fingerprints and cannot
    /// downgrade to a normal Index commit.
    SelectionCommit(SelectionCommitRequest),
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
    match request.action {
        ActionId::Stage | ActionId::Unstage if request.context.selected_files.is_empty() => {
            Err("action requires at least one selected file".to_owned())
        }
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
            ActionPayload::SelectionCommit(commit) => Ok(GitCommand::CommitSelection {
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
            expected_head: refresh
                .snapshot
                .as_ref()
                .and_then(|snapshot| match snapshot.head {
                    HeadState::Attached { oid, .. } | HeadState::Detached { oid } => Some(oid),
                    HeadState::Unborn { .. } => None,
                }),
            expected_index: None,
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

    fn oid_value(value: u8) -> ObjectId {
        ObjectId::from_bytes(ObjectFormat::Sha1, &[value; 20]).expect("test OID should be valid")
    }

    fn test_target() -> SessionTarget {
        SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        )
    }

    fn whole_file(target: SessionTarget, path: &str) -> ChangeSelection {
        ChangeSelection::WholeFile {
            file: git_domain::FileIdentity {
                target,
                path: RepoPath::try_from(path).expect("valid repository path"),
                previous_path: None,
                kind: git_domain::FileKind::Regular,
            },
        }
    }

    #[test]
    fn commit_drafts_remain_isolated_when_switching_repositories() {
        let first = test_target();
        let second = test_target();
        let mut drafts = CommitDraftStore::default();

        drafts.set_message(first, "first message");
        drafts.set_amend(first, true);
        drafts
            .set_selection(first, vec![whole_file(first, "first.txt")])
            .expect("selection should match first repository");
        drafts.set_message(second, "second message");

        let first_draft = drafts.draft(first).expect("first draft should remain");
        assert_eq!(first_draft.message, "first message");
        assert!(first_draft.amend);
        assert_eq!(first_draft.selected_changes.len(), 1);
        assert_eq!(
            drafts
                .draft(second)
                .expect("second draft should remain")
                .message,
            "second message"
        );
    }

    #[test]
    fn commit_panel_projection_preserves_draft_across_snapshot_refresh() {
        let target = test_target();
        let mut drafts = CommitDraftStore::default();
        drafts.set_message(target, "keep after failure");
        let initial_revision = drafts
            .draft(target)
            .expect("draft should exist")
            .draft_revision;
        let changes = ChangesTreeModel {
            target: Some(target),
            revision: Some(SnapshotRevision::new(9)),
            ..ChangesTreeModel::default()
        };

        let panel = CommitPanelViewModel::from_changes(&changes, &drafts, Some(oid()), None)
            .expect("complete snapshot should project");

        assert_eq!(panel.message, "keep after failure");
        assert_eq!(panel.revision, SnapshotRevision::new(9));
        assert_eq!(panel.draft_revision, initial_revision);
    }

    #[test]
    fn changelist_mode_requires_identity_and_never_mutates_git_selection() {
        let target = test_target();
        let changes = ChangesTreeModel {
            target: Some(target),
            revision: Some(SnapshotRevision::new(1)),
            ..ChangesTreeModel::default()
        };
        let drafts = CommitDraftStore::default();
        let mut panel = CommitPanelViewModel::from_changes(&changes, &drafts, None, None)
            .expect("panel should project");
        assert_eq!(panel.mode, CommitMode::Staging);
        assert!(panel.changelist.is_none());

        let selected = whole_file(target, "selected.txt");
        panel.selected.push(selected.clone());
        let changelist = ChangelistId::new();
        panel.bind_changelist(changelist);
        assert_eq!(panel.mode, CommitMode::Changelist);
        assert_eq!(panel.changelist, Some(changelist));
        assert_eq!(panel.selected, vec![selected]);

        panel.set_mode(CommitMode::Staging);
        assert_eq!(panel.changelist, None);
        assert_eq!(panel.selected.len(), 1);
    }

    #[test]
    fn commit_draft_rejects_cross_repository_selection() {
        let target = test_target();
        let foreign = test_target();
        let mut drafts = CommitDraftStore::default();

        let error = drafts
            .set_selection(target, vec![whole_file(foreign, "foreign.txt")])
            .expect_err("foreign selection must fail closed");

        assert_eq!(
            error,
            "commit selection belongs to another repository session"
        );
        assert!(drafts.draft(target).is_none());
    }

    #[test]
    fn branch_popup_filters_rows_and_builds_typed_checkout() {
        let target = test_target();
        let main = RefName::new(b"main".to_vec()).expect("valid branch");
        let feature = RefName::new(b"feature/ui".to_vec()).expect("valid branch");
        let mut popup = BranchPopupViewModel {
            target,
            search_query: "ui".to_owned(),
            selected_branch: Some(feature.clone()),
            sections: vec![BranchSectionViewModel {
                title: "Local".to_owned(),
                rows: vec![
                    BranchRowViewModel {
                        reference: main,
                        display_name: "main".to_owned(),
                        is_current: true,
                        is_favorite: false,
                        ahead: None,
                        behind: None,
                    },
                    BranchRowViewModel {
                        reference: feature.clone(),
                        display_name: "feature/ui".to_owned(),
                        is_current: false,
                        is_favorite: true,
                        ahead: Some(1),
                        behind: Some(2),
                    },
                ],
            }],
            loading: false,
            error: None,
        };

        let visible = popup.visible_sections();
        assert_eq!(visible[0].rows.len(), 1);
        assert_eq!(visible[0].rows[0].reference, feature);
        let request = popup
            .checkout_request()
            .expect("selected branch should checkout");
        assert!(matches!(
            request.payload,
            ActionPayload::Branch(BranchCommand::Checkout { .. })
        ));
        popup.select(None);
        assert!(popup.checkout_request().is_none());
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

    #[test]
    fn commit_and_branch_payloads_map_to_typed_commands() {
        let target = SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        );
        let commit = ActionRequest {
            action: ActionId::Commit,
            context: ActionContext {
                target: Some(target),
                selected_files: vec![RepoPath::try_from("a.txt").expect("valid path")],
                ..Default::default()
            },
            payload: ActionPayload::Commit(StandardCommitRequest {
                options: git_domain::CommitOptions {
                    message: "提交消息".as_bytes().to_vec(),
                    amend: false,
                    allow_empty: false,
                    signoff: false,
                    author: None,
                    committer: None,
                    signing: git_domain::SigningMode::UseGitConfig,
                },
                expected_revision: SnapshotRevision::new(1),
                expected_head: None,
                expected_index: None,
            }),
        };
        assert!(matches!(
            to_git_command(&commit),
            Ok(GitCommand::CommitIndex { target: actual, .. }) if actual == target
        ));

        let selection = whole_file(target, "a.txt");
        let selection_commit = ActionRequest {
            action: ActionId::Commit,
            context: commit.context.clone(),
            payload: ActionPayload::SelectionCommit(SelectionCommitRequest {
                options: git_domain::CommitOptions {
                    message: b"selection commit".to_vec(),
                    amend: false,
                    allow_empty: false,
                    signoff: false,
                    author: None,
                    committer: None,
                    signing: git_domain::SigningMode::Disable,
                },
                selections: vec![selection],
                fingerprints: vec![git_domain::SelectionFingerprint {
                    expected_head: None,
                    expected_index: None,
                    diff_generation: SnapshotRevision::new(1),
                    content_digest: vec![7; 32],
                }],
                expected_revision: SnapshotRevision::new(1),
            }),
        };
        assert!(matches!(
            to_git_command(&selection_commit),
            Ok(GitCommand::CommitSelection { target: actual, .. }) if actual == target
        ));

        let branch = ActionRequest {
            action: ActionId::CreateBranch,
            context: ActionContext {
                target: Some(target),
                ..Default::default()
            },
            payload: ActionPayload::Branch(git_domain::BranchCommand::Create {
                name: git_domain::RefName::new(b"feature".to_vec()).expect("valid branch"),
            }),
        };
        assert!(matches!(
            to_git_command(&branch),
            Ok(GitCommand::Branch { target: actual, .. }) if actual == target
        ));
    }

    #[test]
    fn rebase_dialog_edits_order_actions_and_builds_a_validated_plan() {
        let target = SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        );
        let source = InteractiveRebasePlan {
            target,
            upstream: oid_value(1),
            entries: vec![
                RebaseTodoEntry {
                    commit: oid_value(2),
                    action: RebaseAction::Pick,
                    message_override: None,
                },
                RebaseTodoEntry {
                    commit: oid_value(3),
                    action: RebaseAction::Pick,
                    message_override: None,
                },
            ],
        };
        let mut dialog = RebaseDialogViewModel::from_plan(
            &source,
            &[
                (oid_value(2), b"first".to_vec()),
                (oid_value(3), b"second".to_vec()),
            ],
        );
        assert!(dialog.move_row(1, 0));
        assert!(dialog.set_action(0, RebaseAction::Reword));
        assert!(dialog.set_message_override(0, Some("updated message".to_owned())));
        let plan = dialog.plan().expect("dialog should produce a valid plan");
        assert_eq!(plan.entries[0].commit, oid_value(3));
        assert_eq!(plan.entries[0].action, RebaseAction::Reword);
        assert_eq!(
            plan.entries[0].message_override.as_deref(),
            Some("updated message")
        );
        assert_eq!(dialog.rows[0].subject, b"second".to_vec());
    }

    #[test]
    fn rebase_dialog_rejects_invalid_squash_and_drop_message() {
        let target = SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        );
        let plan = InteractiveRebasePlan {
            target,
            upstream: oid_value(1),
            entries: vec![RebaseTodoEntry {
                commit: oid_value(2),
                action: RebaseAction::Pick,
                message_override: None,
            }],
        };
        let mut dialog = RebaseDialogViewModel::from_plan(&plan, &[]);
        assert!(dialog.set_action(0, RebaseAction::Drop));
        assert!(!dialog.set_message_override(0, Some("cannot apply".to_owned())));
        assert!(dialog.set_action(0, RebaseAction::Squash));
        assert!(matches!(
            dialog.validation_error(),
            Some(git_domain::RebasePlanValidationError::SquashOrFixupWithoutPrevious)
        ));
    }

    #[test]
    fn shelf_view_filters_and_marks_stale_unshelve_as_conflict() {
        let target = test_target();
        let entry = git_shelf::ShelfEntry::new(
            target,
            "before refactor",
            1,
            Some(oid_value(1)),
            git_shelf::PatchBundle::new(vec![
                git_shelf::ShelvedFile::new(
                    RepoPath::try_from("a.txt").expect("path"),
                    b"diff".to_vec(),
                    vec![1],
                )
                .expect("file"),
            ])
            .expect("bundle"),
        )
        .expect("entry");
        let mut view = ShelfStashViewModel::from_entries(
            target,
            std::slice::from_ref(&entry),
            Some(oid_value(2)),
            false,
        );
        assert_eq!(view.view, SavedChangesView::Shelf);
        assert!(!view.can_unshelve_selected());
        view.select(Some(entry.id));
        assert!(!view.can_unshelve_selected());
        view.set_search_query("refactor");
        assert_eq!(view.visible_rows().len(), 1);
        view.set_search_query("missing");
        assert!(view.visible_rows().is_empty());
    }

    #[test]
    fn shelf_view_allows_clean_matching_base() {
        let target = test_target();
        let entry = git_shelf::ShelfEntry::new(
            target,
            "saved",
            1,
            Some(oid_value(1)),
            git_shelf::PatchBundle::new(vec![
                git_shelf::ShelvedFile::new(
                    RepoPath::try_from("a.txt").expect("path"),
                    b"diff".to_vec(),
                    vec![1],
                )
                .expect("file"),
            ])
            .expect("bundle"),
        )
        .expect("entry");
        let view = ShelfStashViewModel::from_entries(target, &[entry], Some(oid_value(1)), false);
        assert_eq!(view.rows[0].unshelve_decision, UnshelveDecision::Apply);
    }
}
