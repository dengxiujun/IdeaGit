use crate::{ObjectId, RepoPath, SessionTarget};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHistoryQuery {
    pub target: SessionTarget,
    pub path: RepoPath,
    pub limit: usize,
    pub skip: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHistoryEntry {
    pub commit: ObjectId,
    pub parent: Option<ObjectId>,
    pub path: RepoPath,
    pub author: Vec<u8>,
    pub timestamp: i64,
    pub subject: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    pub commit: ObjectId,
    pub author: Vec<u8>,
    pub timestamp: i64,
    pub original_line: u32,
    pub final_line: u32,
    pub content: Vec<u8>,
}
