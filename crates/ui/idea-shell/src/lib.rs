//! Desktop workbench shell, native window lifecycle, DockArea, and overlay host.

use std::sync::Mutex;
use std::{sync::Arc, time::Duration};

use git_operations::{OperationOutcome, OperationPhase, OperationSnapshot};
use git_ui_model::{
    ActionAvailability, ActionContext, ActionDispatcher, ActionId, ActionPolicy, ActionRequest,
    ChangeContentSource, ChangesTreeModel, CommitDraftStore, CommitPanelViewModel,
    RepositoryWorkbenchModel,
};
use git_workbench::{PanelKind, WorkbenchLayout};
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::dock::{
    BasePanel, DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, DockSkin, Panel,
    PanelEvent, panel_handle, register_panel,
};
use gpui_kit::component::{
    Disableable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{InputEvent, Textarea, TextareaState},
};
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    Render, SharedString, Subscription, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use idea_theme::IdeaTheme;

const APP_NAME: &str = "IdeaGit";
const APP_ID: &str = "com.ideagit.desktop";
const LAYOUT_VERSION: usize = 1;

/// Opens the main IdeaGit workbench window.
///
/// The desktop composition root must call [`gpui_kit::init`] first. UI-004 will replace the
/// repository placeholder with live session tabs; UI-005 will replace panel placeholders with the
/// read-only Changes projection.
pub fn open_main_window(cx: &mut App) -> gpui_kit::Result<AnyWindowHandle> {
    open_main_window_with_layout(cx, WorkbenchLayoutIo::disabled())
}

/// Optional restored layout and a non-blocking persistence callback.
#[derive(Clone)]
pub struct WorkbenchLayoutIo {
    initial: Option<WorkbenchLayout>,
    save: Arc<dyn Fn(WorkbenchLayout) + Send + Sync>,
}

impl WorkbenchLayoutIo {
    #[must_use]
    pub fn new(
        initial: Option<WorkbenchLayout>,
        save: impl Fn(WorkbenchLayout) + Send + Sync + 'static,
    ) -> Self {
        Self {
            initial,
            save: Arc::new(save),
        }
    }

    #[must_use]
    pub fn disabled() -> Self {
        Self::new(None, |_| {})
    }
}

/// Opens the workbench with layout restoration and debounced persistence enabled.
pub fn open_main_window_with_layout(
    cx: &mut App,
    layout_io: WorkbenchLayoutIo,
) -> gpui_kit::Result<AnyWindowHandle> {
    open_workbench_window(cx, layout_io).map(|opened| opened.window)
}

/// Opened native workbench and its typed, UI-thread update controller.
pub struct OpenedWorkbench {
    pub window: AnyWindowHandle,
    pub controller: WorkbenchController,
}

/// Sends repository projections to the workbench entity on the UI thread.
#[derive(Clone)]
pub struct WorkbenchController {
    view: Entity<WorkbenchWindow>,
    action_dispatcher: Arc<Mutex<Option<Arc<dyn ActionDispatcher>>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationFeedback {
    Running(String),
    Succeeded(String),
    Failed(String),
}

/// Weak UI-thread target used by long-running desktop operation subscriptions.
#[derive(Clone)]
pub struct WorkbenchOperationSink {
    view: WeakEntity<WorkbenchWindow>,
}

impl WorkbenchOperationSink {
    /// Applies one coordinator snapshot while the workbench entity is still alive.
    ///
    /// Returns `false` after the entity has been released so the caller can stop consuming the
    /// channel without retaining the window or its action dispatcher.
    pub fn set_operation_snapshot(&self, snapshot: &OperationSnapshot, cx: &mut App) -> bool {
        let feedback = operation_feedback(snapshot);
        self.view
            .update(cx, |view, cx| {
                view.operation_feedback = Some(feedback);
                cx.notify();
            })
            .is_ok()
    }
}

impl WorkbenchController {
    /// Installs the application-owned dispatcher used by all workbench actions.
    pub fn set_action_dispatcher(&self, dispatcher: Arc<dyn ActionDispatcher>) {
        if let Ok(mut slot) = self.action_dispatcher.lock() {
            *slot = Some(dispatcher);
        }
    }

