//! Desktop workbench shell, native window lifecycle, DockArea, and overlay host.

use std::sync::Mutex;
use std::{sync::Arc, time::Duration};

use git_ui_model::{
    ActionAvailability, ActionContext, ActionDispatcher, ActionId, ActionPolicy, ActionRequest,
    ChangesTreeModel, RepositoryWorkbenchModel,
};
use git_workbench::{PanelKind, WorkbenchLayout};
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::dock::{
    BasePanel, DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, DockSkin, Panel,
    PanelEvent, panel_handle, register_panel,
};
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    Render, SharedString, Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
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
                panel.changes = model.changes;
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
        dispatcher.dispatch(ActionRequest { action, context })?;
        self.set_operation_feedback(OperationFeedback::Running(format!("{action:?}")), cx);
        Ok(())
    }
}

/// Opens a native workbench and retains a typed controller for async Git projections.
pub fn open_workbench_window(
    cx: &mut App,
    layout_io: WorkbenchLayoutIo,
) -> gpui_kit::Result<OpenedWorkbench> {
    register_workbench_panels(cx);
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

    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| WorkbenchWindow::new(window, layout_io, cx))
    })
    .map(|(window, view)| OpenedWorkbench {
        window,
        controller: WorkbenchController {
            view,
            action_dispatcher: Arc::new(Mutex::new(None)),
        },
    })
}

fn register_workbench_panels(cx: &mut App) {
    for kind in PanelKind::ALL {
        register_panel(cx, kind.panel_name(), move |_, _, cx| {
            panel_handle(WorkbenchPanel::new(kind, cx))
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
}

impl WorkbenchWindow {
    fn new(window: &mut Window, layout_io: WorkbenchLayoutIo, cx: &mut Context<Self>) -> Self {
        let theme = IdeaTheme::dark();
        let (dock_area, _skin) =
            DockSkin::dock_area("ideagit.workbench.main", Some(LAYOUT_VERSION), window, cx);
        let repository = WorkbenchPanel::new(PanelKind::RepositoryExplorer, cx);
        let commit = WorkbenchPanel::new(PanelKind::Commit, cx);
        let log = WorkbenchPanel::new(PanelKind::GitLog, cx);
        let log_panel = log.clone();
        let diff = WorkbenchPanel::new(PanelKind::Diff, cx);
        let console = WorkbenchPanel::new(PanelKind::Console, cx);
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
    log_rows: Vec<git_log_ui::LogRow>,
}

impl WorkbenchPanel {
    fn new(kind: PanelKind, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            kind,
            focus: cx.focus_handle(),
            changes: ChangesTreeModel::default(),
            log_rows: Vec::new(),
        })
    }
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

impl Focusable for WorkbenchPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for WorkbenchPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if self.kind == PanelKind::GitLog {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .p_3()
                .gap_1()
                .child(div().font_semibold().child("Git Log"))
                .children(self.log_rows.iter().map(|row| {
                    div().text_sm().child(format!(
                        "{} · {}",
                        row.id,
                        String::from_utf8_lossy(&row.subject)
                    ))
                }));
        }
        if self.kind == PanelKind::Commit {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .p_3()
                .gap_2()
                .child(div().font_semibold().child("Changes"))
                .children(self.changes.groups.iter().map(|group| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().font_semibold().child(format!(
                            "{} ({})",
                            group.kind.label(),
                            group.entries.len()
                        )))
                        .children(group.entries.iter().map(|entry| {
                            let original = entry
                                .original_path
                                .as_ref()
                                .map_or_else(String::new, |path| {
                                    format!(" ← {}", path.display_lossy())
                                });
                            let conflict = if entry.conflicted { " · conflict" } else { "" };
                            div().pl_3().text_sm().child(format!(
                                "{} · {:?}{}{}",
                                entry.path.display_lossy(),
                                entry.change_kind,
                                original,
                                conflict
                            ))
                        }))
                }));
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
}
