//! Repository discovery using only paths reported by Git itself.

#![allow(clippy::result_large_err)] // GitError layout is fixed by the shared Core contract.

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult, OperationId};
use git_ports::{
    EnvironmentVariable, ExecutionContext, ProcessOutput, ProcessRunner, RepositoryLocation,
    RepositoryReader,
};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

/// Git CLI implementation of repository discovery.
#[derive(Debug)]
pub struct GitCliRepositoryReader<R> {
    executor: GitCliExecutor<R>,
    environment: Vec<EnvironmentVariable>,
}

impl<R> GitCliRepositoryReader<R> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            environment: Vec::new(),
        }
    }

    /// Creates a reader for a Git executable selected by the platform composition root.
    #[must_use]
    pub fn with_executable(runner: R, executable: impl Into<OsString>) -> Self {
        Self {
            executor: GitCliExecutor::with_executable(runner, executable),
            environment: Vec::new(),
        }
    }

    /// Adds explicit environment overrides, primarily for fixture isolation and composition roots.
    #[must_use]
    pub fn with_environment(
        mut self,
        environment: impl IntoIterator<Item = EnvironmentVariable>,
    ) -> Self {
        self.environment = environment.into_iter().collect();
        self
    }
}

#[async_trait]
impl<R: ProcessRunner> RepositoryReader for GitCliRepositoryReader<R> {
    #[allow(clippy::result_large_err)]
    async fn discover(
        &self,
        path: PathBuf,
        context: ExecutionContext,
    ) -> GitResult<RepositoryLocation> {
        let operation_id = context.operation_id;
        let candidate = canonical_directory(path, operation_id).await?;
        let is_bare = self
            .run_rev_parse(&candidate, "--is-bare-repository", context.clone())
            .await
            .map_err(|error| classify_initial_discovery(error, operation_id))?;
        let is_bare = parse_bare_flag(&is_bare, operation_id)?;

        let git_dir = self
            .reported_path(&candidate, "--git-dir", context.clone())
            .await?;
        let common_dir = self
            .reported_path(&candidate, "--git-common-dir", context.clone())
            .await?;

        if is_bare {
            Ok(RepositoryLocation::bare(git_dir, common_dir))
        } else {
            let worktree_root = self
                .reported_path(&candidate, "--show-toplevel", context)
                .await?;
            Ok(RepositoryLocation::worktree(
                worktree_root,
                git_dir,
                common_dir,
            ))
        }
    }
}

impl<R: ProcessRunner> GitCliRepositoryReader<R> {
    #[allow(clippy::result_large_err)]
    async fn reported_path(
        &self,
        candidate: &Path,
        argument: &'static str,
        context: ExecutionContext,
    ) -> GitResult<PathBuf> {
        let operation_id = context.operation_id;
        let raw = self.run_rev_parse(candidate, argument, context).await?;
        let reported = path_from_output(raw, operation_id)?;
        let path = if reported.is_absolute() {
            reported
        } else {
            candidate.join(reported)
        };
        canonicalize_path(path, operation_id).await
    }

    #[allow(clippy::result_large_err)]
    async fn run_rev_parse(
        &self,
        candidate: &Path,
        argument: &'static str,
        context: ExecutionContext,
    ) -> GitResult<Vec<u8>> {
        let mut environment = vec![
            EnvironmentVariable::public("GIT_OPTIONAL_LOCKS", "0"),
            EnvironmentVariable::public("GIT_TERMINAL_PROMPT", "0"),
        ];
        environment.extend(self.environment.iter().cloned());
        let command = DiscoveryCommand {
            cwd: candidate.to_path_buf(),
            environment,
            argument,
        };
        let operation_id = context.operation_id;
        let output = self.executor.execute(command, context).await?;
        successful_stdout(output, argument, operation_id)
    }
}

struct DiscoveryCommand {
    cwd: PathBuf,
    environment: Vec<EnvironmentVariable>,
    argument: &'static str,
}

impl GitCliCommand for DiscoveryCommand {}

impl sealed::Sealed for DiscoveryCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("-c"),
                OsString::from("core.quotepath=false"),
                OsString::from("rev-parse"),
                OsString::from(self.argument),
            ],
        )
        .with_cwd(self.cwd)
        .with_environment(self.environment)
    }
}

