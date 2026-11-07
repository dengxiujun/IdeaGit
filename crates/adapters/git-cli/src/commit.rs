//! Typed Git CLI implementation of standard Index commits.

use std::{ffi::OsString, path::PathBuf, str::FromStr};

use async_trait::async_trait;
use git_domain::{
    GitError, GitErrorKind, GitResult, ObjectId, SessionTarget, SigningMode, StandardCommitRequest,
};
use git_ports::{
    CommitOutcome, CommitWriter, EnvironmentVariable, ExecutionContext, ProcessRunner,
    ProcessStdin, RepositoryLocation,
};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

#[derive(Debug)]
pub struct GitCliCommitWriter<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
    environment: Vec<EnvironmentVariable>,
}

impl<R> GitCliCommitWriter<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
            environment: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_environment(
        mut self,
        variables: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Self {
        self.environment = variables
            .into_iter()
            .map(|(name, value)| EnvironmentVariable::public(name, value))
            .collect();
        self
    }
}

#[async_trait]
impl<R: ProcessRunner> CommitWriter for GitCliCommitWriter<R> {
    #[allow(clippy::result_large_err)]
    async fn commit(
        &self,
        _target: SessionTarget,
        request: StandardCommitRequest,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        let cwd = self
            .location
            .worktree_root()
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "commit is unavailable for a bare repository",
                )
            })?
            .to_path_buf();
        let output = self
            .executor
            .execute(
                CommitCommand {
                    cwd: cwd.clone(),
                    request,
                    environment: self.environment.clone(),
                },
                context.clone(),
            )
            .await?;
        if !output.success {
            return Err(
                GitError::new(GitErrorKind::HookRejected, "Git commit was rejected")
                    .with_diagnostic(format!(
                        "exit_code={:?}; stderr={}; stdout={}",
                        output.exit_code,
                        String::from_utf8_lossy(&output.stderr),
                        String::from_utf8_lossy(&output.stdout)
                    )),
            );
        }
        let head = self
            .executor
            .execute(RevParseCommand { cwd: cwd.clone() }, context.clone())
            .await?;
        let parents = self
            .executor
            .execute(ParentsCommand { cwd }, context)
            .await?;
        let commit =
            ObjectId::from_str(String::from_utf8_lossy(&head.stdout).trim()).map_err(|error| {
                GitError::new(GitErrorKind::Internal, "Git returned an invalid commit OID")
                    .with_diagnostic(error.to_string())
            })?;
        let values = String::from_utf8_lossy(&parents.stdout)
            .split_whitespace()
            .skip(1)
            .map(|value| {
                ObjectId::from_str(value).map_err(|error| {
                    GitError::new(GitErrorKind::Internal, "Git returned an invalid parent OID")
                        .with_diagnostic(error.to_string())
                })
            })
            .collect::<GitResult<Vec<_>>>()?;
        Ok(CommitOutcome {
            commit,
            parents: values,
        })
    }
}

struct CommitCommand {
    cwd: PathBuf,
    request: StandardCommitRequest,
    environment: Vec<EnvironmentVariable>,
}
impl GitCliCommand for CommitCommand {}
impl sealed::Sealed for CommitCommand {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = vec![OsString::from("commit"), OsString::from("--file=-")];
        if self.request.options.amend {
            args.push(OsString::from("--amend"));
        }
        if self.request.options.allow_empty {
            args.push(OsString::from("--allow-empty"));
        }
        if self.request.options.signoff {
            args.push(OsString::from("--signoff"));
        }
        match self.request.options.signing {
            SigningMode::UseGitConfig => {}
            SigningMode::Disable => args.push(OsString::from("--no-gpg-sign")),
            SigningMode::Require => args.push(OsString::from("--gpg-sign")),
        }
        GitCliInvocation::new(None, args)
            .with_cwd(self.cwd)
            .with_environment(self.environment)
            .with_stdin(ProcessStdin::Bytes(self.request.options.message))
    }
}

struct RevParseCommand {
    cwd: PathBuf,
}
impl GitCliCommand for RevParseCommand {}
impl sealed::Sealed for RevParseCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(None, [OsString::from("rev-parse"), OsString::from("HEAD")])
            .with_cwd(self.cwd)
    }
}

struct ParentsCommand {
    cwd: PathBuf,
}
impl GitCliCommand for ParentsCommand {}
impl sealed::Sealed for ParentsCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("rev-list"),
                OsString::from("-1"),
                OsString::from("--parents"),
                OsString::from("HEAD"),
            ],
        )
        .with_cwd(self.cwd)
    }
}

#[cfg(test)]
mod tests {
    use git_domain::{CommitOptions, OperationId, RepositoryId, SnapshotRevision, WorktreeId};
    use git_fixture::TestRepository;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[tokio::test]
    async fn commits_real_index_and_returns_oid_and_parents() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("file.txt", b"one")
            .expect("initial commit");
        fixture.write_file("file.txt", b"two").expect("modify");
        fixture.stage_file("file.txt").expect("stage modification");
        let writer = GitCliCommitWriter::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        )
        .with_environment(fixture.isolated_environment());
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let (progress, _receiver) = mpsc::channel(4);
        let context = ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress);
        let outcome = writer
            .commit(
                target,
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"second\n".to_vec(),
                        amend: false,
                        allow_empty: false,
                        signoff: false,
                        author: None,
                        committer: None,
                        signing: SigningMode::Disable,
                    },
                    expected_revision: SnapshotRevision::new(1),
                    expected_head: None,
                    expected_index: None,
                },
                context,
            )
            .await
            .expect("commit should succeed");
        assert_eq!(outcome.parents.len(), 1);
    }
}
