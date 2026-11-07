//! Typed Git CLI implementation of cherry-pick and revert.

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};
use async_trait::async_trait;
use git_domain::{
    GitError, GitErrorKind, GitResult, RecoveryAction, SequencerCommand, SessionTarget,
};
use git_ports::{
    EnvironmentVariable, ExecutionContext, ProcessRunner, RepositoryLocation, SequencerWriter,
};
use std::{ffi::OsString, path::PathBuf};

#[derive(Debug)]
pub struct GitCliSequencerWriter<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliSequencerWriter<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> SequencerWriter for GitCliSequencerWriter<R> {
    #[allow(clippy::result_large_err)]
    async fn apply(
        &self,
        target: SessionTarget,
        command: SequencerCommand,
        context: ExecutionContext,
    ) -> GitResult<()> {
        let cwd = self
            .location
            .worktree_root()
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "sequencer mutation is unavailable for a bare repository",
                )
                .with_target(target)
            })?
            .to_path_buf();
        let output = self
            .executor
            .execute(SequencerMutation { cwd, command }, context.clone())
            .await?;
        if output.success {
            Ok(())
        } else {
            Err(GitError::new(
                GitErrorKind::MergeConflict,
                "Git sequencer operation did not complete",
            )
            .with_target(target)
            .with_operation(context.operation_id)
            .with_recovery_actions([RecoveryAction::ContinueMerge, RecoveryAction::AbortMerge])
            .with_diagnostic(format!("exit_code={:?}", output.exit_code)))
        }
    }
}

struct SequencerMutation {
    cwd: PathBuf,
    command: SequencerCommand,
}
impl GitCliCommand for SequencerMutation {}
impl sealed::Sealed for SequencerMutation {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = Vec::new();
        match self.command {
            SequencerCommand::CherryPick { commit } => {
                args.extend([
                    OsString::from("cherry-pick"),
                    OsString::from(commit.to_string()),
                ]);
            }
            SequencerCommand::Revert { commit, no_edit } => {
                args.push(OsString::from("revert"));
                if no_edit {
                    args.push(OsString::from("--no-edit"));
                }
                args.push(OsString::from(commit.to_string()));
            }
            SequencerCommand::Continue => {
                args.extend([OsString::from("cherry-pick"), OsString::from("--continue")])
            }
            SequencerCommand::Skip => {
                args.extend([OsString::from("cherry-pick"), OsString::from("--skip")])
            }
            SequencerCommand::Abort => {
                args.extend([OsString::from("cherry-pick"), OsString::from("--abort")])
            }
        }
        GitCliInvocation::new(None, args)
            .with_environment([
                EnvironmentVariable::public("GIT_EDITOR", "true"),
                EnvironmentVariable::public("GIT_TERMINAL_PROMPT", "0"),
                EnvironmentVariable::public("GIT_OPTIONAL_LOCKS", "0"),
            ])
            .with_cwd(self.cwd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{ObjectId, OperationId, RepositoryId, SessionTarget, WorktreeId};
    use git_fixture::TestRepository;
    use std::{process::Command, str::FromStr};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn cherry_picks_a_real_commit() {
        let fixture = TestRepository::init().expect("fixture");
        fixture.commit_file("base.txt", b"base\n").expect("base");
        let initial_branch = String::from_utf8_lossy(
            &Command::new("git")
                .args([
                    "-C",
                    fixture.worktree.to_str().expect("utf8"),
                    "branch",
                    "--show-current",
                ])
                .output()
                .expect("git")
                .stdout,
        )
        .trim()
        .to_owned();
        assert!(!initial_branch.is_empty());
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
        git(&["switch", "-c", "feature/cherry"]);
        fixture
            .commit_file("cherry.txt", b"cherry\n")
            .expect("feature");
        let oid = String::from_utf8_lossy(
            &Command::new("git")
                .args([
                    "-C",
                    fixture.worktree.to_str().expect("utf8"),
                    "rev-parse",
                    "HEAD",
                ])
                .output()
                .expect("git")
                .stdout,
        )
        .trim()
        .to_owned();
        git(&["switch", &initial_branch]);
        let writer = GitCliSequencerWriter::new(
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
                SequencerCommand::CherryPick {
                    commit: ObjectId::from_str(&oid).expect("oid"),
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("cherry-pick");
        assert!(fixture.worktree.join("cherry.txt").exists());
    }
}