fn successful_stdout(
    output: ProcessOutput,
    query: &'static str,
    operation_id: OperationId,
) -> GitResult<Vec<u8>> {
    if output.success {
        Ok(output.stdout)
    } else {
        Err(GitError::new(
            GitErrorKind::InvalidRepository,
            "Git repository query failed",
        )
        .with_operation(operation_id)
        .with_diagnostic(format!("query={query}; exit_code={:?}", output.exit_code)))
    }
}

fn classify_initial_discovery(mut error: GitError, operation_id: OperationId) -> GitError {
    if error.kind == GitErrorKind::InvalidRepository {
        error.kind = GitErrorKind::RepositoryNotFound;
        error.message = "path is not inside a Git repository".to_owned();
    }
    if error.operation_id.is_none() {
        error.operation_id = Some(operation_id);
    }
    error
}

async fn canonical_directory(path: PathBuf, operation_id: OperationId) -> GitResult<PathBuf> {
    let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
        filesystem_error(
            GitErrorKind::RepositoryNotFound,
            "repository path does not exist",
            operation_id,
            &error,
        )
    })?;
    if !metadata.is_dir() {
        return Err(GitError::new(
            GitErrorKind::InvalidRepository,
            "repository path is not a directory",
        )
        .with_operation(operation_id));
    }
    canonicalize_path(path, operation_id).await
}

async fn canonicalize_path(path: PathBuf, operation_id: OperationId) -> GitResult<PathBuf> {
    tokio::fs::canonicalize(path).await.map_err(|error| {
        let kind = if error.kind() == io::ErrorKind::PermissionDenied {
            GitErrorKind::PermissionDenied
        } else {
            GitErrorKind::InvalidRepository
        };
        filesystem_error(
            kind,
            "failed to normalize a Git-reported repository path",
            operation_id,
            &error,
        )
    })
}

fn parse_bare_flag(raw: &[u8], operation_id: OperationId) -> GitResult<bool> {
    match trim_ascii_line(raw) {
        b"true" => Ok(true),
        b"false" => Ok(false),
        _ => Err(GitError::new(
            GitErrorKind::InvalidRepository,
            "Git returned an invalid bare-repository flag",
        )
        .with_operation(operation_id)),
    }
}

fn path_from_output(mut raw: Vec<u8>, operation_id: OperationId) -> GitResult<PathBuf> {
    strip_one_line_ending(&mut raw);
    if raw.is_empty() || raw.contains(&0) {
        return Err(GitError::new(
            GitErrorKind::InvalidRepository,
            "Git returned an invalid repository path",
        )
        .with_operation(operation_id));
    }
    os_path_from_bytes(raw, operation_id)
}

fn trim_ascii_line(raw: &[u8]) -> &[u8] {
    let without_newline = raw.strip_suffix(b"\n").unwrap_or(raw);
    without_newline
        .strip_suffix(b"\r")
        .unwrap_or(without_newline)
}

fn strip_one_line_ending(raw: &mut Vec<u8>) {
    if raw.last() == Some(&b'\n') {
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
    }
}

#[cfg(unix)]
fn os_path_from_bytes(raw: Vec<u8>, _operation_id: OperationId) -> GitResult<PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    Ok(PathBuf::from(OsString::from_vec(raw)))
}

#[cfg(windows)]
fn os_path_from_bytes(raw: Vec<u8>, operation_id: OperationId) -> GitResult<PathBuf> {
    String::from_utf8(raw).map(PathBuf::from).map_err(|_| {
        GitError::new(
            GitErrorKind::UnsupportedEncoding,
            "Git returned a repository path that is not valid UTF-8 on Windows",
        )
        .with_operation(operation_id)
    })
}

