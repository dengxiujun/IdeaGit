//! Temporary index creation isolated from a repository's real index.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult, OperationId};
use git_ports::{
    EnvironmentVariable, ExecutionContext, RepositoryLocation, TemporaryIndexLease,
    TemporaryIndexManager,
};

use crate::{
    TokioProcessRunner,
    executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed},
};

#[derive(Debug)]
pub struct GitCliTemporaryIndexManager<R = TokioProcessRunner> {
    executor: GitCliExecutor<R>,
    root: PathBuf,
}

impl<R> GitCliTemporaryIndexManager<R> {
    #[must_use]
    pub fn new(executor: GitCliExecutor<R>, root: impl Into<PathBuf>) -> Self {
        Self {
            executor,
            root: root.into(),
        }
    }
}

impl GitCliTemporaryIndexManager<TokioProcessRunner> {
    #[must_use]
    pub fn system(executor: GitCliExecutor<TokioProcessRunner>) -> Self {
        Self::new(
            executor,
            std::env::temp_dir().join("ideagit-temporary-index"),
        )
    }
}

#[derive(Debug)]
struct ReadTreeCommand {
    index: PathBuf,
    cwd: PathBuf,
    unborn: bool,
}
impl GitCliCommand for ReadTreeCommand {}
impl sealed::Sealed for ReadTreeCommand {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = vec![OsString::from("read-tree")];
        args.push(if self.unborn {
            OsString::from("--empty")
        } else {
            OsString::from("HEAD")
        });
        GitCliInvocation::new(None, args)
            .with_cwd(self.cwd)
            .with_environment([EnvironmentVariable::public("GIT_INDEX_FILE", self.index)])
    }
}

#[async_trait]
impl<R: git_ports::ProcessRunner> TemporaryIndexManager for GitCliTemporaryIndexManager<R> {
    async fn create(
        &self,
        repository: &RepositoryLocation,
        operation: OperationId,
        context: ExecutionContext,
    ) -> GitResult<TemporaryIndexLease> {
        let cwd = repository.worktree_root().ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "temporary indexes require a worktree",
            )
            .with_operation(operation)
        })?;
        let directory = self
            .root
            .join(format!("ideagit-{}-{}", std::process::id(), operation));
        std::fs::create_dir_all(&directory).map_err(|error| {
            GitError::new(
                GitErrorKind::PermissionDenied,
                "failed to create temporary index directory",
            )
            .with_diagnostic(error.to_string())
            .with_operation(operation)
        })?;
        let path = directory.join("index");
        let unborn = head_is_unborn(repository.git_dir());
        let result = self
            .executor
            .execute(
                ReadTreeCommand {
                    index: path.clone(),
                    cwd: cwd.to_path_buf(),
                    unborn,
                },
                context,
            )
            .await;
        if let Err(error) = result {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_dir(&directory);
            return Err(error.with_operation(operation));
        }
        Ok(TemporaryIndexLease {
            path,
            transaction_id: operation,
        })
    }
}

fn head_is_unborn(git_dir: &Path) -> bool {
    let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) else {
        return true;
    };
    let Some(reference) = head.strip_prefix("ref: ").map(str::trim) else {
        return false;
    };
    if git_dir.join(reference).exists() {
        return false;
    }
    let Ok(packed_refs) = std::fs::read_to_string(git_dir.join("packed-refs")) else {
        return true;
    };
    !packed_refs.lines().any(|line| {
        line.split_once(' ')
            .is_some_and(|(_, name)| name == reference)
    })
}

#[allow(dead_code)]
fn _is_safe_path(path: &Path, repository: &RepositoryLocation) -> bool {
    path != repository.git_dir().join("index")
}

#[cfg(test)]
mod tests {
    use git_domain::OperationId;
    use git_ports::ProcessProgress;
    use std::process::Command;
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;

    fn context(operation: OperationId) -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(8);
        ExecutionContext::new(operation, CancellationToken::new(), progress)
    }

    fn git(repository: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repository)
            .status()
            .expect("git starts");
        assert!(status.success(), "git command failed: {args:?}");
    }

    #[tokio::test]
    async fn creates_and_drops_an_index_without_touching_real_index() {
        let temp = TempDir::new().expect("temp directory");
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).expect("repo directory");
        git(&root, &["init", "--quiet"]);
        git(&root, &["config", "user.email", "test@example.com"]);
        git(&root, &["config", "user.name", "IdeaGit Test"]);
        std::fs::write(root.join("tracked.txt"), "before\n").expect("file");
        git(&root, &["add", "tracked.txt"]);
        git(&root, &["commit", "--quiet", "-m", "initial"]);
        let git_dir = root.join(".git");
        let real_index = git_dir.join("index");
        let before = std::fs::read(&real_index).expect("real index");
        let repository = RepositoryLocation::worktree(&root, &git_dir, &git_dir);
        let operation = OperationId::new();
        let manager = GitCliTemporaryIndexManager::system(GitCliExecutor::new(TokioProcessRunner));
        let lease = manager
            .create(&repository, operation, context(operation))
            .await
            .expect("lease");
        assert_ne!(lease.path, real_index);
        assert!(lease.path.exists());
        assert_eq!(std::fs::read(&real_index).expect("real index"), before);
        let path = lease.path.clone();
        drop(lease);
        assert!(!path.exists(), "lease cleanup must remove temporary index");
    }

    #[test]
    fn detects_unborn_head() {
        let temp = TempDir::new().expect("temp directory");
        let git_dir = temp.path().join(".git");
        std::fs::create_dir(&git_dir).expect("git directory");
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").expect("head");
        assert!(head_is_unborn(&git_dir));
    }
}
