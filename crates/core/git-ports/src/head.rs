//! HEAD and branch tracking reader contracts.

use async_trait::async_trait;
use git_domain::{BranchTracking, GitResult, HeadState};

use crate::{ExecutionContext, RepositoryLocation};

#[async_trait]
pub trait HeadReader: Send + Sync {
    /// Reads the symbolic or detached `HEAD` state for one repository location.
    async fn read_head(
        &self,
        location: &RepositoryLocation,
        context: ExecutionContext,
    ) -> GitResult<HeadState>;
}

#[async_trait]
pub trait BranchReader: Send + Sync {
    /// Reads the current attached branch's upstream and divergence counts.
    ///
    /// Returns `None` when `HEAD` is detached or the branch has no upstream.
    async fn read_tracking(
        &self,
        location: &RepositoryLocation,
        context: ExecutionContext,
    ) -> GitResult<Option<BranchTracking>>;
}