    pub fn set_repository_model(&self, model: RepositoryWorkbenchModel, cx: &mut App) {
        self.view.update(cx, |view, cx| {
            view.repository_chrome = model.chrome;
            view.commit_panel.update(cx, |panel, cx| {
                panel.set_changes(model.changes, model.expected_head, model.expected_index);
                cx.notify();
            });
            cx.notify();
        });
    }

    pub fn set_log_page(&self, page: git_domain::CommitPage, cx: &mut App) {
        let rows = git_log_ui::project(&page);
        self.view.update(cx, |view, cx| {
            view.log_panel.update(cx, |panel, cx| {
                panel.log_rows = rows;
                cx.notify();
            });
            cx.notify();
        });
    }

    pub fn set_operation_feedback(&self, feedback: OperationFeedback, cx: &mut App) {
        self.view.update(cx, |view, cx| {
            view.operation_feedback = Some(feedback);
            cx.notify();
        });
    }

    /// Publishes the latest coordinator snapshot as UI-safe progress or terminal feedback.
    pub fn set_operation_snapshot(&self, snapshot: &OperationSnapshot, cx: &mut App) {
        self.set_operation_feedback(operation_feedback(snapshot), cx);
        let is_commit = matches!(
            &snapshot.command,
            git_domain::GitCommand::CommitIndex { .. }
        );
        if is_commit {
            let snapshot = snapshot.clone();
            self.view.update(cx, |view, cx| {
                view.commit_panel.update(cx, |panel, cx| {
                    if panel
                        .commit_model
                        .as_ref()
                        .is_some_and(|model| model.target == snapshot.target)
                    {
                        let submission = if snapshot.phase != OperationPhase::Finished {
                            git_ui_model::CommitSubmissionState::Running(snapshot.id)
                        } else {
                            match snapshot.outcome {
                                Some(OperationOutcome::Failed(error))
                                | Some(OperationOutcome::RecoveryRequired(error)) => {
                                    git_ui_model::CommitSubmissionState::NoCommit(error.to_string())
                                }
                                Some(OperationOutcome::Cancelled(_)) => {
                                    git_ui_model::CommitSubmissionState::NoCommit(
                                        "提交已取消，草稿已保留".to_owned(),
                                    )
                                }
                                Some(OperationOutcome::Success) | None => {
                                    git_ui_model::CommitSubmissionState::Idle
                                }
                            }
                        };
                        if let Some(model) = panel.commit_model.as_mut() {
                            model.submission = submission;
                        }
                    }
                    cx.notify();
                });
                cx.notify();
            });
        }
    }

    /// Creates a weak feedback target for a desktop-owned asynchronous subscription.
    #[must_use]
    pub fn operation_sink(&self) -> WorkbenchOperationSink {
        WorkbenchOperationSink {
            view: self.view.downgrade(),
        }
    }

