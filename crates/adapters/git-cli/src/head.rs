//! HEAD and upstream readers built from stable Git plumbing commands.

#![allow(clippy::result_large_err)] // GitError layout is fixed by the shared Core contract.

use std::{ffi::OsString, path::PathBuf, str};

use async_trait::async_trait;
use git_domain::{
    BranchTracking, GitError, GitErrorKind, GitResult, HeadState, ObjectId, OperationId, RefName,
};
use git_ports::{
    BranchReader, EnvironmentVariable, ExecutionContext, HeadReader, ProcessOutput, ProcessRunner,
    RepositoryLocation,
};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

/// Git CLI implementation of HEAD and current-branch tracking reads.
#[derive(Debug)]
pub struct GitCliHeadReader<R> {
    executor: GitCliExecutor<R>,
    environment: Vec<EnvironmentVariable>,
}

impl<R> GitCliHeadReader<R> {
    /// Creates a reader using the platform-selected `git` executable.
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            environment: Vec::new(),
        }
    }

    /// Creates a reader using an explicit Git executable.
    #[must_use]
    pub fn with_executable(runner: R, executable: impl Into<OsString>) -> Self {
        Self {
            executor: GitCliExecutor::with_executable(runner, executable),
            environment: Vec::new(),
        }
    }

    /// Adds environment overrides supplied by the composition root or a test fixture.
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
impl<R: ProcessRunner> HeadReader for GitCliHeadReader<R> {
    async fn read_head(
        &self,
        location: &RepositoryLocation,
        context: ExecutionContext,
    ) -> GitResult<HeadState> {
        let symbolic = self
            .run(
                location,
                ["symbolic-ref", "--quiet", "--short", "HEAD"],
                context.clone(),
            )
            .await?;
        let oid = self
            .run(location, ["rev-parse", "--verify", "HEAD"], context.clone())
            .await?;

        match (symbolic.success, oid.success) {
            (true, true) => Ok(HeadState::Attached {
                branch: parse_ref_line(symbolic.stdout, context.operation_id, "HEAD branch")?,
                oid: parse_oid_line(oid.stdout, context.operation_id, "HEAD OID")?,
            }),
            (true, false) => Ok(HeadState::Unborn {
                branch: parse_ref_line(symbolic.stdout, context.operation_id, "HEAD branch")?,
            }),
            (false, true) if symbolic.exit_code == Some(1) => Ok(HeadState::Detached {
                oid: parse_oid_line(oid.stdout, context.operation_id, "HEAD OID")?,
            }),
            _ => Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "failed to resolve repository HEAD",
            )
            .with_operation(context.operation_id)
            .with_diagnostic(format!(
                "symbolic_exit={:?}; oid_exit={:?}",
                symbolic.exit_code, oid.exit_code
            ))),
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> BranchReader for GitCliHeadReader<R> {
    async fn read_tracking(
        &self,
        location: &RepositoryLocation,
        context: ExecutionContext,
    ) -> GitResult<Option<BranchTracking>> {
        let symbolic = self
            .run(
                location,
                ["symbolic-ref", "--quiet", "HEAD"],
                context.clone(),
            )
            .await?;
        if !symbolic.success {
            return if symbolic.exit_code == Some(1) {
                Ok(None)
            } else {
                Err(command_failure(
                    "failed to resolve symbolic HEAD",
                    context.operation_id,
                    &symbolic,
                ))
            };
        }
        let head_ref = parse_ref_line(symbolic.stdout, context.operation_id, "HEAD ref")?;
        let head_argument = os_string_from_bytes(
            head_ref.as_bytes().to_vec(),
            context.operation_id,
            "HEAD ref",
        )?;
        let upstream = self
            .run_os(
                location,
                [
                    OsString::from("for-each-ref"),
                    OsString::from("--format=%(upstream)"),
                    head_argument,
                ],
                context.clone(),
            )
            .await?;
        if !upstream.success {
            return Err(command_failure(
                "failed to resolve branch upstream",
                context.operation_id,
                &upstream,
            ));
        }
        let upstream_bytes = strip_one_line_ending(upstream.stdout);
        if upstream_bytes.is_empty() {
            return Ok(None);
        }
        if upstream_bytes.contains(&b'\n') || upstream_bytes.contains(&b'\r') {
            return Err(invalid_output(
                context.operation_id,
                "upstream ref contains multiple records",
            ));
        }
        let upstream = RefName::new(upstream_bytes).map_err(|error| {
            invalid_output(
                context.operation_id,
                &format!("invalid upstream ref: {error}"),
            )
        })?;

        let mut range = b"HEAD...".to_vec();
        range.extend_from_slice(upstream.as_bytes());
        let range = os_string_from_bytes(range, context.operation_id, "revision range")?;
        let counts = self
            .run_os(
                location,
                [
                    OsString::from("rev-list"),
                    OsString::from("--left-right"),
                    OsString::from("--count"),
                    range,
                ],
                context.clone(),
            )
            .await?;
        if !counts.success {
            return Err(GitError::new(
                GitErrorKind::ReferenceChanged,
                "branch or upstream changed while reading tracking state",
            )
            .with_operation(context.operation_id)
            .with_diagnostic(format!("exit_code={:?}", counts.exit_code)));
        }
        let (ahead, behind) = parse_counts(&counts.stdout, context.operation_id)?;
        Ok(Some(BranchTracking {
            upstream,
            ahead,
            behind,
        }))
    }
}

impl<R: ProcessRunner> GitCliHeadReader<R> {
    async fn run<const N: usize>(
        &self,
        location: &RepositoryLocation,
        args: [&str; N],
        context: ExecutionContext,
    ) -> GitResult<ProcessOutput> {
        self.run_os(location, args.map(OsString::from), context)
            .await
    }

