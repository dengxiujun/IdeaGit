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
                    OsString::from("--no-edit"),
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
    use git_ports::{ProcessOutput, ProcessSpec};
    use std::{
        str::FromStr,
        sync::{Arc, Mutex},
    };
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[derive(Clone)]
    struct RecordingRunner {
        args: Arc<Mutex<Vec<std::ffi::OsString>>>,
        success: bool,
    }

    #[async_trait]
    impl ProcessRunner for RecordingRunner {
        async fn run(
            &self,
            spec: ProcessSpec,
            _context: ExecutionContext,
        ) -> GitResult<ProcessOutput> {
            *self.args.lock().expect("test mutex") = spec.args().to_vec();
            Ok(ProcessOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
                exit_code: Some(if self.success { 0 } else { 1 }),
                success: self.success,
            })
        }
    }

    #[tokio::test]
    async fn builds_cherry_pick_without_shell_arguments() {
        let args = Arc::new(Mutex::new(Vec::new()));
        let writer = GitCliSequencerWriter::new(
            RecordingRunner {
                args: Arc::clone(&args),
                success: true,
            },
            RepositoryLocation::worktree(".", ".git", ".git"),
        );
        let (progress, _) = mpsc::channel(4);
        writer
            .apply(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                SequencerCommand::CherryPick {
                    commit: ObjectId::from_str("0123456789012345678901234567890123456789")
                        .expect("oid"),
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("cherry-pick");
        let args = args.lock().expect("test mutex");
        assert_eq!(args[0].to_string_lossy(), "cherry-pick");
        assert_eq!(args[1].to_string_lossy(), "--no-edit");
        assert_eq!(
            args[2].to_string_lossy(),
            "0123456789012345678901234567890123456789"
        );
    }

    #[tokio::test]
    async fn maps_sequencer_failure_to_recovery_actions() {
        let writer = GitCliSequencerWriter::new(
            RecordingRunner {
                args: Arc::new(Mutex::new(Vec::new())),
                success: false,
            },
            RepositoryLocation::worktree(".", ".git", ".git"),
        );
        let (progress, _) = mpsc::channel(4);
        let error = writer
            .apply(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                SequencerCommand::Revert {
                    commit: ObjectId::from_str("0123456789012345678901234567890123456789")
                        .expect("oid"),
                    no_edit: true,
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect_err("failure should be structured");
        assert_eq!(error.kind, GitErrorKind::MergeConflict);
        assert_eq!(
            error.recovery_actions,
            vec![RecoveryAction::ContinueMerge, RecoveryAction::AbortMerge]
        );
    }
}