    /// Validates and queues one UI action without executing Git on the UI thread.
    pub fn dispatch_action(
        &self,
        action: ActionId,
        context: ActionContext,
        policy: &dyn ActionPolicy,
        cx: &mut App,
    ) -> Result<(), String> {
        match policy.evaluate(action, &context) {
            ActionAvailability::Enabled => {}
            ActionAvailability::Disabled(reason) => {
                self.set_operation_feedback(OperationFeedback::Failed(reason.clone()), cx);
                return Err(reason);
            }
            ActionAvailability::Hidden => {
                let reason = "此操作当前不可用".to_owned();
                self.set_operation_feedback(OperationFeedback::Failed(reason.clone()), cx);
                return Err(reason);
            }
        }
        let dispatcher = self
            .action_dispatcher
            .lock()
            .map_err(|_| "action dispatcher lock poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "action dispatcher is not configured".to_owned())?;
        dispatcher.dispatch(ActionRequest {
            action,
            context,
            payload: git_ui_model::ActionPayload::None,
        })?;
        self.set_operation_feedback(OperationFeedback::Running(format!("{action:?}")), cx);
        Ok(())
    }
}

fn operation_feedback(snapshot: &OperationSnapshot) -> OperationFeedback {
    match snapshot.outcome.as_ref() {
        Some(OperationOutcome::Success) => OperationFeedback::Succeeded("Git 操作完成".to_owned()),
        Some(OperationOutcome::Failed(error)) => OperationFeedback::Failed(error.to_string()),
        Some(OperationOutcome::Cancelled(_)) => {
            OperationFeedback::Failed("Git 操作已取消，仓库状态需重新读取".to_owned())
        }
        Some(OperationOutcome::RecoveryRequired(error)) => {
            OperationFeedback::Failed(format!("需要恢复：{error}"))
        }
        None if snapshot.phase == OperationPhase::Finished => {
            OperationFeedback::Failed("Git 操作结束但没有终态结果".to_owned())
        }
        None => OperationFeedback::Running(format!("Git 操作：{:?}", snapshot.phase)),
    }
}

/// Opens a native workbench and retains a typed controller for async Git projections.
pub fn open_workbench_window(
    cx: &mut App,
    layout_io: WorkbenchLayoutIo,
) -> gpui_kit::Result<OpenedWorkbench> {
    register_workbench_panels(cx);
    let action_dispatcher = Arc::new(Mutex::new(None));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1100.0), px(720.0)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            ..Default::default()
        }),
        app_id: Some(APP_ID.to_owned()),
        window_min_size: Some(size(px(760.0), px(520.0))),
        ..Default::default()
    };

    let window_dispatcher = Arc::clone(&action_dispatcher);
    gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| WorkbenchWindow::new(window, layout_io, window_dispatcher, cx))
    })
    .map(|(window, view)| OpenedWorkbench {
        window,
        controller: WorkbenchController {
            view,
            action_dispatcher,
        },
    })
}

fn register_workbench_panels(cx: &mut App) {
    for kind in PanelKind::ALL {
        register_panel(cx, kind.panel_name(), move |_, window, cx| {
            panel_handle(WorkbenchPanel::new(kind, window, cx))
        });
    }
}

struct WorkbenchWindow {
    dock_area: Entity<DockArea>,
    theme: IdeaTheme,
    layout_save: Arc<dyn Fn(WorkbenchLayout) + Send + Sync>,
    layout_generation: u64,
    _layout_subscription: Subscription,
    repository_chrome: git_ui_model::RepositoryChromeModel,
    commit_panel: Entity<WorkbenchPanel>,
    log_panel: Entity<WorkbenchPanel>,
    operation_feedback: Option<OperationFeedback>,
    action_dispatcher: Arc<Mutex<Option<Arc<dyn ActionDispatcher>>>>,
    _commit_action_subscription: Subscription,
}