    async fn run_os(
        &self,
        location: &RepositoryLocation,
        args: impl IntoIterator<Item = OsString>,
        context: ExecutionContext,
    ) -> GitResult<ProcessOutput> {
        let mut environment = vec![
            EnvironmentVariable::public("GIT_OPTIONAL_LOCKS", "0"),
            EnvironmentVariable::public("GIT_TERMINAL_PROMPT", "0"),
        ];
        let cwd = if let Some(worktree) = location.worktree_root() {
            worktree.to_path_buf()
        } else {
            environment.push(EnvironmentVariable::public(
                "GIT_DIR",
                location.git_dir().as_os_str(),
            ));
            location.git_dir().to_path_buf()
        };
        environment.extend(self.environment.iter().cloned());
        self.executor
            .execute(
                HeadCommand {
                    cwd,
                    environment,
                    args: args.into_iter().collect(),
                },
                context,
            )
            .await
    }
}

struct HeadCommand {
    cwd: PathBuf,
    environment: Vec<EnvironmentVariable>,
    args: Vec<OsString>,
}

impl GitCliCommand for HeadCommand {}

impl sealed::Sealed for HeadCommand {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = vec![
            OsString::from("-c"),
            OsString::from("core.quotepath=false"),
            OsString::from("--no-optional-locks"),
        ];
        args.extend(self.args);
        GitCliInvocation::new(None, args)
            .with_cwd(self.cwd)
            .with_environment(self.environment)
    }
}

fn parse_ref_line(
    raw: Vec<u8>,
    operation_id: OperationId,
    field: &'static str,
) -> GitResult<RefName> {
    let raw = strip_one_line_ending(raw);
    if raw.contains(&b'\n') || raw.contains(&b'\r') {
        return Err(invalid_output(
            operation_id,
            "ref output contains multiple records",
        ));
    }
    RefName::new(raw)
        .map_err(|error| invalid_output(operation_id, &format!("invalid {field}: {error}")))
}

