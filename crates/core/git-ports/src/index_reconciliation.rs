//! Safe publication contract for a verified real Git Index candidate.

use async_trait::async_trait;
use git_domain::{GitResult, SessionTarget};

use crate::ExecutionContext;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexReconciliationRequest {
    pub target: SessionTarget,
    pub expected_index_digest: [u8; 32],
    pub candidate_index: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexReconciliationOutcome {
    Published,
    Conflict,
    Unknown,
}

#[async_trait]
pub trait IndexReconciliationPublisher: Send + Sync {
    async fn publish(
        &self,
        request: IndexReconciliationRequest,
        context: ExecutionContext,
    ) -> GitResult<IndexReconciliationOutcome>;
}
