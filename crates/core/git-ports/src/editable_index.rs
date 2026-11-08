//! Candidate Index construction for editable Index sessions.

use std::path::PathBuf;

use async_trait::async_trait;
use git_domain::{GitResult, RepoPath, SessionTarget};

use crate::ExecutionContext;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditableIndexCandidateRequest {
    pub target: SessionTarget,
    pub path: RepoPath,
    pub mode: u32,
    pub content: Vec<u8>,
    pub temporary_index: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditableIndexCandidate {
    pub index_bytes: Vec<u8>,
    pub content: Vec<u8>,
}

#[async_trait]
pub trait EditableIndexCandidateBuilder: Send + Sync {
    async fn build(
        &self,
        request: EditableIndexCandidateRequest,
        context: ExecutionContext,
    ) -> GitResult<EditableIndexCandidate>;
}