fn parse_oid_line(
    raw: Vec<u8>,
    operation_id: OperationId,
    field: &'static str,
) -> GitResult<ObjectId> {
    let raw = strip_one_line_ending(raw);
    let text = str::from_utf8(&raw)
        .map_err(|error| invalid_output(operation_id, &format!("invalid {field}: {error}")))?;
    text.parse().map_err(|error: git_domain::ObjectIdError| {
        invalid_output(operation_id, &format!("invalid {field}: {error}"))
    })
}

fn parse_counts(raw: &[u8], operation_id: OperationId) -> GitResult<(u64, u64)> {
    let without_newline = raw.strip_suffix(b"\n").unwrap_or(raw);
    let line = without_newline
        .strip_suffix(b"\r")
        .unwrap_or(without_newline);
    let mut fields = line
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty());
    let ahead = parse_count(fields.next(), operation_id, "ahead")?;
    let behind = parse_count(fields.next(), operation_id, "behind")?;
    if fields.next().is_some() {
        return Err(invalid_output(
            operation_id,
            "ahead/behind output has extra fields",
        ));
    }
    Ok((ahead, behind))
}

fn parse_count(
    raw: Option<&[u8]>,
    operation_id: OperationId,
    field: &'static str,
) -> GitResult<u64> {
    let raw = raw.ok_or_else(|| invalid_output(operation_id, "missing ahead/behind count"))?;
    let text = str::from_utf8(raw)
        .map_err(|error| invalid_output(operation_id, &format!("invalid {field}: {error}")))?;
    text.parse().map_err(|error: std::num::ParseIntError| {
        invalid_output(operation_id, &format!("invalid {field}: {error}"))
    })
}

fn strip_one_line_ending(mut raw: Vec<u8>) -> Vec<u8> {
    if raw.last() == Some(&b'\n') {
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
    }
    raw
}

fn command_failure(
    message: &'static str,
    operation_id: OperationId,
    output: &ProcessOutput,
) -> GitError {
    GitError::new(GitErrorKind::InvalidRepository, message)
        .with_operation(operation_id)
        .with_diagnostic(format!("exit_code={:?}", output.exit_code))
}

fn invalid_output(operation_id: OperationId, reason: &str) -> GitError {
    GitError::new(
        GitErrorKind::Internal,
        "Git returned invalid HEAD or branch data",
    )
    .with_operation(operation_id)
    .with_diagnostic(reason)
}

#[cfg(unix)]
fn os_string_from_bytes(
    raw: Vec<u8>,
    _operation_id: OperationId,
    _field: &'static str,
) -> GitResult<OsString> {
    use std::os::unix::ffi::OsStringExt;

    Ok(OsString::from_vec(raw))
}

#[cfg(windows)]
fn os_string_from_bytes(
    raw: Vec<u8>,
    operation_id: OperationId,
    field: &'static str,
) -> GitResult<OsString> {
    String::from_utf8(raw)
        .map(OsString::from)
        .map_err(|error| invalid_output(operation_id, &format!("invalid {field}: {error}")))
}

#[cfg(test)]
mod tests {
    use git_domain::OperationId;
    use git_fixture::TestRepository;
    use git_ports::{
        BranchReader, EnvironmentVariable, HeadReader, ProcessProgress, RepositoryReader,
    };
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{GitCliRepositoryReader, TokioProcessRunner};

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(32);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn isolated_environment(fixture: &TestRepository) -> Vec<EnvironmentVariable> {
        fixture
            .isolated_environment()
            .into_iter()
            .map(|(name, value)| EnvironmentVariable::public(name, value))
            .collect()
    }

    fn reader(fixture: &TestRepository) -> GitCliHeadReader<TokioProcessRunner> {
        GitCliHeadReader::new(TokioProcessRunner).with_environment(isolated_environment(fixture))
    }

    async fn location(fixture: &TestRepository) -> RepositoryLocation {
        GitCliRepositoryReader::new(TokioProcessRunner)
            .with_environment(isolated_environment(fixture))
            .discover(fixture.worktree.clone(), context())
            .await
            .expect("fixture repository should be discovered")
    }

