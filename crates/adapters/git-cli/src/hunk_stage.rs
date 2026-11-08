//! Safe, preflighted application of a caller-built hunk patch to the real Index.

use std::{ffi::OsString, path::PathBuf};

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult};
use git_ports::{
    ExecutionContext, HunkStageStateReader, HunkStageWriter, ProcessRunner, ProcessStdin,
    RepositoryLocation, StageHunksRequest,
};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

#[derive(Debug)]
pub struct GitCliHunkStageWriter<R, S> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
    state_reader: S,
}

impl<R, S> GitCliHunkStageWriter<R, S> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation, state_reader: S) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
            state_reader,
        }
    }
}

#[async_trait]
impl<R, S> HunkStageWriter for GitCliHunkStageWriter<R, S>
where
    R: ProcessRunner,
    S: HunkStageStateReader,
{
    #[allow(clippy::result_large_err)]
    async fn stage_hunks(
        &self,
        request: StageHunksRequest,
        context: ExecutionContext,
    ) -> GitResult<()> {
        if request.hunks.is_empty() || request.patch.is_empty() {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "hunk stage requires a non-empty selection and patch",
            ));
        }
        let observed = self
            .state_reader
            .read(request.target, &request.path, context.clone())
            .await?;
        if observed.index_oid != request.expected_index_oid {
            return Err(GitError::new(
                GitErrorKind::IndexChangedExternally,
                "Index changed since hunk selection",
            ));
        }
        if observed.worktree_fingerprint != request.expected_worktree_fingerprint {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "Worktree changed since hunk selection",
            ));
        }
        let cwd = self
            .location
            .worktree_root()
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "hunk stage is unavailable for a bare repository",
                )
            })?
            .to_path_buf();
        let check = self
            .executor
            .execute(
                ApplyPatchCommand {
                    cwd: cwd.clone(),
                    patch: request.patch.clone(),
                    check: true,
                },
                context.clone(),
            )
            .await?;
        if !check.success {
            return Err(GitError::new(
                GitErrorKind::PatchCheckFailed,
                "Git rejected the selected hunk patch",
            ));
        }
        let applied = self
            .executor
            .execute(
                ApplyPatchCommand {
                    cwd,
                    patch: request.patch,
                    check: false,
                },
                context,
            )
            .await?;
        if applied.success {
            Ok(())
        } else {
            Err(GitError::new(
                GitErrorKind::PatchCheckFailed,
                "Git failed to apply the selected hunk patch",
            ))
        }
    }
}

struct ApplyPatchCommand {
    cwd: PathBuf,
    patch: Vec<u8>,
    check: bool,
}
impl GitCliCommand for ApplyPatchCommand {}
impl sealed::Sealed for ApplyPatchCommand {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = vec![OsString::from("apply"), OsString::from("--cached")];
        if self.check {
            args.push(OsString::from("--check"));
        }
        GitCliInvocation::new(None, args)
            .with_cwd(self.cwd)
            .with_stdin(ProcessStdin::Bytes(self.patch))
    }
}

#[cfg(test)]
mod tests {
    use std::{process::Command, str::FromStr};

    use git_domain::{
        HunkSelection, ObjectId, OperationId, RepoPath, RepositoryId, SessionTarget, WorktreeId,
    };
    use git_fixture::TestRepository;
    use git_ports::{HunkStageStateReader, HunkStageWriter, StageHunksRequest, StageHunksState};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[derive(Clone, Copy)]
    struct FixedState(StageHunksState);

    #[async_trait]
    impl HunkStageStateReader for FixedState {
        async fn read(
            &self,
            _target: SessionTarget,
            _path: &RepoPath,
            _context: ExecutionContext,
        ) -> GitResult<StageHunksState> {
            Ok(self.0)
        }
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel(4);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn request(fingerprint: [u8; 32], patch: &[u8]) -> StageHunksRequest {
        StageHunksRequest {
            target: target(),
            path: RepoPath::try_from("file.txt").expect("valid path"),
            expected_index_oid: None,
            expected_worktree_fingerprint: fingerprint,
            hunks: vec![HunkSelection { hunk_id: 0 }],
            patch: patch.to_vec(),
        }
    }

    #[tokio::test]
    async fn applies_a_checked_hunk_patch_to_a_real_index() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("file.txt", b"base\n")
            .expect("commit should succeed");
        let patch = b"diff --git a/file.txt b/file.txt\n--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-base\n+selected\n";
        let fingerprint = [7; 32];
        let writer = GitCliHunkStageWriter::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
            FixedState(StageHunksState {
                index_oid: None,
                worktree_fingerprint: fingerprint,
            }),
        );
        fixture
            .write_file("file.txt", b"selected\n")
            .expect("edit should succeed");
        writer
            .stage_hunks(request(fingerprint, patch), context())
            .await
            .expect("hunk should stage");
        let output = Command::new("git")
            .args([
                "-C",
                fixture.worktree.to_str().expect("utf8"),
                "diff",
                "--cached",
                "--",
                "file.txt",
            ])
            .output()
            .expect("git should run");
        assert!(String::from_utf8_lossy(&output.stdout).contains("+selected"));
    }

    #[tokio::test]
    async fn rejects_stale_index_before_git_apply() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("file.txt", b"base\n")
            .expect("commit should succeed");
        let writer = GitCliHunkStageWriter::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
            FixedState(StageHunksState {
                index_oid: Some(
                    ObjectId::from_str("0000000000000000000000000000000000000000").expect("oid"),
                ),
                worktree_fingerprint: [1; 32],
            }),
        );
        let result = writer
            .stage_hunks(request([1; 32], b"invalid patch"), context())
            .await;
        assert_eq!(
            result.expect_err("stale index should fail").kind,
            GitErrorKind::IndexChangedExternally
        );
    }

    #[tokio::test]
    async fn rejects_stale_worktree_before_git_apply() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("file.txt", b"base\n")
            .expect("commit should succeed");
        let writer = GitCliHunkStageWriter::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
            FixedState(StageHunksState {
                index_oid: None,
                worktree_fingerprint: [2; 32],
            }),
        );
        let result = writer
            .stage_hunks(request([1; 32], b"invalid patch"), context())
            .await;
        assert_eq!(
            result.expect_err("stale worktree should fail").kind,
            GitErrorKind::StaleSelection
        );
    }
}
