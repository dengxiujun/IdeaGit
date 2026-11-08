//! Git CLI conflict-result writer with read-before-write preflight.

use std::{fs, path::PathBuf};

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult, RepoPath};
use git_ports::{
    ApplyResolutionRequest, ConflictResolutionWriter, ConflictStageReader, ExecutionContext,
    IndexMutation, IndexWriter, ProcessRunner, RepositoryLocation, validate_resolution_request,
};

use crate::{GitCliConflictStageReader, GitCliIndexWriter};

#[derive(Debug)]
pub struct GitCliConflictResolutionWriter<R> {
    location: RepositoryLocation,
    stages: GitCliConflictStageReader<R>,
    index: GitCliIndexWriter<R>,
}

impl<R: Clone> GitCliConflictResolutionWriter<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            stages: GitCliConflictStageReader::new(runner.clone()),
            index: GitCliIndexWriter::new(runner, location.clone()),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner + Clone> ConflictResolutionWriter for GitCliConflictResolutionWriter<R> {
    #[allow(clippy::result_large_err)]
    async fn apply(
        &self,
        request: ApplyResolutionRequest,
        context: ExecutionContext,
    ) -> GitResult<()> {
        request
            .validate()
            .map_err(|error| GitError::new(GitErrorKind::InvalidPath, error.to_string()))?;
        let root = self.location.worktree_root().ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "conflict resolution is unavailable for a bare repository",
            )
        })?;
        let path = utf8_path(&request.path)?;
        let full_path = root.join(&path);
        let current = fs::read(&full_path).map_err(|error| {
            GitError::new(
                GitErrorKind::InvalidPath,
                "conflict worktree file could not be read",
            )
            .with_diagnostic(error.to_string())
        })?;
        let observed = self.stages.read(&self.location, context.clone()).await?;
        let observed = observed
            .iter()
            .find(|change| change.path == request.path)
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::StaleSelection,
                    "conflict path is no longer unresolved",
                )
            })?;
        validate_resolution_request(&request, observed, &current)
            .map_err(|error| GitError::new(GitErrorKind::StaleSelection, error.to_string()))?;

        fs::write(&full_path, &request.result).map_err(|error| {
            GitError::new(
                GitErrorKind::PermissionDenied,
                "resolved worktree file could not be written",
            )
            .with_diagnostic(error.to_string())
        })?;
        self.index
            .apply(
                IndexMutation::Stage {
                    target: request.target,
                    paths: vec![request.path],
                },
                context,
            )
            .await
    }
}

#[allow(clippy::result_large_err)]
fn utf8_path(path: &RepoPath) -> GitResult<PathBuf> {
    String::from_utf8(path.as_bytes().to_vec())
        .map(PathBuf::from)
        .map_err(|_| {
            GitError::new(
                GitErrorKind::UnsupportedEncoding,
                "conflict path is not UTF-8",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GitCliSequencerWriter;
    use git_domain::{RepositoryId, SequencerCommand, SessionTarget, SnapshotRevision, WorktreeId};
    use git_fixture::TestRepository;
    use git_ports::{ConflictStages, SequencerWriter, worktree_digest};
    use std::process::Command;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel(16);
        ExecutionContext::new(
            git_domain::OperationId::new(),
            CancellationToken::new(),
            progress,
        )
    }

    #[tokio::test]
    async fn writes_and_stages_a_real_conflict_resolution() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .configure_local_identity()
            .expect("fixture identity should configure");
        fixture
            .create_conflict("conflict.txt", b"base\n", b"ours\n", b"theirs\n")
            .expect("fixture should create conflict");
        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let path = RepoPath::try_from("conflict.txt").expect("valid path");
        let current = fs::read(fixture.worktree.join("conflict.txt")).expect("conflict file");
        let stages = GitCliConflictStageReader::new(crate::TokioProcessRunner)
            .read(&location, context())
            .await
            .expect("stages should be readable");
        let change = &stages[0];
        let writer = GitCliConflictResolutionWriter::new(crate::TokioProcessRunner, location);
        writer
            .apply(
                ApplyResolutionRequest {
                    target: SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                    path,
                    expected_conflict_revision: SnapshotRevision::INITIAL,
                    expected_stage_oids: ConflictStages {
                        stage1: change.stage1_oid,
                        stage2: change.stage2_oid,
                        stage3: change.stage3_oid,
                    },
                    expected_worktree_digest: worktree_digest(&current),
                    mode: 0o100644,
                    result: b"resolved\n".to_vec(),
                },
                context(),
            )
            .await
            .expect("resolution should be written and staged");

        GitCliSequencerWriter::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        )
        .apply(
            SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
            SequencerCommand::ContinueMerge,
            context(),
        )
        .await
        .expect("merge should continue after resolution");

        let status = Command::new("git")
            .args([
                "-C",
                fixture.worktree.to_str().expect("utf8"),
                "status",
                "--porcelain",
            ])
            .output()
            .expect("git status should run");
        assert!(!String::from_utf8_lossy(&status.stdout).starts_with("UU "));
        assert!(!fixture.worktree.join(".git/MERGE_HEAD").exists());
        assert_eq!(
            fs::read(fixture.worktree.join("conflict.txt")).expect("resolved file"),
            b"resolved\n"
        );
    }
}
