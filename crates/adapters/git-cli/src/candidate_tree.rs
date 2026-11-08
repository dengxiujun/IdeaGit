use std::{ffi::OsString, path::PathBuf};

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult, ObjectId};
use git_ports::{
    CandidateTreeBuilder, CandidateTreeRequest, EnvironmentVariable, ExecutionContext,
};

use crate::{
    TokioProcessRunner,
    executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed},
};

#[derive(Debug)]
pub struct GitCliCandidateTreeBuilder<R = TokioProcessRunner> {
    executor: GitCliExecutor<R>,
}

impl<R> GitCliCandidateTreeBuilder<R> {
    #[must_use]
    pub fn new(executor: GitCliExecutor<R>) -> Self {
        Self { executor }
    }
}

struct ApplyPatchCommand {
    cwd: PathBuf,
    index: PathBuf,
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
            .with_environment([EnvironmentVariable::public("GIT_INDEX_FILE", self.index)])
            .with_stdin(git_ports::ProcessStdin::Bytes(self.patch))
    }
}

struct WriteTreeCommand {
    cwd: PathBuf,
    index: PathBuf,
}
impl GitCliCommand for WriteTreeCommand {}
impl sealed::Sealed for WriteTreeCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(None, [OsString::from("write-tree")])
            .with_cwd(self.cwd)
            .with_environment([EnvironmentVariable::public("GIT_INDEX_FILE", self.index)])
    }
}

#[async_trait]
impl<R: git_ports::ProcessRunner> CandidateTreeBuilder for GitCliCandidateTreeBuilder<R> {
    async fn build(
        &self,
        request: CandidateTreeRequest,
        context: ExecutionContext,
    ) -> GitResult<ObjectId> {
        if request.patch.is_empty() {
            return Err(GitError::new(
                GitErrorKind::InvalidPath,
                "candidate patch must not be empty",
            ));
        }
        self.executor
            .execute(
                ApplyPatchCommand {
                    cwd: request.worktree_root.clone(),
                    index: request.temporary_index.clone(),
                    patch: request.patch.clone(),
                    check: true,
                },
                context.clone(),
            )
            .await?;
        self.executor
            .execute(
                ApplyPatchCommand {
                    cwd: request.worktree_root.clone(),
                    index: request.temporary_index.clone(),
                    patch: request.patch,
                    check: false,
                },
                context.clone(),
            )
            .await?;
        let output = self
            .executor
            .execute(
                WriteTreeCommand {
                    cwd: request.worktree_root,
                    index: request.temporary_index,
                },
                context,
            )
            .await?;
        let text = std::str::from_utf8(&output.stdout).map_err(|_| {
            GitError::new(
                GitErrorKind::Internal,
                "git write-tree returned non-UTF-8 output",
            )
        })?;
        text.trim().parse().map_err(|_| {
            GitError::new(
                GitErrorKind::Internal,
                "git write-tree returned an invalid object id",
            )
        })
    }
}
