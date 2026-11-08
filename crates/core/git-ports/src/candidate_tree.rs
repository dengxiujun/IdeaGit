use std::path::PathBuf;

use async_trait::async_trait;
use git_domain::{GitResult, ObjectId};

use crate::ExecutionContext;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateTreeRequest {
    pub temporary_index: PathBuf,
    pub worktree_root: PathBuf,
    pub patch: Vec<u8>,
}

#[async_trait]
pub trait CandidateTreeBuilder: Send + Sync {
    async fn build(
        &self,
        request: CandidateTreeRequest,
        context: ExecutionContext,
    ) -> GitResult<ObjectId>;
}
