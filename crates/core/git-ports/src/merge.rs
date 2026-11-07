//! Merge mutation boundary.

use async_trait::async_trait;
use git_domain::{GitResult, MergeCommand, SessionTarget};

use crate::ExecutionContext;

/// Executes a typed merge against the selected repository session.
#[async_trait]
pub trait MergeWriter: Send + Sync {
    async fn apply(
        &self,
        target: SessionTarget,
        command: MergeCommand,
        context: ExecutionContext,
    ) -> GitResult<()>;
}
