//! Conflict-stage reader backed by porcelain v2 status.

use async_trait::async_trait;
use git_domain::{GitResult, StatusEntry, UnmergedChange};
use git_ports::{
    ConflictStageReader, ExecutionContext, ProcessRunner, RepositoryLocation, StatusOptions,
    StatusReader,
};

use crate::status::GitCliStatusReader;

#[derive(Debug)]
pub struct GitCliConflictStageReader<R> {
    status: GitCliStatusReader<R>,
}

impl<R> GitCliConflictStageReader<R> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self {
            status: GitCliStatusReader::new(runner),
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> ConflictStageReader for GitCliConflictStageReader<R> {
    async fn read(
        &self,
        location: &RepositoryLocation,
        context: ExecutionContext,
    ) -> GitResult<Vec<UnmergedChange>> {
        let status = self
            .status
            .status(
                location,
                StatusOptions {
                    include_untracked: false,
                    include_ignored: false,
                },
                context,
            )
            .await?;
        Ok(status
            .entries
            .into_iter()
            .filter_map(|entry| match entry {
                StatusEntry::Unmerged(change) => Some(change),
                _ => None,
            })
            .collect())
    }
}
