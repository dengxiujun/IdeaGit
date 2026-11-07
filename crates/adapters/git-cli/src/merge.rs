//! Typed Git CLI implementation of merge execution.

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};
use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult, MergeCommand, RecoveryAction, SessionTarget};
use git_ports::{ExecutionContext, MergeWriter, ProcessRunner, RepositoryLocation};
use std::{ffi::OsString, path::PathBuf};

#[derive(Debug)]
pub struct GitCliMergeWriter<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliMergeWriter<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> MergeWriter for GitCliMergeWriter<R> {
    #[allow(clippy::result_large_err)]
    async fn apply(
        &self,
        target: SessionTarget,
        command: MergeCommand,
        context: ExecutionContext,
    ) -> GitResult<()> {
        let cwd = self
            .location
            .worktree_root()
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "merge is unavailable for a bare repository",
                )
                .with_target(target)
            })?
            .to_path_buf();
        let output = self
            .executor
            .execute(MergeMutation { cwd, command }, context.clone())
            .await?;
        if output.success {
            Ok(())
        } else {
            Err(
                GitError::new(GitErrorKind::MergeConflict, "Git merge did not complete")
                    .with_target(target)
                    .with_operation(context.operation_id)
                    .with_recovery_actions([
                        RecoveryAction::ContinueMerge,
                        RecoveryAction::AbortMerge,
                    ])
                    .with_diagnostic(format!("exit_code={:?}", output.exit_code)),
            )
        }
    }
}

struct MergeMutation {
    cwd: PathBuf,
    command: MergeCommand,
}
impl GitCliCommand for MergeMutation {}
impl sealed::Sealed for MergeMutation {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = vec![OsString::from("merge")];
        if self.command.no_ff {
            args.push(OsString::from("--no-ff"));
        }
        args.push(OsString::from(self.command.source.display_lossy().as_ref()));
        GitCliInvocation::new(None, args).with_cwd(self.cwd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{OperationId, RefName, RepositoryId, WorktreeId};
    use git_fixture::TestRepository;
    use std::process::Command;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn merges_a_real_branch_without_shell_interpolation() {
        let fixture = TestRepository::init().expect("fixture");
        fixture.commit_file("file.txt", b"base\n").expect("base");
        let branch_output = Command::new("git")
            .args([
                "-C",
                fixture.worktree.to_str().expect("utf8"),
                "branch",
                "--show-current",
            ])
            .output()
            .expect("git");
        assert!(branch_output.status.success());
        let branch = String::from_utf8_lossy(&branch_output.stdout)
            .trim()
            .to_owned();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .args(["-C", fixture.worktree.to_str().expect("utf8")])
                .args(args)
                .output()
                .expect("git");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["switch", "-c", "feature/merge"]);
        fixture
            .commit_file("feature.txt", b"feature\n")
            .expect("feature");
        git(&["switch", &branch]);
        let writer = GitCliMergeWriter::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        );
        let (progress, _) = mpsc::channel(4);
        writer
            .apply(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                MergeCommand {
                    source: RefName::new(b"feature/merge".to_vec()).expect("ref"),
                    no_ff: false,
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("merge");
        assert!(fixture.worktree.join("feature.txt").exists());
    }
}
