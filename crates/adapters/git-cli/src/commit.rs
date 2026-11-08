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

/// Authoritative HEAD and Index tree identity observed immediately before a standard commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitCliCommitState {
    pub head: Option<ObjectId>,
    pub index: ObjectId,
}

/// Typed Git CLI probe used by the composition root to implement commit preflight.
#[derive(Debug)]
pub struct GitCliCommitStateProbe<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliCommitStateProbe<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

impl<R: ProcessRunner> GitCliCommitStateProbe<R> {
    /// Reads `HEAD` and the tree represented by the real Index without changing Index or refs.
    #[allow(clippy::result_large_err)]
    pub async fn read(&self, context: ExecutionContext) -> GitResult<GitCliCommitState> {
        let cwd = self
            .location
            .worktree_root()
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "commit preflight is unavailable for a bare repository",
                )
            })?
            .to_path_buf();
        let head = self
            .executor
            .execute(HeadOidCommand { cwd: cwd.clone() }, context.clone())
            .await?;
        let head = if head.success {
            Some(parse_oid(&head.stdout, "Git returned an invalid HEAD OID")?)
        } else if head.exit_code == Some(1) {
            None
        } else {
            return Err(
                GitError::new(GitErrorKind::InvalidRepository, "Git could not read HEAD")
                    .with_diagnostic(format!("exit_code={:?}", head.exit_code)),
            );
        };
        let index = self
            .executor
            .execute(IndexTreeCommand { cwd }, context)
            .await?;
        if !index.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git could not read the Index tree",
            )
            .with_diagnostic(format!("exit_code={:?}", index.exit_code)));
        }
        Ok(GitCliCommitState {
            head,
            index: parse_oid(&index.stdout, "Git returned an invalid Index tree OID")?,
        })
    }
}

#[allow(clippy::result_large_err)]
fn parse_oid(bytes: &[u8], message: &'static str) -> GitResult<ObjectId> {
    ObjectId::from_str(String::from_utf8_lossy(bytes).trim()).map_err(|error| {
        GitError::new(GitErrorKind::Internal, message).with_diagnostic(error.to_string())
    })
}

#[derive(Debug)]
pub struct GitCliCommitWriter<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
    environment: Vec<EnvironmentVariable>,
}

/// Commit writer explicitly bound to a temporary Index. It never changes the repository's
/// real `.git/index`; Git hooks still run under Git's normal semantics and are therefore observed
/// by the caller as possible external side effects.
#[derive(Debug)]
pub struct GitCliTemporaryIndexCommitWriter<R> {
    inner: GitCliCommitWriter<R>,
}

impl<R> GitCliTemporaryIndexCommitWriter<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation, index: impl Into<OsString>) -> Self {
        Self {
            inner: GitCliCommitWriter::new(runner, location)
                .with_environment([(OsString::from("GIT_INDEX_FILE"), index.into())]),
        }
    }

    #[must_use]
    pub fn with_environment(
        mut self,
        variables: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Self {
        self.inner.environment.extend(
            variables
                .into_iter()
                .map(|(name, value)| EnvironmentVariable::public(name, value)),
        );
        self
    }
}

#[async_trait]
impl<R: ProcessRunner> CommitWriter for GitCliTemporaryIndexCommitWriter<R> {
    async fn commit(
        &self,
        target: SessionTarget,
        request: StandardCommitRequest,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        self.inner.commit(target, request, context).await
    }
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

struct HeadOidCommand {
    cwd: PathBuf,
}
impl GitCliCommand for HeadOidCommand {}
impl sealed::Sealed for HeadOidCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("--quiet"),
                OsString::from("HEAD"),
            ],
        )
        .with_cwd(self.cwd)
    }
}