impl WorkbenchWindow {
    fn new(
        window: &mut Window,
        layout_io: WorkbenchLayoutIo,
        action_dispatcher: Arc<Mutex<Option<Arc<dyn ActionDispatcher>>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = IdeaTheme::dark();
        let (dock_area, _skin) =
            DockSkin::dock_area("ideagit.workbench.main", Some(LAYOUT_VERSION), window, cx);
        let repository = WorkbenchPanel::new(PanelKind::RepositoryExplorer, window, cx);
        let commit = WorkbenchPanel::new(PanelKind::Commit, window, cx);
        let commit_action_subscription =
            cx.subscribe(&commit, |this, _, request: &ActionRequest, cx| {
                let result = match git_ui_model::DefaultActionPolicy
                    .evaluate(request.action, &request.context)
                {
                    ActionAvailability::Enabled => this
                        .action_dispatcher
                        .lock()
                        .map_err(|_| "action dispatcher lock poisoned".to_owned())
                        .and_then(|dispatcher| {
                            dispatcher
                                .clone()
                                .ok_or_else(|| "action dispatcher is not configured".to_owned())
                        })
                        .and_then(|dispatcher| dispatcher.dispatch(request.clone())),
                    ActionAvailability::Disabled(reason) => Err(reason),
                    ActionAvailability::Hidden => Err("此操作当前不可用".to_owned()),
                };
                this.operation_feedback = Some(match result {
                    Ok(()) => OperationFeedback::Running(format!("{:?}", request.action)),
                    Err(error) => OperationFeedback::Failed(error),
                });
                cx.notify();
            });
        let log = WorkbenchPanel::new(PanelKind::GitLog, window, cx);
        let log_panel = log.clone();
        let diff = WorkbenchPanel::new(PanelKind::Diff, window, cx);
        let console = WorkbenchPanel::new(PanelKind::Console, window, cx);
        dock_area.update(cx, |area, cx| {
            area.set_center(
                DockLayout::tabs()
                    .panel_view(panel_handle(commit.clone()), cx)
                    .panel_view(panel_handle(log), cx)
                    .panel_view(panel_handle(diff), cx),
                window,
                cx,
            );
            area.set_dock(
                DockPlacement::Left,
                DockLayout::tabs().panel_view(panel_handle(repository), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Left, px(260.0), window, cx);
            area.set_dock(
                DockPlacement::Bottom,
                DockLayout::tabs().panel_view(panel_handle(console), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Bottom, px(190.0), window, cx);
            if let Some(state) = layout_io.initial.as_ref().and_then(decode_layout)
                && state.version == Some(LAYOUT_VERSION)
            {
                let _ = area.load(state, window, cx);
            }
        });
        let layout_subscription = cx.subscribe(&dock_area, |this, _, event: &DockEvent, cx| {
            if matches!(event, DockEvent::LayoutChanged) {
                this.layout_generation = this.layout_generation.saturating_add(1);
                let generation = this.layout_generation;
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if generation != this.layout_generation {
                            return;
                        }
                        let state = this.dock_area.read(cx).dump(cx);
                        if let Some(layout) = encode_layout(&state) {
                            (this.layout_save)(layout);
                        }
                    });
                })
                .detach();
            }
        });
        if let Some(layout) = encode_layout(&dock_area.read(cx).dump(cx)) {
            (layout_io.save)(layout);
        }
        Self {
            dock_area,
            theme,
            layout_save: layout_io.save,
            layout_generation: 0,
            _layout_subscription: layout_subscription,
            repository_chrome: git_ui_model::RepositoryChromeModel::default(),
            commit_panel: commit,
            log_panel,
            operation_feedback: None,
            action_dispatcher,
            _commit_action_subscription: commit_action_subscription,
        }
    }
}

fn encode_layout(state: &DockAreaState) -> Option<WorkbenchLayout> {
    serde_json::to_value(state)
        .ok()
        .map(|dock| WorkbenchLayout::new("main", dock))
}

fn decode_layout(layout: &WorkbenchLayout) -> Option<DockAreaState> {
    layout.validate().ok()?;
    serde_json::from_value(layout.dock.clone()).ok()
}

