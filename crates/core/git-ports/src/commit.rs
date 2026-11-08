//! Standard commit write boundary.

use async_trait::async_trait;
use git_domain::{GitResult, ObjectId, OperationId, SessionTarget, StandardCommitRequest};

use crate::{ExecutionContext, TemporaryIndexLease};

/// Result observed after Git accepted a standard Index commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitOutcome {
    pub commit: ObjectId,
    pub parents: Vec<ObjectId>,
}

/// Adapter boundary for a real Index commit.
#[async_trait]
pub trait CommitWriter: Send + Sync {
    async fn commit(
        &self,
        target: SessionTarget,
        request: StandardCommitRequest,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome>;
}

/// Adapter boundary for a Partial Commit using the exact Temporary Index bound to its Preview.
#[async_trait]
pub trait PartialCommitWriter: Send + Sync {
    async fn commit(
        &self,
        target: SessionTarget,
        operation: OperationId,
        request: StandardCommitRequest,
        temporary_index: &TemporaryIndexLease,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome>;
}
