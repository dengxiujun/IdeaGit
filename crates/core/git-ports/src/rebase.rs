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
