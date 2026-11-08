use std::{ffi::OsString, path::PathBuf};

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult};
use git_ports::{
    ExecutionContext, ProcessRunner, ProcessStdin, RepositoryLocation, ShelfWriter, UnshelveRequest,
};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

#[derive(Debug)]
pub struct GitCliShelfWriter<R> {
    executor: GitCliExecutor<R>,
}

impl<R> GitCliShelfWriter<R> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> ShelfWriter for GitCliShelfWriter<R> {
    #[allow(clippy::result_large_err)]
    async fn unshelve(
        &self,
        location: &RepositoryLocation,
        request: UnshelveRequest,
        context: ExecutionContext,
    ) -> GitResult<()> {
        request
            .validate()
            .map_err(|error| GitError::new(GitErrorKind::StaleSelection, error.to_string()))?;
        let cwd = location
            .worktree_root()
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "unshelve is unavailable for a bare repository",
                )
            })?
            .to_path_buf();
        let mut patch = Vec::new();
        for bytes in request.patches {
            if !patch.is_empty() {
                patch.push(b'\n');
            }
            patch.extend(bytes);
        }
        let check = self
            .executor
            .execute(
                ApplyShelfPatchCommand {
                    cwd: cwd.clone(),
                    patch: patch.clone(),
                    check: true,
                },
                context.clone(),
            )
            .await?;
        if !check.success {
            return Err(GitError::new(
                GitErrorKind::PatchCheckFailed,
                "Git rejected the Shelf patch",
            ));
        }
        let applied = self
            .executor
            .execute(
                ApplyShelfPatchCommand {
                    cwd,
                    patch,
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
                "Git failed to apply the Shelf patch",
            ))
        }
    }
}

struct ApplyShelfPatchCommand {
    cwd: PathBuf,
    patch: Vec<u8>,
    check: bool,
}
impl GitCliCommand for ApplyShelfPatchCommand {}
impl sealed::Sealed for ApplyShelfPatchCommand {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = vec![OsString::from("apply")];
        if self.check {
            args.push(OsString::from("--check"));
        }
        GitCliInvocation::new(None, args)
            .with_cwd(self.cwd)
            .with_stdin(ProcessStdin::Bytes(self.patch))
    }
}
