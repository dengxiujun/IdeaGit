//! Merge mutation boundary.

use async_trait::async_trait;
use git_domain::{GitResult, MergeCommand, SequencerCommand, SessionTarget};

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

/// Executes typed cherry-pick/revert sequencer mutations.
#[async_trait]
pub trait SequencerWriter: Send + Sync {
    async fn apply(
        &self,
        target: SessionTarget,
        command: SequencerCommand,
        context: ExecutionContext,
    ) -> GitResult<()>;
}
