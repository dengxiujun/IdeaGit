//! Typed Git CLI implementation of Stage and Unstage.

use std::{ffi::OsString, path::PathBuf};

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult, RepoPath};
use git_ports::{ExecutionContext, IndexMutation, IndexWriter, ProcessRunner, RepositoryLocation};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

/// Index writer bound to one discovered repository location.
#[derive(Debug)]
pub struct GitCliIndexWriter<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliIndexWriter<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> IndexWriter for GitCliIndexWriter<R> {
    #[allow(clippy::result_large_err)]
    async fn apply(&self, mutation: IndexMutation, context: ExecutionContext) -> GitResult<()> {
        mutation
            .validate()
            .map_err(|error| GitError::new(GitErrorKind::InvalidPath, error.to_string()))?;
        let worktree = self.location.worktree_root().ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "Index mutation is unavailable for a bare repository",
            )
        })?;
        let (stage, paths) = match mutation {
            IndexMutation::Stage { paths, .. } => (true, paths),
            IndexMutation::Unstage { paths, .. } => (false, paths),
        };
        let args = paths_to_args(paths)?;
        let output = self
            .executor
            .execute(
                IndexCommand {
                    cwd: worktree.to_path_buf(),
                    stage,
                    paths: args,
                },
                context,
            )
            .await?;
        if output.success {
            Ok(())
        } else {
            Err(
                GitError::new(GitErrorKind::InvalidRepository, "Git Index mutation failed")
                    .with_diagnostic(format!("exit_code={:?}", output.exit_code)),
            )
        }
    }
}

#[allow(clippy::result_large_err)]
fn paths_to_args(paths: Vec<RepoPath>) -> GitResult<Vec<OsString>> {
    paths
        .into_iter()
        .map(|path| {
            String::from_utf8(path.as_bytes().to_vec())
                .map(OsString::from)
                .map_err(|_| {
                    GitError::new(GitErrorKind::UnsupportedEncoding, "Git path is not UTF-8")
                })
        })
        .collect()
}

struct IndexCommand {
    cwd: PathBuf,
    stage: bool,
    paths: Vec<OsString>,
}

impl GitCliCommand for IndexCommand {}

impl sealed::Sealed for IndexCommand {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = if self.stage {
            vec![OsString::from("add")]
        } else {
            vec![OsString::from("restore"), OsString::from("--staged")]
        };
        args.push(OsString::from("--"));
        args.extend(self.paths);
        GitCliInvocation::new(None, args).with_cwd(self.cwd)
    }
}
