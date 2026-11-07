//! Commit history and graph views.
//!
//! Virtualized, pure projection for Git Log rows.

use git_domain::{CommitPage, ObjectId};
use git_graph::{GraphRow, layout};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRow {
    pub id: ObjectId,
    pub subject: Vec<u8>,
    pub author: Vec<u8>,
    pub timestamp: i64,
    pub graph: GraphRow,
}

#[must_use]
pub fn project(page: &CommitPage) -> Vec<LogRow> {
    let graph = layout(&page.commits);
    page.commits
        .iter()
        .zip(graph)
        .map(|(commit, graph)| LogRow {
            id: commit.id,
            subject: commit.subject.clone(),
            author: commit.author.clone(),
            timestamp: commit.timestamp,
            graph,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{CommitSummary, ObjectFormat, RepositoryId, SessionTarget, WorktreeId};

    fn oid(value: u8) -> ObjectId {
        ObjectId::from_bytes(ObjectFormat::Sha1, &[value; 20]).expect("oid")
    }

    #[test]
    fn projects_rows_without_creating_entities() {
        let page = CommitPage {
            target: SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
            commits: vec![CommitSummary {
                id: oid(1),
                parents: vec![],
                subject: b"subject".to_vec(),
                author: b"author".to_vec(),
                timestamp: 1,
            }],
            has_more: false,
        };
        let rows = project(&page);
        assert_eq!(rows[0].subject, b"subject");
        assert_eq!(rows[0].graph.commit, oid(1));
    }
}
