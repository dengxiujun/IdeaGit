//! Interactive rebase planning ports.

use async_trait::async_trait;
use git_domain::{GitResult, InteractiveRebasePlan, ObjectId, SessionTarget};

use crate::ExecutionContext;

#[async_trait]
pub trait InteractiveRebasePlanReader: Send + Sync {
    async fn plan(
        &self,
        target: SessionTarget,
        upstream: ObjectId,
        context: ExecutionContext,
    ) -> GitResult<InteractiveRebasePlan>;
}

/// Executes a previously validated interactive rebase plan.
///
/// Implementations must provide the todo file through a typed, non-shell process boundary and
/// must not silently replace the supplied plan with Git's default editor contents.
#[async_trait]
pub trait InteractiveRebaseWriter: Send + Sync {
    async fn apply(
        &self,
        target: SessionTarget,
        plan: InteractiveRebasePlan,
        context: ExecutionContext,
    ) -> GitResult<()>;
}