impl Render for WorkbenchWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors;
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(colors.window_background.as_rgb()))
            .text_color(rgb(colors.text_primary.as_rgb()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(36.0))
                    .px_3()
                    .border_b_1()
                    .border_color(rgb(colors.border.as_rgb()))
                    .child(div().font_semibold().child(APP_NAME))
                    .child(div().flex().ml_4().gap_1().children(
                        self.repository_chrome.tabs.iter().map(|tab| {
                            div()
                                .px_2()
                                .py_1()
                                .when(tab.active, |element| {
                                    element.bg(rgb(colors.selection_background.as_rgb()))
                                })
                                .child(format!("{} · {}", tab.title, tab.branch))
                        }),
                    ))
                    .when(self.repository_chrome.tabs.is_empty(), |element| {
                        element.child(
                            div()
                                .ml_4()
                                .px_2()
                                .py_1()
                                .bg(rgb(colors.selection_background.as_rgb()))
                                .child("No repository"),
                        )
                    }),
            )
            .child(div().flex_1().min_h_0().child(self.dock_area.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(24.0))
                    .px_2()
                    .border_t_1()
                    .border_color(rgb(colors.border.as_rgb()))
                    .text_sm()
                    .text_color(rgb(colors.text_secondary.as_rgb()))
                    .child(self.repository_chrome.status.message.clone())
                    .child(format!(
                        "{} {}",
                        self.repository_chrome.status.branch, self.repository_chrome.status.sync
                    )),
            )
            .when_some(self.operation_feedback.clone(), |element, feedback| {
                element.child(
                    div()
                        .id("operation-feedback")
                        .absolute()
                        .top_2()
                        .right_2()
                        .p_2()
                        .child(match feedback {
                            OperationFeedback::Running(message) => format!("Running: {message}"),
                            OperationFeedback::Succeeded(message) => format!("Done: {message}"),
                            OperationFeedback::Failed(message) => format!("Failed: {message}"),
                        }),
                )
            })
            .child(div().id("workbench-overlay-host").absolute().inset_0())
    }
}

struct WorkbenchPanel {
    kind: PanelKind,
    focus: FocusHandle,
    changes: ChangesTreeModel,
    commit_drafts: CommitDraftStore,
    commit_model: Option<CommitPanelViewModel>,
    commit_message: Option<Entity<TextareaState>>,
    pending_message_sync: Option<SharedString>,
    _commit_message_subscription: Option<Subscription>,
    selected_changes: Vec<(ChangeContentSource, git_domain::RepoPath)>,
    log_rows: Vec<git_log_ui::LogRow>,
}

impl WorkbenchPanel {
    fn new(kind: PanelKind, window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let commit_message = (kind == PanelKind::Commit).then(|| {
                cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder("Commit message")
                        .auto_grow(3, 8)
                })
            });
            let commit_message_subscription = commit_message.as_ref().map(|message| {
                cx.subscribe(
                    message,
                    |this: &mut Self, message, event: &InputEvent, cx| {
                        if !matches!(event, InputEvent::Change) {
                            return;
                        }
                        let Some(target) = this.commit_model.as_ref().map(|model| model.target)
                        else {
                            return;
                        };
                        let value = message.read(cx).value().to_string();
                        this.commit_drafts.set_message(target, value.clone());
                        if let Some(model) = this.commit_model.as_mut() {
                            model.message = value;
                            model.draft_revision = this
                                .commit_drafts
                                .draft(target)
                                .map_or(0, |draft| draft.draft_revision);
                        }
                        cx.notify();
                    },
                )
            });
            Self {
                kind,
                focus: cx.focus_handle(),
                changes: ChangesTreeModel::default(),
                commit_drafts: CommitDraftStore::default(),
                commit_model: None,
                commit_message,
                pending_message_sync: None,
                _commit_message_subscription: commit_message_subscription,
                selected_changes: Vec::new(),
                log_rows: Vec::new(),
            }
        })
    }

    fn set_changes(
        &mut self,
        changes: ChangesTreeModel,
        expected_head: Option<git_domain::ObjectId>,
        expected_index: Option<git_domain::ObjectId>,
    ) {
        let same_target = self.changes.target == changes.target;
        self.changes = changes;
        if same_target {
            self.selected_changes.retain(|selected| {
                self.changes.groups.iter().any(|group| {
                    group
                        .entries
                        .iter()
                        .any(|entry| entry.source == selected.0 && entry.path == selected.1)
                })
            });
        } else {
            self.selected_changes.clear();
        }
        self.commit_model = CommitPanelViewModel::from_changes(
            &self.changes,
            &self.commit_drafts,
            expected_head,
            expected_index,
        );
        self.pending_message_sync = self
            .commit_model
            .as_ref()
            .map(|model| SharedString::from(model.message.clone()));
    }

    fn emit_file_action(&mut self, action: ActionId, cx: &mut Context<Self>) {
        if let Some(request) = file_action_request(&self.changes, &self.selected_changes, action) {
            cx.emit(request);
        }
    }

    fn emit_commit_action(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.commit_model.as_ref() else {
            return;
        };
        let staged_files = self
            .changes
            .group(git_ui_model::ChangeGroup::Staged)
            .into_iter()
            .flat_map(|group| group.entries.iter())
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        let Some(request) = commit_action_request(model, staged_files) else {
            return;
        };
        cx.emit(request);
    }
}

