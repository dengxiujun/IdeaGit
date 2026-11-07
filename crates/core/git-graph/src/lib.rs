//! Commit DAG layout and incremental lane allocation algorithms.
//!
//! Compact commit DAG lane projection for virtualized Log rows.

use git_domain::{CommitSummary, ObjectId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRow {
    pub commit: ObjectId,
    pub lane: usize,
    pub parent_lanes: Vec<usize>,
}

/// Allocates stable lanes for one visible commit page without creating UI entities.
#[must_use]
pub fn layout(commits: &[CommitSummary]) -> Vec<GraphRow> {
    let mut lanes: Vec<ObjectId> = Vec::new();
    commits
        .iter()
        .map(|commit| {
            let lane = lanes
                .iter()
                .position(|id| *id == commit.id)
                .unwrap_or_else(|| {
                    lanes.push(commit.id);
                    lanes.len() - 1
                });
            lanes.remove(lane);
            let mut parent_lanes = Vec::new();
            for parent in &commit.parents {
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
                commit: commit.id,
                lane,
                parent_lanes,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{ObjectFormat, ObjectId};

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
    }
}
