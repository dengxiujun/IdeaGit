//! Merge mutation boundary.

use async_trait::async_trait;
use git_domain::{GitResult, MergeCommand, SequencerCommand, SequencerSnapshot, SessionTarget};

use crate::{ExecutionContext, RepositoryLocation};

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

/// Reads active Merge/Rebase/Cherry-pick/Revert state without mutating Git.
#[async_trait]
pub trait SequencerReader: Send + Sync {
    async fn read(
        &self,
        location: &RepositoryLocation,
        context: ExecutionContext,
    ) -> GitResult<SequencerSnapshot>;
}
