//! Stable commit history projections.

use serde::{Deserialize, Serialize};

use crate::{ObjectId, SessionTarget};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogQuery {
    pub limit: usize,
    pub skip: usize,
}

impl Default for LogQuery {
    fn default() -> Self {
        Self {
            limit: 100,
            skip: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitSummary {
    pub id: ObjectId,
    pub parents: Vec<ObjectId>,
    pub subject: Vec<u8>,
    pub author: Vec<u8>,
    pub timestamp: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitPage {
    pub target: SessionTarget,
    pub commits: Vec<CommitSummary>,
    pub has_more: bool,
}
