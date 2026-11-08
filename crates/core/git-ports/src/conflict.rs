//! Conflict-stage inspection boundary.

use async_trait::async_trait;
use git_domain::{GitResult, UnmergedChange};

use crate::{ExecutionContext, RepositoryLocation};

/// Reads unmerged index entries and their stage 1/2/3 object identities.
#[async_trait]
pub trait ConflictStageReader: Send + Sync {
    async fn read(
        &self,
        location: &RepositoryLocation,
        context: ExecutionContext,
    ) -> GitResult<Vec<UnmergedChange>>;
}
