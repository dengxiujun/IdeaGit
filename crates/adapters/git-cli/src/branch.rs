//! Typed Git CLI implementation of branch create/checkout.

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};
use async_trait::async_trait;
use git_domain::{BranchCommand, GitError, GitErrorKind, GitResult, SessionTarget};
use git_ports::{BranchWriter, ExecutionContext, ProcessRunner, RepositoryLocation};
use std::{ffi::OsString, path::PathBuf};

#[derive(Debug)]
pub struct GitCliBranchWriter<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliBranchWriter<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> BranchWriter for GitCliBranchWriter<R> {
    #[allow(clippy::result_large_err)]
    async fn apply(
        &self,
        _target: SessionTarget,
        command: BranchCommand,
        context: ExecutionContext,
    ) -> GitResult<()> {
        let cwd = self
            .location
            .worktree_root()
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "branch mutation is unavailable for a bare repository",
                )
            })?
            .to_path_buf();
        let output = self
            .executor
            .execute(BranchMutation { cwd, command }, context)
            .await?;
        if output.success {
            Ok(())
        } else {
            Err(
                GitError::new(GitErrorKind::ReferenceChanged, "Git branch mutation failed")
                    .with_diagnostic(format!("exit_code={:?}", output.exit_code)),
            )
        }
    }
}

struct BranchMutation {
    cwd: PathBuf,
    command: BranchCommand,
}
impl GitCliCommand for BranchMutation {}
impl sealed::Sealed for BranchMutation {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = Vec::new();
        match self.command {
            BranchCommand::Create { name } => {
                args.extend([
                    OsString::from("switch"),
                    OsString::from("-c"),
                    OsString::from(name.display_lossy().as_ref()),
                ]);
            }
            BranchCommand::Checkout { name } => {
                args.extend([
                    OsString::from("switch"),
                    OsString::from(name.display_lossy().as_ref()),
                ]);
            }
            BranchCommand::Rename { from, to } => {
                args.extend([
                    OsString::from("branch"),
                    OsString::from("-m"),
                    OsString::from(from.display_lossy().as_ref()),
                    OsString::from(to.display_lossy().as_ref()),
                ]);
            }
            BranchCommand::Delete { name, force } => {
                args.extend([
                    OsString::from("branch"),
                    if force {
                        OsString::from("-D")
                    } else {
                        OsString::from("-d")
                    },
                    OsString::from(name.display_lossy().as_ref()),
                ]);
            }
        }
        GitCliInvocation::new(None, args).with_cwd(self.cwd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{BranchCommand, OperationId, RefName, RepositoryId, WorktreeId};
    use git_fixture::TestRepository;
    use std::process::Command;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn creates_and_checks_out_a_real_branch() {
        let fixture = TestRepository::init().expect("fixture");
        fixture.commit_file("file.txt", b"one").expect("commit");
        let writer = GitCliBranchWriter::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        );
        let (progress, _receiver) = mpsc::channel(4);
        let context = ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress);
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        writer
            .apply(
                target,
                BranchCommand::Create {
                    name: RefName::new(b"feature/demo".to_vec()).expect("name"),
                },
                context.clone(),
            )
            .await
            .expect("create");
        writer
            .apply(
                target,
                BranchCommand::Checkout {
                    name: RefName::new(b"main".to_vec()).expect("name"),
                },
                context,
            )
            .await
            .expect("checkout");
        let output = Command::new("git")
            .args([
                "-C",
                fixture.worktree.to_str().expect("utf8"),
                "branch",
                "--show-current",
            ])
            .output()
            .expect("git");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "main");
    }
}
