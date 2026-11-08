//! Clone operation boundary.

use async_trait::async_trait;
use git_domain::{GitResult, LocalPath};

use crate::ExecutionContext;

/// Explicitly authorized clone destination and source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloneRequest {
    pub source: String,
    pub destination: LocalPath,
}

#[async_trait]
pub trait CloneWriter: Send + Sync {
    async fn clone_repository(
        &self,
        request: CloneRequest,
        context: ExecutionContext,
    ) -> GitResult<()>;
}
