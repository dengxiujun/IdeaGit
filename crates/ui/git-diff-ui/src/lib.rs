//! Diff viewers and staging presentation.

use git_diff::{AlignedDiffRow, Patch, ThreeVersionDocument, align_hunk};
use git_domain::EditedIndexSessionId;

/// Read-only side-by-side projection of a parsed patch.
///
/// The projection keeps the original [`Patch`] immutable and stores only logical row mappings;
/// rendering and scrolling layers can therefore use the same mapping without writing Git state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideBySideDiffViewer {
    patch: Patch,
    rows: Vec<AlignedDiffRow>,
    selected_hunk: Option<u32>,
}

impl SideBySideDiffViewer {
    #[must_use]
    pub fn new(patch: Patch) -> Self {
        let rows = patch
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .enumerate()
            .flat_map(|(hunk_id, hunk)| align_hunk(hunk_id as u32, hunk))
            .collect();
        Self {
            patch,
            rows,
            selected_hunk: None,
        }
    }

    #[must_use]
    pub fn patch(&self) -> &Patch {
        &self.patch
    }

    #[must_use]
    pub fn rows(&self) -> &[AlignedDiffRow] {
        &self.rows
    }

    #[must_use]
    pub fn selected_hunk(&self) -> Option<u32> {
        self.selected_hunk
    }

    /// Select a logical hunk without mutating the underlying patch or repository state.
    pub fn select_hunk(&mut self, hunk_id: Option<u32>) {
        self.selected_hunk = hunk_id.filter(|id| (*id as usize) < self.hunk_count());
    }

    #[must_use]
    fn hunk_count(&self) -> usize {
        self.patch.files.iter().map(|file| file.hunks.len()).sum()
    }
}

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
    use git_diff::{
        AlignedRowKind, ContentSource, FilePatch, Hunk, Line, LineKind, ThreeVersionFile,
        VersionIdentity,
    };
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

    #[test]
    fn side_by_side_projection_aligns_logical_rows_without_mutating_patch() {
        let patch = Patch {
            files: vec![FilePatch {
                old_path: Some(b"old.txt".to_vec()),
                new_path: Some(b"new.txt".to_vec()),
                binary: false,
                hunks: vec![Hunk {
                    old_start: 1,
                    old_count: 2,
                    new_start: 1,
                    new_count: 2,
                    lines: vec![
                        Line {
                            kind: LineKind::Removed,
                            old_line: Some(1),
                            new_line: None,
                            content: b"old\n".to_vec(),
                            has_newline: true,
                        },
                        Line {
                            kind: LineKind::Added,
                            old_line: None,
                            new_line: Some(1),
                            content: b"new\n".to_vec(),
                            has_newline: true,
                        },
                        Line {
                            kind: LineKind::Context,
                            old_line: Some(2),
                            new_line: Some(2),
                            content: b"same\n".to_vec(),
                            has_newline: true,
                        },
                    ],
                }],
            }],
            binary: false,
        };
        let original = patch.clone();
        let mut viewer = SideBySideDiffViewer::new(patch);

        assert_eq!(viewer.rows().len(), 2);
        assert_eq!(viewer.rows()[0].kind, AlignedRowKind::Modified);
        assert_eq!(viewer.rows()[1].kind, AlignedRowKind::Context);
        viewer.select_hunk(Some(0));
        assert_eq!(viewer.selected_hunk(), Some(0));
        viewer.select_hunk(Some(1));
        assert_eq!(viewer.selected_hunk(), None);
        assert_eq!(viewer.patch(), &original);
    }
}
