//! Remote configuration and network-operation boundaries.

use async_trait::async_trait;
use git_domain::{
    GitResult, PushMode, PushPreview, RefName, RemoteCommand, RemoteConfig, SessionTarget,
};

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

#[async_trait]
pub trait PushPreviewReader: Send + Sync {
    async fn preview_push(
        &self,
        target: SessionTarget,
        remote: RefName,
        branch: RefName,
        mode: PushMode,
        context: ExecutionContext,
    ) -> GitResult<PushPreview>;
}