fn file_action_request(
    changes: &ChangesTreeModel,
    selected_changes: &[(ChangeContentSource, git_domain::RepoPath)],
    action: ActionId,
) -> Option<ActionRequest> {
    let target = changes.target?;
    let expected_source = match action {
        ActionId::Stage => ChangeContentSource::Worktree,
        ActionId::Unstage => ChangeContentSource::Index,
        _ => return None,
    };
    let selected_files = selected_changes
        .iter()
        .filter(|(source, _)| *source == expected_source)
        .map(|(_, path)| path.clone())
        .collect();
    Some(ActionRequest {
        action,
        context: ActionContext {
            target: Some(target),
            selected_files,
            active_panel: git_ui_model::ActionPanel::Commit,
            ..ActionContext::default()
        },
        payload: git_ui_model::ActionPayload::None,
    })
}

fn commit_action_request(
    model: &CommitPanelViewModel,
    staged_files: Vec<git_domain::RepoPath>,
) -> Option<ActionRequest> {
    if model.message.trim().is_empty() || model.expected_index.is_none() {
        return None;
    }
    Some(ActionRequest {
        action: ActionId::Commit,
        context: ActionContext {
            target: Some(model.target),
            selected_files: staged_files,
            active_panel: git_ui_model::ActionPanel::Commit,
            ..ActionContext::default()
        },
        payload: git_ui_model::ActionPayload::Commit(git_domain::StandardCommitRequest {
            options: git_domain::CommitOptions {
                message: model.message.as_bytes().to_vec(),
                amend: model.amend,
                allow_empty: false,
                signoff: false,
                author: None,
                committer: None,
                signing: git_domain::SigningMode::UseGitConfig,
            },
            expected_revision: model.revision,
            expected_head: model.expected_head,
            expected_index: model.expected_index,
        }),
    })
}

impl BasePanel for WorkbenchPanel {
    fn panel_name(&self) -> &'static str {
        self.kind.panel_name()
    }
}

impl Panel for WorkbenchPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(self.kind.title().into())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.kind.title()
    }
}

impl EventEmitter<PanelEvent> for WorkbenchPanel {}
impl EventEmitter<ActionRequest> for WorkbenchPanel {}

