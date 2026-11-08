#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
use std::{ffi::OsString, path::PathBuf};

use async_trait::async_trait;
use git_diff::{ContentSource, parse};
use git_domain::{GitError, GitErrorKind, GitResult, SnapshotRevision};
use git_ports::{
    DiffDocument, DiffReadRequest, DiffReader, EnvironmentVariable, ExecutionContext,
    RepositoryLocation,
};
use sha2::{Digest, Sha256};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

#[derive(Debug)]
pub struct GitCliDiffReader<R> {
    executor: GitCliExecutor<R>,
    environment: Vec<EnvironmentVariable>,
}

impl<R> GitCliDiffReader<R> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            environment: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_environment(
        mut self,
        environment: impl IntoIterator<Item = EnvironmentVariable>,
    ) -> Self {
        self.environment = environment.into_iter().collect();
        self
    }
}

struct DiffCommand {
    cwd: PathBuf,
    args: Vec<OsString>,
    environment: Vec<EnvironmentVariable>,
}
impl GitCliCommand for DiffCommand {}
impl sealed::Sealed for DiffCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(None, self.args)
            .with_cwd(self.cwd)
            .with_environment(self.environment)
    }
}

#[async_trait]
impl<R: git_ports::ProcessRunner> DiffReader for GitCliDiffReader<R> {
    #[allow(clippy::result_large_err)]
    async fn read(
        &self,
        location: &RepositoryLocation,
        request: DiffReadRequest,
        context: ExecutionContext,
    ) -> GitResult<DiffDocument> {
        let cwd = location.worktree_root().ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "diff reading requires a worktree",
            )
            .with_target(request.target)
            .with_operation(context.operation_id)
        })?;
        let comparison = comparison_args(request.before, request.after).ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "unsupported diff source pair",
            )
            .with_target(request.target)
            .with_operation(context.operation_id)
        })?;
        let mut args = vec![
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-color"),
            OsString::from(format!("--unified={}", request.context_lines)),
        ];
        args.extend(comparison);
        if !request.paths.is_empty() {
            args.push(OsString::from("--"));
            args.extend(
                request
                    .paths
                    .iter()
                    .map(|path| path_to_os_string(path.as_bytes())),
            );
        }
        let mut environment = vec![
            EnvironmentVariable::public("GIT_OPTIONAL_LOCKS", "0"),
            EnvironmentVariable::public("GIT_TERMINAL_PROMPT", "0"),
        ];
        environment.extend(self.environment.iter().cloned());
        let output = self
            .executor
            .execute(
                DiffCommand {
                    cwd: cwd.to_path_buf(),
                    args,
                    environment,
                },
                context.clone(),
            )
            .await?;
        if !output.success {
            return Err(
                GitError::new(GitErrorKind::PatchCheckFailed, "git diff failed")
                    .with_target(request.target)
                    .with_operation(context.operation_id),
            );
        }
        let patch = parse(&output.stdout).map_err(|error| {
            GitError::new(
                GitErrorKind::PatchCheckFailed,
                "git diff output is not a valid patch",
            )
            .with_diagnostic(error.to_string())
            .with_target(request.target)
            .with_operation(context.operation_id)
        })?;
        Ok(DiffDocument {
            generation: generation(&output.stdout),
            patch,
        })
    }
}

fn comparison_args(before: ContentSource, after: ContentSource) -> Option<Vec<OsString>> {
    match (before, after) {
        (ContentSource::Head, ContentSource::Index) => Some(vec![OsString::from("--cached")]),
        (ContentSource::Index, ContentSource::Worktree) => Some(Vec::new()),
        (ContentSource::Head, ContentSource::Worktree) => Some(vec![OsString::from("HEAD")]),
        _ => None,
    }
}

fn generation(bytes: &[u8]) -> SnapshotRevision {
    let digest = Sha256::digest(bytes);
    let mut value = [0_u8; 8];
    value.copy_from_slice(&digest[..8]);
    SnapshotRevision::new(u64::from_be_bytes(value))
}

#[cfg(unix)]
fn path_to_os_string(bytes: &[u8]) -> OsString {
    OsString::from_vec(bytes.to_vec())
}

#[cfg(windows)]
fn path_to_os_string(bytes: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TokioProcessRunner;
    use git_domain::{OperationId, RepoPath, RepositoryId, SessionTarget, WorktreeId};
    use git_fixture::TestRepository;
    use git_ports::{DiffReadRequest, DiffReader, ProcessProgress};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(2);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }
    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }
    fn location(fixture: &TestRepository) -> RepositoryLocation {
        RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        )
    }

    #[tokio::test]
    async fn reads_head_index_and_worktree_comparisons_from_a_real_repository() {
        let fixture = TestRepository::init().expect("fixture");
        fixture.commit_file("file.txt", b"base\n").expect("commit");
        fixture.write_file("file.txt", b"worktree\n").expect("edit");
        let reader = GitCliDiffReader::new(TokioProcessRunner).with_environment(
            fixture
                .isolated_environment()
                .into_iter()
                .map(|(name, value)| EnvironmentVariable::public(name, value)),
        );
        let path = RepoPath::try_from("file.txt").expect("path");
        let target = target();
        let request = |before, after| DiffReadRequest {
            target,
            before,
            after,
            paths: vec![path.clone()],
            context_lines: 3,
        };
        let head_worktree = reader
            .read(
                &location(&fixture),
                request(ContentSource::Head, ContentSource::Worktree),
                context(),
            )
            .await
            .expect("diff");
        assert_eq!(head_worktree.patch.files.len(), 1);
        fixture.stage_file("file.txt").expect("stage");
        let head_index = reader
            .read(
                &location(&fixture),
                request(ContentSource::Head, ContentSource::Index),
                context(),
            )
            .await
            .expect("diff");
        assert_eq!(head_index.patch.files.len(), 1);
        let index_worktree = reader
            .read(
                &location(&fixture),
                request(ContentSource::Index, ContentSource::Worktree),
                context(),
            )
            .await
            .expect("diff");
        assert!(index_worktree.patch.files.is_empty());
    }
}
