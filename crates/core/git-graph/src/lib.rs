//! Commit DAG layout and incremental lane allocation algorithms.
//!
//! Compact commit DAG lane projection for virtualized Log rows.

use git_domain::{CommitSummary, ObjectId, RefName};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRef {
    pub name: RefName,
    pub target: ObjectId,
    pub is_head: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphFilter {
    pub text: Option<Vec<u8>>,
    pub author: Option<Vec<u8>>,
    pub committed_after: Option<i64>,
    pub committed_before: Option<i64>,
}

impl GraphFilter {
    #[must_use]
    pub fn matches(&self, commit: &CommitSummary) -> bool {
        self.text
            .as_ref()
            .is_none_or(|text| contains_case_insensitive(&commit.subject, text))
            && self
                .author
                .as_ref()
                .is_none_or(|author| contains_case_insensitive(&commit.author, author))
            && self
                .committed_after
                .is_none_or(|timestamp| commit.timestamp >= timestamp)
            && self
                .committed_before
                .is_none_or(|timestamp| commit.timestamp <= timestamp)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCommit {
    pub summary: CommitSummary,
    pub refs: Vec<GraphRef>,
    pub is_head: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRow {
    pub commit: ObjectId,
    pub lane: usize,
    pub parent_lanes: Vec<usize>,
    pub refs: Vec<GraphRef>,
    pub is_head: bool,
}

/// Allocates stable lanes for one visible commit page without creating UI entities.
#[must_use]
pub fn layout(commits: &[CommitSummary]) -> Vec<GraphRow> {
    layout_with_refs(
        &commits
            .iter()
            .cloned()
            .map(|summary| GraphCommit {
                summary,
                refs: Vec::new(),
                is_head: false,
            })
            .collect::<Vec<_>>(),
    )
}

/// Filters commits and projects all refs pointing at the visible commits.
#[must_use]
pub fn filter_and_layout(commits: &[GraphCommit], filter: &GraphFilter) -> Vec<GraphRow> {
    let visible = commits
        .iter()
        .filter(|commit| filter.matches(&commit.summary))
        .cloned()
        .collect::<Vec<_>>();
    layout_with_refs(&visible)
}

/// Allocates lanes while preserving ref decorations and the HEAD marker.
#[must_use]
pub fn layout_with_refs(commits: &[GraphCommit]) -> Vec<GraphRow> {
    let mut lanes: Vec<ObjectId> = Vec::new();
    commits
        .iter()
        .map(|commit| {
            let lane = lanes
                .iter()
                .position(|id| *id == commit.summary.id)
                .unwrap_or_else(|| {
                    lanes.push(commit.summary.id);
                    lanes.len() - 1
                });
            lanes.remove(lane);
            let mut parent_lanes = Vec::new();
            for parent in &commit.summary.parents {
                let parent_lane = lanes
                    .iter()
                    .position(|id| *id == *parent)
                    .unwrap_or_else(|| {
                        lanes.push(*parent);
                        lanes.len() - 1
                    });
                parent_lanes.push(parent_lane);
            }
            GraphRow {
                commit: commit.summary.id,
                lane,
                parent_lanes,
                refs: commit.refs.clone(),
                is_head: commit.is_head || commit.refs.iter().any(|reference| reference.is_head),
            }
        })
        .collect()
}

fn contains_case_insensitive(value: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    value
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{ObjectFormat, ObjectId, RefName};

    fn oid(value: u8) -> ObjectId {
        ObjectId::from_bytes(ObjectFormat::Sha1, &[value; 20]).expect("oid")
    }

    #[test]
    fn preserves_parent_lane_for_linear_history() {
        let rows = layout(&[
            CommitSummary {
                id: oid(2),
                parents: vec![oid(1)],
                subject: vec![],
                author: vec![],
                timestamp: 0,
            },
            CommitSummary {
                id: oid(1),
                parents: vec![],
                subject: vec![],
                author: vec![],
                timestamp: 0,
            },
        ]);
        assert_eq!(rows[0].parent_lanes, vec![0]);
        assert_eq!(rows[1].lane, 0);
        assert!(rows[0].refs.is_empty());
    }

    #[test]
    fn filters_commits_and_projects_multiple_refs_and_head() {
        let main = RefName::new(b"refs/heads/main".to_vec()).expect("ref");
        let tag = RefName::new(b"refs/tags/v1".to_vec()).expect("ref");
        let commits = [
            GraphCommit {
                summary: CommitSummary {
                    id: oid(2),
                    parents: vec![oid(1)],
                    subject: b"Fix parser".to_vec(),
                    author: b"Alice".to_vec(),
                    timestamp: 20,
                },
                refs: vec![
                    GraphRef {
                        name: main,
                        target: oid(2),
                        is_head: true,
                    },
                    GraphRef {
                        name: tag,
                        target: oid(2),
                        is_head: false,
                    },
                ],
                is_head: false,
            },
            GraphCommit {
                summary: CommitSummary {
                    id: oid(1),
                    parents: vec![],
                    subject: b"Old change".to_vec(),
                    author: b"Bob".to_vec(),
                    timestamp: 10,
                },
                refs: vec![],
                is_head: false,
            },
        ];
        let rows = filter_and_layout(
            &commits,
            &GraphFilter {
                text: Some(b"parser".to_vec()),
                committed_after: Some(20),
                ..GraphFilter::default()
            },
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].commit, oid(2));
        assert!(rows[0].is_head);
        assert_eq!(rows[0].refs.len(), 2);
    }
}