struct IndexTreeCommand {
    cwd: PathBuf,
}
impl GitCliCommand for IndexTreeCommand {}
impl sealed::Sealed for IndexTreeCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(None, [OsString::from("write-tree")]).with_cwd(self.cwd)
    }
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
    use std::{fs, process::Command};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::TokioProcessRunner;

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

    #[tokio::test]
    async fn commit_state_probe_tracks_unborn_head_and_real_index_changes() {
        let fixture = TestRepository::init().expect("fixture");
        let probe = GitCliCommitStateProbe::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        );
        let (progress, _receiver) = mpsc::channel(4);
        let unborn = probe
            .read(ExecutionContext::new(
                OperationId::new(),
                CancellationToken::new(),
                progress,
            ))
            .await
            .expect("unborn state should be readable");
        assert_eq!(unborn.head, None);

        fixture
            .commit_file("one.txt", b"one\n")
            .expect("first commit should succeed");
        let (progress, _receiver) = mpsc::channel(4);
        let committed = probe
            .read(ExecutionContext::new(
                OperationId::new(),
                CancellationToken::new(),
                progress,
            ))
            .await
            .expect("committed state should be readable");
        assert!(committed.head.is_some());
        assert_ne!(committed.index, unborn.index);

        fixture
            .write_file("two.txt", b"two\n")
            .expect("second file should be written");
        fixture
            .stage_file("two.txt")
            .expect("second file should be staged");
        let (progress, _receiver) = mpsc::channel(4);
        let changed = probe
            .read(ExecutionContext::new(
                OperationId::new(),
                CancellationToken::new(),
                progress,
            ))
            .await
            .expect("changed Index state should be readable");
        assert_eq!(changed.head, committed.head);
        assert_ne!(changed.index, committed.index);
    }

    #[tokio::test]
    async fn temporary_index_commit_preserves_real_index_bytes() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("file.txt", b"one\n")
            .expect("initial commit");
        let real_index = fixture.worktree.join(".git/index");
        let before = fs::read(&real_index).expect("real index");
        fixture.write_file("file.txt", b"two\n").expect("modify");
        let temporary_index = fixture.temp_dir.path().join("candidate-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        read_tree.env("GIT_INDEX_FILE", &temporary_index);
        assert!(read_tree.status().expect("git starts").success());
        let mut add = Command::new("git");
        add.args(["add", "--", "file.txt"])
            .current_dir(&fixture.worktree);
        for (name, value) in fixture.isolated_environment() {
            add.env(name, value);
        }
        add.env("GIT_INDEX_FILE", &temporary_index);
        assert!(add.status().expect("git starts").success());
        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let writer =
            GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location, &temporary_index)
                .with_environment(fixture.isolated_environment());
        let (progress, _receiver) = mpsc::channel(4);
        let outcome = writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"temporary commit\n".to_vec(),
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
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("temporary commit should succeed");
        assert!(!outcome.commit.to_string().is_empty());
        assert_eq!(fs::read(&real_index).expect("real index"), before);
    }

    #[tokio::test]
    async fn temporary_partial_commit_keeps_unselected_worktree_and_index_content() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("selected.txt", b"selected-base\n")
            .expect("first file");
        fixture
            .write_file("unselected.txt", b"unselected-base\n")
            .expect("second file");
        fixture
            .stage_file("unselected.txt")
            .expect("stage second file");
        let commit_writer = GitCliCommitWriter::new(
            TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        )
        .with_environment(fixture.isolated_environment());
        let (progress, _receiver) = mpsc::channel(4);
        commit_writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"add second file\n".to_vec(),
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
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("baseline commit");

        fixture
            .write_file("selected.txt", b"selected-changed\n")
            .expect("selected edit");
        fixture
            .write_file("unselected.txt", b"unselected-changed\n")
            .expect("unselected edit");
        let real_index = fixture.worktree.join(".git/index");
        let index_before = fs::read(&real_index).expect("real Index");
        let temporary_index = fixture.temp_dir.path().join("partial-candidate-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        assert!(read_tree.status().expect("git starts").success());
        let mut add_selected = Command::new("git");
        add_selected
            .args(["add", "--", "selected.txt"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            add_selected.env(name, value);
        }
        assert!(add_selected.status().expect("git starts").success());

        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let writer =
            GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location, &temporary_index)
                .with_environment(fixture.isolated_environment());
        let (progress, _receiver) = mpsc::channel(4);
        writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"partial selected commit\n".to_vec(),
                        amend: false,
                        allow_empty: false,
                        signoff: false,
                        author: None,
                        committer: None,
                        signing: SigningMode::Disable,
                    },
                    expected_revision: SnapshotRevision::new(2),
                    expected_head: None,
                    expected_index: None,
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("partial commit");

        assert_eq!(fs::read(&real_index).expect("real Index"), index_before);
        assert_eq!(
            fs::read(fixture.worktree.join("unselected.txt")).expect("worktree"),
            b"unselected-changed\n"
        );
        let selected_head = Command::new("git")
            .args(["show", "HEAD:selected.txt"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("git show");
        assert_eq!(selected_head.stdout, b"selected-changed\n");
        let unselected_head = Command::new("git")
            .args(["show", "HEAD:unselected.txt"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("git show");
        assert_eq!(unselected_head.stdout, b"unselected-base\n");
    }
}