fn filesystem_error(
    kind: GitErrorKind,
    message: &'static str,
    operation_id: OperationId,
    error: &io::Error,
) -> GitError {
    GitError::new(kind, message)
        .with_operation(operation_id)
        .with_diagnostic(format!(
            "io_kind={:?}; os_code={:?}",
            error.kind(),
            error.raw_os_error()
        ))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use git_domain::OperationId;
    use git_fixture::TestRepository;
    use git_ports::{ProcessProgress, RepositoryReader};
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::TokioProcessRunner;

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(32);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn reader(fixture: &TestRepository) -> GitCliRepositoryReader<TokioProcessRunner> {
        GitCliRepositoryReader::new(TokioProcessRunner).with_environment(
            fixture
                .isolated_environment()
                .into_iter()
                .map(|(name, value)| EnvironmentVariable::public(name, value)),
        )
    }

    async fn canonical(path: impl AsRef<Path>) -> PathBuf {
        tokio::fs::canonicalize(path)
            .await
            .expect("fixture path should canonicalize")
    }

    #[tokio::test]
    async fn discovers_ordinary_repository_from_nested_directory() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        let nested = fixture.worktree.join("nested").join("directory");
        fs::create_dir_all(&nested).expect("nested directory should be created");

        let location = reader(&fixture)
            .discover(nested, context())
            .await
            .expect("ordinary repository should be discovered");

        let expected_root = canonical(&fixture.worktree).await;
        assert_eq!(location.worktree_root(), Some(expected_root.as_path()));
        assert_eq!(location.git_dir(), expected_root.join(".git"));
        assert_eq!(location.common_dir(), expected_root.join(".git"));
        assert!(!location.is_bare());
    }

    #[tokio::test]
    async fn linked_worktrees_share_common_dir_but_not_git_dir() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("README.md", b"primary\n")
            .expect("fixture commit should succeed");
        let linked = fixture
            .add_linked_worktree()
            .expect("linked worktree should be created");
        let reader = reader(&fixture);

        let primary = reader
            .discover(fixture.worktree.clone(), context())
            .await
            .expect("primary worktree should be discovered");
        let linked_location = reader
            .discover(linked.clone(), context())
            .await
            .expect("linked worktree should be discovered");

        assert_eq!(
            linked_location.worktree_root(),
            Some(canonical(linked).await.as_path())
        );
        assert_eq!(primary.common_dir(), linked_location.common_dir());
        assert_ne!(primary.git_dir(), linked_location.git_dir());
    }

    #[tokio::test]
    async fn discovers_bare_repository_without_fake_worktree() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("README.md", b"bare remote seed\n")
            .expect("fixture commit should succeed");
        let bare = fixture
            .create_local_remote()
            .expect("bare remote should be created");

        let location = reader(&fixture)
            .discover(bare.path().to_path_buf(), context())
            .await
            .expect("bare repository should be discovered");
        let expected = canonical(bare.path()).await;

        assert!(location.is_bare());
        assert!(location.worktree_root().is_none());
        assert_eq!(location.git_dir(), expected);
        assert_eq!(location.common_dir(), expected);
    }

    #[tokio::test]
    async fn discovers_submodule_as_its_own_repository() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        let submodule = fixture
            .add_local_submodule()
            .expect("local submodule should be created");
        let superproject = reader(&fixture)
            .discover(fixture.worktree.clone(), context())
            .await
            .expect("superproject should be discovered");

        let location = reader(&fixture)
            .discover(submodule.clone(), context())
            .await
            .expect("submodule should be discovered");

        assert_eq!(
            location.worktree_root(),
            Some(canonical(submodule).await.as_path())
        );
        assert_ne!(location.common_dir(), superproject.common_dir());
        assert!(location.git_dir().ends_with("modules/fixture-submodule"));
        assert!(!location.is_bare());
    }

    #[tokio::test]
    async fn rejects_non_repository_missing_and_file_paths() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let unrelated_file = temp.path().join("file.txt");
        fs::write(&unrelated_file, b"not a repository").expect("test file should be written");
        let reader = GitCliRepositoryReader::new(TokioProcessRunner);

        let non_repository = reader
            .discover(temp.path().to_path_buf(), context())
            .await
            .expect_err("ordinary directory should not be accepted");
        assert_eq!(non_repository.kind, GitErrorKind::RepositoryNotFound);

        let missing = reader
            .discover(temp.path().join("missing"), context())
            .await
            .expect_err("missing directory should not be accepted");
        assert_eq!(missing.kind, GitErrorKind::RepositoryNotFound);

        let file = reader
            .discover(unrelated_file, context())
            .await
            .expect_err("file path should not be accepted as a repository root");
        assert_eq!(file.kind, GitErrorKind::InvalidRepository);
    }

    #[cfg(unix)]
    #[test]
    fn path_output_removes_only_the_git_record_terminator() {
        use std::os::unix::ffi::OsStrExt;

        let path = path_from_output(
            b"/tmp/directory with\nnewline\n".to_vec(),
            OperationId::new(),
        )
        .expect("raw Unix path should be preserved");

        assert_eq!(path.as_os_str().as_bytes(), b"/tmp/directory with\nnewline");
    }
}
