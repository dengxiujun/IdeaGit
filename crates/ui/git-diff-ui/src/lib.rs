//! Diff viewers and staging presentation.

use git_diff::ThreeVersionDocument;
use git_domain::EditedIndexSessionId;

/// The visible columns in the three-version stage viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageViewerSide {
    Head,
    Index,
    Worktree,
}

/// Read-only UI projection for HEAD/Index/Worktree staging review.
///
/// The viewer deliberately stores no mutable Git state.  The middle column becomes eligible
/// for Apply only after the application layer binds an [`EditedIndexSessionId`]; the UI itself
/// never writes the real Index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreeVersionStageViewer {
    document: ThreeVersionDocument,
    active_side: StageViewerSide,
    edit_session: Option<EditedIndexSessionId>,
}

impl ThreeVersionStageViewer {
    #[must_use]
    pub fn new(document: ThreeVersionDocument) -> Self {
        Self {
            document,
            active_side: StageViewerSide::Index,
            edit_session: None,
        }
    }

    #[must_use]
    pub fn document(&self) -> &ThreeVersionDocument {
        &self.document
    }

    #[must_use]
    pub fn active_side(&self) -> StageViewerSide {
        self.active_side
    }

    pub fn select_side(&mut self, side: StageViewerSide) {
        self.active_side = side;
    }

    pub fn bind_edit_session(&mut self, session: EditedIndexSessionId) {
        self.edit_session = Some(session);
    }

    pub fn clear_edit_session(&mut self) {
        self.edit_session = None;
    }

    #[must_use]
    pub fn apply_session(&self) -> Option<EditedIndexSessionId> {
        self.edit_session
    }
}

#[cfg(test)]
mod tests {
    use git_diff::{ContentSource, ThreeVersionFile, VersionIdentity};
    use git_domain::RepoPath;

    use super::*;

    fn document() -> ThreeVersionDocument {
        let identity = |source| VersionIdentity {
            source,
            oid: None,
            mode: Some(0o100644),
        };
        ThreeVersionDocument {
            path: RepoPath::new("src/lib.rs").expect("fixture path is valid"),
            file: ThreeVersionFile {
                path: RepoPath::new("src/lib.rs").expect("fixture path is valid"),
                head: Some(identity(ContentSource::Head)),
                index: Some(identity(ContentSource::Index)),
                worktree: Some(identity(ContentSource::Worktree)),
            },
            head_to_index: None,
            index_to_worktree: None,
            head_to_worktree: None,
        }
    }

    #[test]
    fn apply_is_closed_until_an_edit_session_is_bound() {
        let mut viewer = ThreeVersionStageViewer::new(document());
        assert_eq!(viewer.active_side(), StageViewerSide::Index);
        assert!(viewer.apply_session().is_none());

        let session = EditedIndexSessionId::new();
        viewer.bind_edit_session(session);
        assert_eq!(viewer.apply_session(), Some(session));
        viewer.clear_edit_session();
        assert!(viewer.apply_session().is_none());
    }

    #[test]
    fn side_selection_does_not_mutate_the_three_version_document() {
        let mut viewer = ThreeVersionStageViewer::new(document());
        let before = viewer.document().clone();
        viewer.select_side(StageViewerSide::Worktree);
        assert_eq!(viewer.active_side(), StageViewerSide::Worktree);
        assert_eq!(viewer.document(), &before);
    }
}
