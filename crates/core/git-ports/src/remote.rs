//! Remote configuration and network-operation boundaries.

use async_trait::async_trait;
use git_domain::{GitResult, RemoteCommand, RemoteConfig, SessionTarget};

use crate::{ExecutionContext, RepositoryLocation};

#[async_trait]
pub trait RemoteReader: Send + Sync {
    async fn list(
        &self,
        location: &RepositoryLocation,
        context: ExecutionContext,
    ) -> GitResult<Vec<RemoteConfig>>;
}

#[async_trait]
pub trait RemoteWriter: Send + Sync {
    async fn apply(
        &self,
        target: SessionTarget,
        command: RemoteCommand,
        context: ExecutionContext,
    ) -> GitResult<()>;
}
