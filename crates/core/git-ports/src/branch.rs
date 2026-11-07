//! Branch mutation boundary.

use async_trait::async_trait;
use git_domain::{BranchCommand, GitResult, SessionTarget};

use crate::ExecutionContext;

#[async_trait]
pub trait BranchWriter: Send + Sync {
    async fn apply(
        &self,
        target: SessionTarget,
        command: BranchCommand,
        context: ExecutionContext,
    ) -> GitResult<()>;
}