impl Focusable for WorkbenchPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for WorkbenchPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.kind == PanelKind::GitLog {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .p_3()
                .gap_1()
                .child(div().font_semibold().child("Git Log"))
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .text_xs()
                        .font_semibold()
                        .child(div().w(px(90.0)).child("Graph / OID"))
                        .child(div().flex_1().child("Subject"))
                        .child(div().w(px(140.0)).child("Author"))
                        .child(div().w(px(110.0)).child("Timestamp")),
                )
                .children(self.log_rows.iter().map(|row| {
                    div()
                        .flex()
                        .gap_3()
                        .text_sm()
                        .child(
                            div()
                                .w(px(90.0))
                                .child(format!("L{} {}", row.graph.lane, row.id)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .child(String::from_utf8_lossy(&row.subject).into_owned()),
                        )
                        .child(
                            div()
                                .w(px(140.0))
                                .child(String::from_utf8_lossy(&row.author).into_owned()),
                        )
                        .child(div().w(px(110.0)).child(row.timestamp.to_string()))
                }));
        }
        if self.kind == PanelKind::Commit {
            if let (Some(message), Some(value)) = (
                self.commit_message.as_ref(),
                self.pending_message_sync.take(),
            ) && message.read(cx).value() != value
            {
                message.update(cx, |state, cx| state.set_value(value, window, cx));
            }
            let message = self.commit_message.clone();
            let amend = self.commit_model.as_ref().is_some_and(|model| model.amend);
            let groups = self.changes.groups.clone();
            let selected = self.selected_changes.clone();
            let panel = cx.entity().downgrade();
            let amend_panel = panel.clone();
            let stage_panel = panel.clone();
            let unstage_panel = panel.clone();
            let commit_panel = panel.clone();
            let has_worktree_selection = selected
                .iter()
                .any(|(source, _)| *source == ChangeContentSource::Worktree);
            let has_index_selection = selected
                .iter()
                .any(|(source, _)| *source == ChangeContentSource::Index);
            return div()
                .flex()
                .flex_col()
                .size_full()
                .p_3()
                .gap_2()
                .child(div().font_semibold().child("Changes"))
                .children(groups.into_iter().map(|group| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().font_semibold().child(format!(
                            "{} ({})",
                            group.kind.label(),
                            group.entries.len()
                        )))
                        .children(group.entries.into_iter().map({
                            let selected = selected.clone();
                            let panel = panel.clone();
                            move |entry| {
                                let original = entry
                                    .original_path
                                    .as_ref()
                                    .map_or_else(String::new, |path| {
                                        format!(" ← {}", path.display_lossy())
                                    });
                                let conflict = if entry.conflicted { " · conflict" } else { "" };
                                let source = entry.source;
                                let path = entry.path.clone();
                                let checked = selected.contains(&(source, path.clone()));
                                let id = format!("change-{:?}-{}", source, path.display_lossy());
                                Checkbox::new(id)
                                    .checked(checked)
                                    .label(format!(
                                        "{} · {:?}{}{}",
                                        path.display_lossy(),
                                        entry.change_kind,
                                        original,
                                        conflict
                                    ))
                                    .on_change({
                                        let panel = panel.clone();
                                        move |checked, _, cx| {
                                            let _ = panel.update(cx, |panel, cx| {
                                                let key = (source, path.clone());
                                                if *checked {
                                                    if !panel.selected_changes.contains(&key) {
                                                        panel.selected_changes.push(key);
                                                    }
                                                } else {
                                                    panel
                                                        .selected_changes
                                                        .retain(|item| item != &key);
                                                }
                                                cx.notify();
                                            });
                                        }
                                    })
                            }
                        }))
                }))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("stage-selected")
                                .primary()
                                .label("Stage")
                                .disabled(!has_worktree_selection)
                                .on_click(move |_, _, cx| {
                                    let _ = stage_panel.update(cx, |panel, cx| {
                                        panel.emit_file_action(ActionId::Stage, cx);
                                    });
                                }),
                        )
                        .child(
                            Button::new("unstage-selected")
                                .label("Unstage")
                                .disabled(!has_index_selection)
                                .on_click(move |_, _, cx| {
                                    let _ = unstage_panel.update(cx, |panel, cx| {
                                        panel.emit_file_action(ActionId::Unstage, cx);
                                    });
                                }),
                        ),
                )
                .child(div().mt_2().font_semibold().child("Commit Message"))
                .children(message.map(|message| Textarea::new(&message).h(px(96.0))))
                .child(
                    Checkbox::new("commit-amend")
                        .label("Amend")
                        .checked(amend)
                        .on_change(move |checked, _, cx| {
                            let _ = amend_panel.update(cx, |panel, cx| {
                                let Some(target) =
                                    panel.commit_model.as_ref().map(|model| model.target)
                                else {
                                    return;
                                };
                                panel.commit_drafts.set_amend(target, *checked);
                                if let Some(model) = panel.commit_model.as_mut() {
                                    model.amend = *checked;
                                    model.draft_revision = panel
                                        .commit_drafts
                                        .draft(target)
                                        .map_or(0, |draft| draft.draft_revision);
                                }
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new("commit-selected")
                        .primary()
                        .label("Commit")
                        .disabled(self.commit_model.as_ref().is_none_or(|model| {
                            model.message.trim().is_empty() || model.expected_index.is_none()
                        }))
                        .on_click(move |_, _, cx| {
                            let _ = commit_panel.update(cx, |panel, cx| {
                                panel.emit_commit_action(cx);
                            });
                        }),
                );
        }
        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(format!("{} panel", self.kind.title()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_state_round_trips_through_versioned_layout_envelope() {
        let state = DockAreaState {
            version: Some(LAYOUT_VERSION),
            ..DockAreaState::default()
        };
        let layout = encode_layout(&state).expect("dock state should serialize");
        assert_eq!(decode_layout(&layout), Some(state));
    }

    #[test]
    fn incompatible_or_malformed_layout_is_ignored() {
        let mut incompatible = WorkbenchLayout::new("main", serde_json::json!({}));
        incompatible.schema = 0;
        assert_eq!(decode_layout(&incompatible), None);

        let malformed = WorkbenchLayout::new("main", serde_json::json!({"not": "dock"}));
        assert_eq!(decode_layout(&malformed), None);
    }

    #[test]
    fn stage_and_unstage_requests_keep_content_sources_separate() {
        let target = git_domain::SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        );
        let staged = git_domain::RepoPath::try_from("staged.txt").expect("valid path");
        let unstaged = git_domain::RepoPath::try_from("unstaged.txt").expect("valid path");
        let changes = ChangesTreeModel {
            target: Some(target),
            revision: Some(git_domain::SnapshotRevision::new(1)),
            ..ChangesTreeModel::default()
        };
        let selection = vec![
            (ChangeContentSource::Index, staged.clone()),
            (ChangeContentSource::Worktree, unstaged.clone()),
        ];

        let stage = file_action_request(&changes, &selection, ActionId::Stage)
            .expect("stage request should project");
        let unstage = file_action_request(&changes, &selection, ActionId::Unstage)
            .expect("unstage request should project");

        assert_eq!(stage.context.selected_files, vec![unstaged]);
        assert_eq!(unstage.context.selected_files, vec![staged]);
        assert_eq!(stage.context.target, Some(target));
    }

    #[test]
    fn commit_request_carries_authoritative_revision_and_keeps_message() {
        let target = git_domain::SessionTarget::worktree(
            git_domain::RepositoryId::new(),
            git_domain::WorktreeId::new(),
            1,
        );
        let head = git_domain::ObjectId::from_bytes(git_domain::ObjectFormat::Sha1, &[1; 20])
            .expect("valid head");
        let index = git_domain::ObjectId::from_bytes(git_domain::ObjectFormat::Sha1, &[2; 20])
            .expect("valid index");
        let model = CommitPanelViewModel {
            target,
            revision: git_domain::SnapshotRevision::new(7),
            changes: Vec::new(),
            selected: Vec::new(),
            message: "commit from panel".to_owned(),
            amend: true,
            draft_revision: 3,
            expected_head: Some(head),
            expected_index: Some(index),
            submission: git_ui_model::CommitSubmissionState::Idle,
        };
        let path = git_domain::RepoPath::try_from("ready.txt").expect("valid path");
        let request = commit_action_request(&model, vec![path.clone()])
            .expect("complete commit identity should enable request");

        assert_eq!(request.context.selected_files, vec![path]);
        match request.payload {
            git_ui_model::ActionPayload::Commit(commit) => {
                assert_eq!(
                    commit.expected_revision,
                    git_domain::SnapshotRevision::new(7)
                );
                assert_eq!(commit.expected_head, Some(head));
                assert_eq!(commit.expected_index, Some(index));
                assert!(commit.options.amend);
                assert_eq!(commit.options.message, b"commit from panel");
            }
            _ => panic!("commit action should carry typed commit payload"),
        }
    }
}
