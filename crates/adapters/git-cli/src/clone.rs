//! Safe typed Git CLI clone implementation.

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};
use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult};
use git_ports::{CloneRequest, CloneWriter, ExecutionContext, ProcessRunner};
use std::{ffi::OsString, path::PathBuf};

#[derive(Debug)]
pub struct GitCliCloneWriter<R> {
    executor: GitCliExecutor<R>,
}

impl<R> GitCliCloneWriter<R> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> CloneWriter for GitCliCloneWriter<R> {
    #[allow(clippy::result_large_err)]
    async fn clone_repository(
        &self,
        request: CloneRequest,
        context: ExecutionContext,
    ) -> GitResult<()> {
        if request.source.trim().is_empty() {
            return Err(GitError::new(
                GitErrorKind::InvalidPath,
                "Clone source must not be empty",
            ));
        }
        let destination = request.destination.into_path_buf();
        if destination.as_os_str().is_empty() {
            return Err(GitError::new(
                GitErrorKind::InvalidPath,
                "Clone destination must not be empty",
            ));
        }
        if tokio::fs::try_exists(&destination).await.map_err(|error| {
            GitError::new(
                GitErrorKind::PermissionDenied,
                "Clone destination could not be checked",
            )
            .with_diagnostic(format!("io_kind={:?}", error.kind()))
        })? {
            return Err(GitError::new(
                GitErrorKind::InvalidPath,
                "Clone destination already exists",
            ));
        }
        let Some(parent) = destination.parent() else {
            return Err(GitError::new(
                GitErrorKind::InvalidPath,
                "Clone destination has no parent",
            ));
        };
        if !tokio::fs::try_exists(parent).await.map_err(|error| {
            GitError::new(
                GitErrorKind::PermissionDenied,
                "Clone destination parent could not be checked",
            )
            .with_diagnostic(format!("io_kind={:?}", error.kind()))
        })? {
            return Err(GitError::new(
                GitErrorKind::InvalidPath,
                "Clone destination parent does not exist",
            ));
        }
        let output = self
            .executor
            .execute(
                CloneMutation {
                    source: request.source,
                    destination,
                },
                context,
            )
            .await?;
        if output.success {
            Ok(())
        } else {
            Err(
                GitError::new(GitErrorKind::NetworkFailure, "Git clone failed")
                    .with_diagnostic(format!("exit_code={:?}", output.exit_code)),
            )
        }
    }
}

struct CloneMutation {
    source: String,
    destination: PathBuf,
}
impl GitCliCommand for CloneMutation {}
impl sealed::Sealed for CloneMutation {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("clone"),
                OsString::from(self.source),
                self.destination.into_os_string(),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::LocalPath;
    use git_fixture::TestRepository;
    use git_ports::CloneWriter;
    use std::path::PathBuf;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn clones_a_real_local_repository_into_a_new_directory() {
        let source = TestRepository::init().expect("source");
        source.commit_file("README.md", b"clone\n").expect("commit");
        let destination = source.temp_dir.path().join("cloned");
        let (progress, _) = mpsc::channel(4);
        GitCliCloneWriter::new(crate::TokioProcessRunner)
            .clone_repository(
                CloneRequest {
                    source: source.worktree.to_string_lossy().into_owned(),
                    destination: LocalPath::new(destination.clone()),
                },
                ExecutionContext::new(
                    git_domain::OperationId::new(),
                    CancellationToken::new(),
                    progress,
                ),
            )
            .await
            .expect("clone");
        assert!(destination.join("README.md").exists());
    }

    #[tokio::test]
    async fn rejects_existing_destination_before_spawning_git() {
        let temp = tempfile::tempdir().expect("tempdir");
        let destination = temp.path().join("existing");
        std::fs::create_dir(&destination).expect("destination");
        let (progress, _) = mpsc::channel(4);
        let error = GitCliCloneWriter::new(crate::TokioProcessRunner)
            .clone_repository(
                CloneRequest {
                    source: "file:///source".to_owned(),
                    destination: LocalPath::new(PathBuf::from(&destination)),
                },
                ExecutionContext::new(
                    git_domain::OperationId::new(),
                    CancellationToken::new(),
                    progress,
                ),
            )
            .await
            .expect_err("existing destination should be rejected");
        assert_eq!(error.kind, GitErrorKind::InvalidPath);
    }
}
