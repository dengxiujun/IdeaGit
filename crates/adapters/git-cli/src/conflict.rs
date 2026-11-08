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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GitCliRepositoryReader, TokioProcessRunner};
    use git_domain::OperationId;
    use git_fixture::TestRepository;
    use git_ports::{EnvironmentVariable, RepositoryReader};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel(16);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn environment(fixture: &TestRepository) -> Vec<EnvironmentVariable> {
        fixture
            .isolated_environment()
            .into_iter()
            .map(|(name, value)| EnvironmentVariable::public(name, value))
            .collect()
    }

    #[tokio::test]
    async fn reads_all_conflict_stage_identities_from_a_real_merge() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .create_conflict("conflict.txt", b"base\n", b"ours\n", b"theirs\n")
            .expect("fixture should create a merge conflict");
        let location = GitCliRepositoryReader::new(TokioProcessRunner)
            .with_environment(environment(&fixture))
            .discover(fixture.worktree.clone(), context())
            .await
            .expect("repository should be discovered");
        let changes = GitCliConflictStageReader::new(TokioProcessRunner)
            .read(&location, context())
            .await
            .expect("conflict stages should be readable");
        assert_eq!(changes.len(), 1);
        let change = &changes[0];
        assert_eq!(change.path.as_bytes(), b"conflict.txt");
        assert!(change.stage1_oid.is_some());
        assert!(change.stage2_oid.is_some());
        assert!(change.stage3_oid.is_some());
    }
}
