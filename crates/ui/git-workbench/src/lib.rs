//! Repository UI orchestration and cross-panel coordination.

use git_domain::SessionTarget;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Layout schema understood by the UI-003 workbench shell.
pub const WORKBENCH_LAYOUT_SCHEMA: u32 = 1;

/// Stable identifiers for top-level workbench panels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    RepositoryExplorer,
    GitLog,
    #[default]
    Commit,
    Diff,
    Console,
}

impl PanelKind {
    pub const ALL: [Self; 5] = [
        Self::RepositoryExplorer,
        Self::GitLog,
        Self::Commit,
        Self::Diff,
        Self::Console,
    ];

    /// Stable name persisted by GPUI-Kit's PanelRegistry.
    #[must_use]
    pub const fn panel_name(self) -> &'static str {
        match self {
            Self::RepositoryExplorer => "ideagit.repository-explorer",
            Self::GitLog => "ideagit.git-log",
            Self::Commit => "ideagit.commit",
            Self::Diff => "ideagit.diff",
            Self::Console => "ideagit.console",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::RepositoryExplorer => "Repository",
            Self::GitLog => "Git Log",
            Self::Commit => "Commit",
            Self::Diff => "Diff",
            Self::Console => "Git Console",
        }
    }

    #[must_use]
    pub fn from_panel_name(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|panel| panel.panel_name() == value)
    }
}

/// Stable identifier for an overlay instance.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OverlayId(String);

impl OverlayId {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidOverlayId> {
        let value = value.into();
        if value.trim().is_empty() || value.len() > 128 {
            return Err(InvalidOverlayId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("overlay ID must contain 1 to 128 non-whitespace bytes")]
pub struct InvalidOverlayId;

/// Window-local workbench state. Git sessions may be shared, but selection and layout are not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkbenchState {
    pub active_target: Option<SessionTarget>,
    pub active_panel: PanelKind,
    pub layout_id: String,
    pub overlays: Vec<OverlayId>,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self {
            active_target: None,
            active_panel: PanelKind::Commit,
            layout_id: "main".to_owned(),
            overlays: Vec::new(),
        }
    }
}

/// Versioned persisted DockArea payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkbenchLayout {
    pub schema: u32,
    pub layout_id: String,
    pub dock: serde_json::Value,
}

impl WorkbenchLayout {
    pub fn new(layout_id: impl Into<String>, dock: serde_json::Value) -> Self {
        Self {
            schema: WORKBENCH_LAYOUT_SCHEMA,
            layout_id: layout_id.into(),
            dock,
        }
    }

    pub fn validate(&self) -> Result<(), LayoutCompatibilityError> {
        if self.schema != WORKBENCH_LAYOUT_SCHEMA {
            return Err(LayoutCompatibilityError::UnsupportedSchema(self.schema));
        }
        if self.layout_id.trim().is_empty() {
            return Err(LayoutCompatibilityError::EmptyLayoutId);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum LayoutCompatibilityError {
    #[error("unsupported workbench layout schema {0}")]
    UnsupportedSchema(u32),
    #[error("workbench layout ID is empty")]
    EmptyLayoutId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_panel_names_round_trip() {
        for panel in PanelKind::ALL {
            assert_eq!(PanelKind::from_panel_name(panel.panel_name()), Some(panel));
        }
        assert_eq!(PanelKind::from_panel_name("removed.panel"), None);
    }

    #[test]
    fn layout_schema_rejects_old_or_invalid_payloads() {
        let valid = WorkbenchLayout::new("main", serde_json::json!({"center": {}}));
        assert_eq!(valid.validate(), Ok(()));

        let mut old = valid.clone();
        old.schema = 0;
        assert_eq!(
            old.validate(),
            Err(LayoutCompatibilityError::UnsupportedSchema(0))
        );
        let mut empty = valid;
        empty.layout_id.clear();
        assert_eq!(
            empty.validate(),
            Err(LayoutCompatibilityError::EmptyLayoutId)
        );
    }

    #[test]
    fn overlay_ids_reject_empty_and_unbounded_values() {
        assert_eq!(OverlayId::new(" "), Err(InvalidOverlayId));
        assert_eq!(OverlayId::new("x".repeat(129)), Err(InvalidOverlayId));
        assert_eq!(
            OverlayId::new("branch-popup").map(|id| id.0),
            Ok("branch-popup".into())
        );
    }
}
