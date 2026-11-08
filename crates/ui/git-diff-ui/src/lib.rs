//! Diff viewers and staging presentation.
//!
//! INFRA-001 skeleton only; behavior is implemented by subsequent tracked tasks.

use git_diff::{AlignedDiffRow, Hunk, align_hunk};

/// Read-only side-by-side projection. It owns only logical row references; line content remains
/// in the shared Diff document and is not copied into each UI row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideBySideViewer {
    pub hunk_id: u32,
    pub rows: Vec<AlignedDiffRow>,
}

impl SideBySideViewer {
    #[must_use]
    pub fn from_hunk(hunk_id: u32, hunk: &Hunk) -> Self {
        Self {
            hunk_id,
            rows: align_hunk(hunk_id, hunk),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_diff::{Line, LineKind};

    #[test]
    fn projects_alignment_without_copying_line_content() {
        let hunk = Hunk {
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 2,
            lines: vec![
                Line {
                    kind: LineKind::Context,
                    old_line: Some(1),
                    new_line: Some(1),
                    content: b"same".to_vec(),
                    has_newline: true,
                },
                Line {
                    kind: LineKind::Added,
                    old_line: None,
                    new_line: Some(2),
                    content: b"added".to_vec(),
                    has_newline: true,
                },
            ],
        };
        let view = SideBySideViewer::from_hunk(7, &hunk);
        assert_eq!(view.hunk_id, 7);
        assert_eq!(view.rows.len(), 2);
        assert_eq!(view.rows[1].right.expect("added row").line_index, 1);
    }
}