    #[tokio::test]
    async fn reads_unborn_attached_and_detached_head_states() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        let location = location(&fixture).await;
        let reader = reader(&fixture);

        let unborn = reader
            .read_head(&location, context())
            .await
            .expect("unborn HEAD should be readable");
        assert!(matches!(
            unborn,
            HeadState::Unborn { branch } if branch.as_bytes() == b"main"
        ));

        fixture
            .commit_file("README.md", b"attached\n")
            .expect("fixture commit should succeed");
        let expected: ObjectId = fixture
            .head_oid()
            .expect("HEAD query should succeed")
            .expect("HEAD should exist")
            .parse()
            .expect("fixture HEAD should be a full OID");
        let attached = reader
            .read_head(&location, context())
            .await
            .expect("attached HEAD should be readable");
        assert!(matches!(
            attached,
            HeadState::Attached { branch, oid }
                if branch.as_bytes() == b"main" && oid == expected
        ));

        fixture.detach_head().expect("HEAD should detach");
        let detached = reader
            .read_head(&location, context())
            .await
            .expect("detached HEAD should be readable");
        assert_eq!(detached, HeadState::Detached { oid: expected });
    }

    #[tokio::test]
    async fn reads_upstream_and_diverged_ahead_behind_counts() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("README.md", b"base\n")
            .expect("base commit should succeed");
        let remote = fixture
            .create_local_remote()
            .expect("local remote should be created");
        let location = location(&fixture).await;
        let reader = reader(&fixture);

        let initial = reader
            .read_tracking(&location, context())
            .await
            .expect("tracking state should be readable")
            .expect("pushed branch should have upstream");
        assert_eq!(initial.upstream.as_bytes(), b"refs/remotes/origin/main");
        assert_eq!((initial.ahead, initial.behind), (0, 0));

        fixture
            .commit_file("local.txt", b"local\n")
            .expect("local commit should succeed");
        fixture
            .push_change_from_peer(&remote, "remote.txt", b"remote\n")
            .expect("peer push should succeed");
        fixture
            .fetch_remote()
            .expect("remote-tracking ref should update");

        let diverged = reader
            .read_tracking(&location, context())
            .await
            .expect("diverged tracking state should be readable")
            .expect("branch should retain upstream");
        assert_eq!((diverged.ahead, diverged.behind), (1, 1));
    }

    #[tokio::test]
    async fn returns_no_tracking_for_unconfigured_or_detached_branch() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("README.md", b"base\n")
            .expect("base commit should succeed");
        let location = location(&fixture).await;
        let reader = reader(&fixture);

        assert_eq!(
            reader
                .read_tracking(&location, context())
                .await
                .expect("missing upstream is not an error"),
            None
        );
        fixture.detach_head().expect("HEAD should detach");
        assert_eq!(
            reader
                .read_tracking(&location, context())
                .await
                .expect("detached HEAD has no tracking branch"),
            None
        );
    }

    #[tokio::test]
    async fn reads_symbolic_head_from_bare_repository() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("README.md", b"base\n")
            .expect("base commit should succeed");
        let remote = fixture
            .create_local_remote()
            .expect("bare remote should be created");
        let bare = GitCliRepositoryReader::new(TokioProcessRunner)
            .with_environment(isolated_environment(&fixture))
            .discover(remote.path().to_path_buf(), context())
            .await
            .expect("bare repository should be discovered");

        let head = reader(&fixture)
            .read_head(&bare, context())
            .await
            .expect("bare HEAD should be readable");

        assert!(matches!(
            head,
            HeadState::Attached { branch, .. } if branch.as_bytes() == b"main"
        ));
    }

    #[test]
    fn rejects_malformed_ahead_behind_output() {
        let operation = OperationId::new();
        assert_eq!(
            parse_counts(b"3\t2\n", operation).expect("counts should parse"),
            (3, 2)
        );
        assert!(parse_counts(b"3 only-one-extra field\n", operation).is_err());
        assert!(parse_counts(b"not-a-number 2\n", operation).is_err());
    }
}
