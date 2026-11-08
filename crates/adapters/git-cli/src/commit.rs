//! Typed Git CLI implementation of standard Index commits.

use std::{ffi::OsString, path::PathBuf, str::FromStr};

use async_trait::async_trait;
use git_domain::{
    GitError, GitErrorKind, GitResult, ObjectId, OperationId, SessionTarget, SigningMode,
    StandardCommitRequest,
};
use git_ports::{
    CommitOutcome, CommitSideEffectObserver, CommitSideEffectSnapshot, CommitWriter,
    EnvironmentVariable, ExecutionContext, PartialCommitWriter, ProcessRunner, ProcessStdin,
    RepositoryLocation, TemporaryIndexLease,
};
use sha2::{Digest, Sha256};

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

/// Commit writer bound to the Preview's temporary Index for each invocation. It never selects the
/// repository's real `.git/index`; Git hooks still run under Git's normal semantics and are
/// therefore observed by the caller as possible external side effects.
#[derive(Debug)]
pub struct GitCliTemporaryIndexCommitWriter<R> {
    inner: GitCliCommitWriter<R>,
}

/// Observes commit side effects without inheriting a temporary `GIT_INDEX_FILE`.
#[derive(Debug)]
pub struct GitCliCommitSideEffectObserver<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliCommitSideEffectObserver<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> CommitSideEffectObserver for GitCliCommitSideEffectObserver<R> {
    async fn capture(
        &self,
        _target: SessionTarget,
        context: ExecutionContext,
    ) -> GitResult<CommitSideEffectSnapshot> {
        self.snapshot(context).await
    }

    async fn observe(
        &self,
        _target: SessionTarget,
        context: ExecutionContext,
    ) -> GitResult<CommitSideEffectSnapshot> {
        self.snapshot(context).await
    }
}

impl<R: ProcessRunner> GitCliCommitSideEffectObserver<R> {
    #[allow(clippy::result_large_err)]
    async fn snapshot(&self, context: ExecutionContext) -> GitResult<CommitSideEffectSnapshot> {
        let cwd = self
            .location
            .worktree_root()
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "commit side-effect observation is unavailable for a bare repository",
                )
            })?
            .to_path_buf();
        let head_output = self
            .executor
            .execute(HeadOidCommand { cwd: cwd.clone() }, context.clone())
            .await?;
        let head = if head_output.success {
            Some(parse_oid(
                &head_output.stdout,
                "Git returned an invalid HEAD OID",
            )?)
        } else if head_output.exit_code == Some(1) {
            None
        } else {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git could not read HEAD",
            ));
        };
        let worktree = self
            .executor
            .execute(
                WorktreeObservationCommand { cwd: cwd.clone() },
                context.clone(),
            )
            .await?;
        if !worktree.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git could not observe Worktree",
            ));
        }
        let untracked = self
            .executor
            .execute(
                UntrackedObservationCommand { cwd: cwd.clone() },
                context.clone(),
            )
            .await?;
        if !untracked.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git could not observe untracked Worktree paths",
            ));
        }
        let refs = self
            .executor
            .execute(RefsObservationCommand { cwd }, context)
            .await?;
        if !refs.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git could not observe refs",
            ));
        }
        let mut worktree_bytes = worktree.stdout;
        worktree_bytes.extend_from_slice(&untracked.stdout);
        Ok(CommitSideEffectSnapshot {
            head,
            worktree_digest: Sha256::digest(&worktree_bytes).into(),
            refs_digest: Sha256::digest(&refs.stdout).into(),
        })
    }
}

impl<R> GitCliTemporaryIndexCommitWriter<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            inner: GitCliCommitWriter::new(runner, location),
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
impl<R: ProcessRunner> PartialCommitWriter for GitCliTemporaryIndexCommitWriter<R> {
    async fn commit(
        &self,
        target: SessionTarget,
        operation: OperationId,
        request: StandardCommitRequest,
        temporary_index: &TemporaryIndexLease,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        if temporary_index.transaction_id != operation {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "temporary Index belongs to another operation",
            )
            .with_target(target)
            .with_operation(context.operation_id));
        }
        if temporary_index.path == self.inner.location.git_dir().join("index") {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "partial commit cannot use the real repository Index",
            )
            .with_target(target)
            .with_operation(context.operation_id));
        }
        if !temporary_index.path.is_file() {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "temporary Index is no longer available",
            )
            .with_target(target)
            .with_operation(context.operation_id));
        }
        self.inner
            .commit_with_environment(
                target,
                request,
                [EnvironmentVariable::public(
                    "GIT_INDEX_FILE",
                    temporary_index.path.clone().into_os_string(),
                )],
                context,
            )
            .await
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
        self.commit_with_environment(_target, request, [], context)
            .await
    }
}

impl<R: ProcessRunner> GitCliCommitWriter<R> {
    #[allow(clippy::result_large_err)]
    async fn commit_with_environment(
        &self,
        _target: SessionTarget,
        request: StandardCommitRequest,
        environment: impl IntoIterator<Item = EnvironmentVariable>,
        context: ExecutionContext,
    ) -> GitResult<CommitOutcome> {
        let signing = request.options.signing;
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
        let mut command_environment = self.environment.clone();
        command_environment.extend(environment);
        let output = self
            .executor
            .execute(
                CommitCommand {
                    cwd: cwd.clone(),
                    request,
                    environment: command_environment,
                },
                context.clone(),
            )
            .await?;
        if !output.success {
            let kind = if signing == SigningMode::Require {
                GitErrorKind::SigningFailed
            } else {
                GitErrorKind::HookRejected
            };
            return Err(
                GitError::new(kind, "Git commit was rejected").with_diagnostic(format!(
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

struct WorktreeObservationCommand {
    cwd: PathBuf,
}
impl GitCliCommand for WorktreeObservationCommand {}
impl sealed::Sealed for WorktreeObservationCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("-c"),
                OsString::from("core.quotepath=false"),
                OsString::from("diff"),
                OsString::from("--binary"),
                OsString::from("--no-ext-diff"),
            ],
        )
        .with_cwd(self.cwd)
    }
}

struct UntrackedObservationCommand {
    cwd: PathBuf,
}
impl GitCliCommand for UntrackedObservationCommand {}
impl sealed::Sealed for UntrackedObservationCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("ls-files"),
                OsString::from("--others"),
                OsString::from("--exclude-standard"),
                OsString::from("-z"),
            ],
        )
        .with_cwd(self.cwd)
    }
}

struct RefsObservationCommand {
    cwd: PathBuf,
}
impl GitCliCommand for RefsObservationCommand {}
impl sealed::Sealed for RefsObservationCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("for-each-ref"),
                OsString::from("--format=%(refname)%00%(objectname)%00%(symref)"),
                OsString::from("refs/tags"),
                OsString::from("refs/remotes"),
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
    use async_trait::async_trait;
    use git_commit::{
        PartialCommitCoordinator, PartialCommitJournal, PartialCommitPhase,
        PartialCommitTransaction,
    };
    use git_domain::{
        ChangeSelection, CommitOptions, FileIdentity, FileKind, JournalRecord, JournalResult,
        OperationId, RecoveryPath, RecoveryScope, RepoPath, RepositoryId, SelectionCommitRequest,
        SelectionFingerprint, SessionTarget, SnapshotRevision, WorktreeId,
    };
    use git_fixture::TestRepository;
    use git_ports::{
        CandidateTreeBuilder, CandidateTreeRequest, ExecutionContext, IndexReconciliationOutcome,
        IndexReconciliationPublisher, IndexReconciliationRequest, JournalStore,
        PendingJournalRecords, TemporaryIndexLease,
    };
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;
    use std::sync::{Arc, Mutex};
    use std::{fs, process::Command};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        GitCliCandidateTreeBuilder, GitCliExecutor, GitCliIndexReconciliationPublisher,
        TokioProcessRunner,
    };

    #[derive(Clone, Default)]
    struct MemoryJournal {
        record: Arc<Mutex<Option<JournalRecord>>>,
    }

    struct ExternalIndexMutationPublisher {
        fixture: std::path::PathBuf,
        inner: GitCliIndexReconciliationPublisher,
    }

    #[async_trait]
    impl IndexReconciliationPublisher for ExternalIndexMutationPublisher {
        async fn publish(
            &self,
            request: IndexReconciliationRequest,
            context: ExecutionContext,
        ) -> GitResult<IndexReconciliationOutcome> {
            let index = self.fixture.join("index");
            let mut external = fs::read(&index).expect("real Index should exist");
            external.extend_from_slice(b"external-index-write");
            fs::write(&index, external).expect("external Index mutation should succeed");
            self.inner.publish(request, context).await
        }
    }

    #[async_trait]
    impl JournalStore for MemoryJournal {
        async fn append(&self, record: JournalRecord) -> GitResult<()> {
            *self.record.lock().expect("journal lock") = Some(record);
            Ok(())
        }
        async fn replace(&self, record: JournalRecord) -> GitResult<()> {
            *self.record.lock().expect("journal lock") = Some(record);
            Ok(())
        }
        async fn get(&self, _operation_id: OperationId) -> GitResult<Option<JournalRecord>> {
            Ok(self.record.lock().expect("journal lock").clone())
        }
        async fn pending(&self, _scope: &RecoveryScope) -> GitResult<PendingJournalRecords> {
            Ok(PendingJournalRecords::default())
        }
    }

    fn test_context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel(4);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn partial_context(operation: OperationId) -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel(4);
        ExecutionContext::new(operation, CancellationToken::new(), progress)
    }

    fn temporary_lease(path: &std::path::Path, operation: OperationId) -> TemporaryIndexLease {
        TemporaryIndexLease {
            path: path.to_path_buf(),
            transaction_id: operation,
        }
    }

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
        let writer = GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location)
            .with_environment(fixture.isolated_environment());
        let operation = OperationId::new();
        let lease = temporary_lease(&temporary_index, operation);
        let outcome = writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                operation,
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
                &lease,
                partial_context(operation),
            )
            .await
            .expect("temporary commit should succeed");
        assert!(!outcome.commit.to_string().is_empty());
        assert_eq!(fs::read(&real_index).expect("real index"), before);
    }

    #[tokio::test]
    async fn temporary_index_commit_rejects_a_lease_from_another_operation() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("file.txt", b"one\n")
            .expect("initial commit");
        let real_index = fixture.worktree.join(".git/index");
        let before = fs::read(&real_index).expect("real index");
        let head_before = fixture.head_oid().expect("HEAD");
        let temporary_index = fixture.temp_dir.path().join("candidate-index");
        fs::copy(&real_index, &temporary_index).expect("temporary Index");
        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let writer = GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location)
            .with_environment(fixture.isolated_environment());
        let lease_operation = OperationId::new();
        let context_operation = OperationId::new();
        let lease = temporary_lease(&temporary_index, lease_operation);

        let error = writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                context_operation,
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"must not commit\n".to_vec(),
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
                &lease,
                partial_context(context_operation),
            )
            .await
            .expect_err("another operation's temporary Index must be rejected");

        assert_eq!(error.kind, GitErrorKind::StaleSelection);
        assert_eq!(error.operation_id, Some(context_operation));
        assert_eq!(fs::read(&real_index).expect("real index"), before);
        assert_eq!(fixture.head_oid().expect("HEAD"), head_before);
    }

    #[tokio::test]
    async fn temporary_partial_commit_keeps_unselected_worktree_and_index_content() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("selected.txt", b"selected-base\r\n")
            .expect("first file");
        fixture
            .write_file("unselected.txt", b"unselected-base\r\n")
            .expect("second file");
        fixture
            .write_file("binary.bin", b"\0binary-base\xff")
            .expect("binary file");
        fixture
            .stage_file("unselected.txt")
            .expect("stage second file");
        fixture.stage_file("binary.bin").expect("stage binary file");
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
            .write_file("selected.txt", b"selected-changed\r\n")
            .expect("selected edit");
        fixture
            .write_file("unselected.txt", b"unselected-changed\r\n")
            .expect("unselected edit");
        fixture
            .write_file("binary.bin", b"\0binary-changed\xfe")
            .expect("binary edit");
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
        let writer = GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location)
            .with_environment(fixture.isolated_environment());
        let operation = OperationId::new();
        let lease = temporary_lease(&temporary_index, operation);
        writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                operation,
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
                &lease,
                partial_context(operation),
            )
            .await
            .expect("partial commit");

        assert_eq!(fs::read(&real_index).expect("real Index"), index_before);
        assert_eq!(
            fs::read(fixture.worktree.join("unselected.txt")).expect("worktree"),
            b"unselected-changed\r\n"
        );
        assert_eq!(
            fs::read(fixture.worktree.join("binary.bin")).expect("binary worktree"),
            b"\0binary-changed\xfe"
        );
        let selected_head = Command::new("git")
            .args(["show", "HEAD:selected.txt"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("git show");
        assert_eq!(selected_head.stdout, b"selected-changed\r\n");
        let unselected_head = Command::new("git")
            .args(["show", "HEAD:unselected.txt"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("git show");
        assert_eq!(unselected_head.stdout, b"unselected-base\r\n");
        let binary_head = Command::new("git")
            .args(["show", "HEAD:binary.bin"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("git show binary path");
        assert_eq!(binary_head.stdout, b"\0binary-base\xff");
    }

    #[tokio::test]
    async fn temporary_partial_commit_does_not_mix_staged_and_unstaged_edits() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("mixed.txt", b"base\n")
            .expect("initial commit");
        fixture
            .write_file("mixed.txt", b"staged\n")
            .expect("staged edit");
        fixture.stage_file("mixed.txt").expect("stage edit");
        fixture
            .write_file("mixed.txt", b"unstaged\n")
            .expect("unstaged edit");

        let real_index = fixture.worktree.join(".git/index");
        let index_before = fs::read(&real_index).expect("real Index");
        let temporary_index = fixture.temp_dir.path().join("mixed-candidate-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        assert!(read_tree.status().expect("git starts").success());
        let mut add = Command::new("git");
        add.args(["add", "--", "mixed.txt"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            add.env(name, value);
        }
        assert!(add.status().expect("git starts").success());

        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let writer = GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location)
            .with_environment(fixture.isolated_environment());
        let operation = OperationId::new();
        let lease = temporary_lease(&temporary_index, operation);
        writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                operation,
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"mixed staged and unstaged\n".to_vec(),
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
                &lease,
                partial_context(operation),
            )
            .await
            .expect("temporary partial commit");

        assert_eq!(fs::read(&real_index).expect("real Index"), index_before);
        assert_eq!(
            fs::read(fixture.worktree.join("mixed.txt")).expect("worktree"),
            b"unstaged\n"
        );
        let head = Command::new("git")
            .args(["show", "HEAD:mixed.txt"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("HEAD blob");
        assert_eq!(head.stdout, b"unstaged\n");
        let index = Command::new("git")
            .args(["show", ":mixed.txt"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("Index blob");
        assert_eq!(index.stdout, b"staged\n");
    }

    #[tokio::test]
    async fn temporary_partial_commit_preserves_unselected_content_during_rename() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("old-name.txt", b"old-base\n")
            .expect("first file");
        fixture
            .write_file("other.txt", b"other-base\n")
            .expect("second file");
        fixture.stage_file("other.txt").expect("stage second file");
        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let baseline_writer = GitCliCommitWriter::new(TokioProcessRunner, location.clone())
            .with_environment(fixture.isolated_environment());
        let (progress, _receiver) = mpsc::channel(4);
        baseline_writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"add other file\n".to_vec(),
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
            .rename_file("old-name.txt", "new-name.txt")
            .expect("rename selected file");
        fixture
            .write_file("new-name.txt", b"new-content\n")
            .expect("edit renamed file");
        fixture
            .write_file("other.txt", b"other-unselected\n")
            .expect("edit unselected file");
        let real_index = fixture.worktree.join(".git/index");
        let index_before = fs::read(&real_index).expect("real Index");
        let temporary_index = fixture.temp_dir.path().join("rename-candidate-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        assert!(read_tree.status().expect("git starts").success());
        let mut add_rename = Command::new("git");
        add_rename
            .args(["add", "-A", "--", "old-name.txt", "new-name.txt"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            add_rename.env(name, value);
        }
        assert!(add_rename.status().expect("git starts").success());
        let writer = GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location)
            .with_environment(fixture.isolated_environment());
        let operation = OperationId::new();
        let lease = temporary_lease(&temporary_index, operation);
        writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                operation,
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"partial rename\n".to_vec(),
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
                &lease,
                partial_context(operation),
            )
            .await
            .expect("partial rename commit");

        assert_eq!(fs::read(&real_index).expect("real Index"), index_before);
        let old_head = Command::new("git")
            .args(["show", "HEAD:old-name.txt"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("git show old path");
        assert!(!old_head.status.success());
        let new_head = Command::new("git")
            .args(["show", "HEAD:new-name.txt"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("git show new path");
        assert_eq!(new_head.stdout, b"new-content\n");
        let other_head = Command::new("git")
            .args(["show", "HEAD:other.txt"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("git show unselected path");
        assert_eq!(other_head.stdout, b"other-base\n");
        assert_eq!(
            fs::read(fixture.worktree.join("other.txt")).expect("worktree"),
            b"other-unselected\n"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn temporary_partial_commit_preserves_non_utf8_path_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let fixture = TestRepository::init().expect("fixture");
        let name = OsString::from_vec(b"non-utf8-\xff.txt".to_vec());
        let path = fixture.worktree.join(&name);
        fs::write(&path, b"base\n").expect("non-UTF-8 path");
        let mut add = Command::new("git");
        add.arg("add")
            .arg("--")
            .arg(&name)
            .current_dir(&fixture.worktree);
        for (key, value) in fixture.isolated_environment() {
            add.env(key, value);
        }
        assert!(add.status().expect("git starts").success());
        let mut commit = Command::new("git");
        commit
            .args(["commit", "--no-gpg-sign", "-m", "initial non-utf8"])
            .current_dir(&fixture.worktree);
        for (key, value) in fixture.isolated_environment() {
            commit.env(key, value);
        }
        assert!(commit.status().expect("git starts").success());
        fs::write(&path, b"changed\n").expect("edit non-UTF-8 path");
        let real_index = fixture.worktree.join(".git/index");
        let index_before = fs::read(&real_index).expect("real Index");
        let temporary_index = fixture.temp_dir.path().join("non-utf8-candidate-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (key, value) in fixture.isolated_environment() {
            read_tree.env(key, value);
        }
        assert!(read_tree.status().expect("git starts").success());
        let mut add_changed = Command::new("git");
        add_changed
            .arg("add")
            .arg("--")
            .arg(&name)
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (key, value) in fixture.isolated_environment() {
            add_changed.env(key, value);
        }
        assert!(add_changed.status().expect("git starts").success());
        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let writer = GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location)
            .with_environment(fixture.isolated_environment());
        let operation = OperationId::new();
        let lease = temporary_lease(&temporary_index, operation);
        writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"partial non-utf8 path\n".to_vec(),
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
                &lease,
                partial_context(operation),
            )
            .await
            .expect("non-UTF-8 path commit");
        assert_eq!(fs::read(&real_index).expect("real Index"), index_before);
        let mut show = Command::new("git");
        let mut object_path = OsString::from("HEAD:");
        object_path.push(&name);
        show.args(["show"])
            .arg(&object_path)
            .current_dir(&fixture.worktree);
        for (key, value) in fixture.isolated_environment() {
            show.env(key, value);
        }
        let output = show.output().expect("git show");
        assert_eq!(output.stdout, b"changed\n");
    }

    #[tokio::test]
    async fn candidate_tree_partial_line_patch_commits_only_selected_line() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("lines.txt", b"one\ntwo\nthree\n")
            .expect("initial commit");
        fixture
            .write_file("lines.txt", b"one\ntwo-selected\nthree\n")
            .expect("line edit");
        let temporary_index = fixture.temp_dir.path().join("line-candidate-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (key, value) in fixture.isolated_environment() {
            read_tree.env(key, value);
        }
        assert!(read_tree.status().expect("git starts").success());
        let patch = b"diff --git a/lines.txt b/lines.txt\n--- a/lines.txt\n+++ b/lines.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+two-selected\n three\n";
        let (progress, _receiver) = mpsc::channel(4);
        let builder = GitCliCandidateTreeBuilder::new(GitCliExecutor::new(TokioProcessRunner));
        let tree = builder
            .build(
                CandidateTreeRequest {
                    temporary_index: temporary_index.clone(),
                    worktree_root: fixture.worktree.clone(),
                    patch: patch.to_vec(),
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("candidate tree");
        assert!(!tree.to_string().is_empty());
        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let writer = GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location)
            .with_environment(fixture.isolated_environment());
        let operation = OperationId::new();
        let lease = temporary_lease(&temporary_index, operation);
        writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                operation,
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"partial line\n".to_vec(),
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
                &lease,
                partial_context(operation),
            )
            .await
            .expect("partial line commit");
        let output = Command::new("git")
            .args(["show", "HEAD:lines.txt"])
            .current_dir(&fixture.worktree)
            .output()
            .expect("git show");
        assert_eq!(output.stdout, b"one\ntwo-selected\nthree\n");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn temporary_partial_commit_hook_rejection_keeps_head_and_real_index() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("selected.txt", b"base\n")
            .expect("initial commit");
        let real_index = fixture.worktree.join(".git/index");
        let index_before = fs::read(&real_index).expect("real Index");
        let head_before = fixture.head_oid().expect("head").expect("committed HEAD");
        fixture
            .install_hook("pre-commit", b"#!/bin/sh\nexit 1\n")
            .expect("hook");

        fixture
            .write_file("selected.txt", b"changed\n")
            .expect("edit");
        let temporary_index = fixture.temp_dir.path().join("hook-rejected-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        assert!(read_tree.status().expect("git starts").success());
        let mut add = Command::new("git");
        add.args(["add", "--", "selected.txt"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            add.env(name, value);
        }
        assert!(add.status().expect("git starts").success());

        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let writer = GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location)
            .with_environment(fixture.isolated_environment());
        let operation = OperationId::new();
        let lease = temporary_lease(&temporary_index, operation);
        let result = writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                operation,
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"rejected partial commit\n".to_vec(),
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
                &lease,
                partial_context(operation),
            )
            .await;

        assert!(result.is_err(), "a rejecting hook must not report success");
        assert_eq!(
            fixture
                .head_oid()
                .expect("head")
                .expect("HEAD remains committed"),
            head_before
        );
        assert_eq!(fs::read(&real_index).expect("real Index"), index_before);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn side_effect_observer_detects_hook_worktree_mutation_after_real_commit() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("selected.txt", b"base\n")
            .expect("initial commit");
        fixture
            .write_file("selected.txt", b"changed\n")
            .expect("edit");
        fixture.stage_file("selected.txt").expect("stage");
        fixture
            .install_hook(
                "pre-commit",
                b"#!/bin/sh\nprintf 'hook side effect\\n' > hook-side-effect.txt\nexit 0\n",
            )
            .expect("hook");

        let temporary_index = fixture.temp_dir.path().join("observer-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        assert!(read_tree.status().expect("git starts").success());
        let mut add = Command::new("git");
        add.args(["add", "--", "selected.txt"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            add.env(name, value);
        }
        assert!(add.status().expect("git starts").success());

        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let observer = GitCliCommitSideEffectObserver::new(TokioProcessRunner, location.clone());
        let before = observer
            .capture(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                test_context(),
            )
            .await
            .expect("capture before commit");
        let writer = GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location)
            .with_environment(fixture.isolated_environment());
        let operation = OperationId::new();
        let lease = temporary_lease(&temporary_index, operation);
        writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                operation,
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"hook mutation\n".to_vec(),
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
                &lease,
                partial_context(operation),
            )
            .await
            .expect("commit succeeds before side-effect reconciliation");
        let after = observer
            .observe(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                test_context(),
            )
            .await
            .expect("observe after commit");

        assert_ne!(before.worktree_digest, after.worktree_digest);
        assert!(fixture.worktree.join("hook-side-effect.txt").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn coordinator_persists_recovery_for_external_index_mutation() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("selected.txt", b"base\n")
            .expect("initial commit");
        fixture
            .write_file("selected.txt", b"changed\n")
            .expect("edit");
        fixture.stage_file("selected.txt").expect("stage");
        let location = RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        );
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let probe = GitCliCommitStateProbe::new(TokioProcessRunner, location.clone());
        let state = probe.read(test_context()).await.expect("state");
        let real_index = fixture.worktree.join(".git/index");
        let original_index = fs::read(&real_index).expect("real index");

        let temporary_index = fixture.temp_dir.path().join("coordinator-hook-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        assert!(read_tree.status().expect("git starts").success());
        let patch = b"diff --git a/selected.txt b/selected.txt\n--- a/selected.txt\n+++ b/selected.txt\n@@ -1 +1 @@\n-base\n+changed\n";
        let builder = GitCliCandidateTreeBuilder::new(GitCliExecutor::new(TokioProcessRunner));
        let tree = builder
            .build(
                CandidateTreeRequest {
                    temporary_index: temporary_index.clone(),
                    worktree_root: fixture.worktree.clone(),
                    patch: patch.to_vec(),
                },
                test_context(),
            )
            .await
            .expect("candidate tree");

        let selection = ChangeSelection::WholeFile {
            file: FileIdentity {
                target,
                path: RepoPath::try_from("selected.txt").expect("path"),
                previous_path: None,
                kind: FileKind::Regular,
            },
        };
        let request = SelectionCommitRequest {
            options: CommitOptions {
                message: b"coordinated partial commit\n".to_vec(),
                amend: false,
                allow_empty: false,
                signoff: false,
                author: None,
                committer: None,
                signing: SigningMode::Disable,
            },
            selections: vec![selection],
            fingerprints: vec![SelectionFingerprint {
                expected_head: state.head,
                expected_index: Some(state.index),
                diff_generation: SnapshotRevision::new(1),
                content_digest: vec![1; 32],
            }],
            expected_revision: SnapshotRevision::new(1),
        };
        let mut transaction = PartialCommitTransaction::begin(request).expect("transaction");
        transaction
            .temp_index_prepared(state.head, Some(state.index))
            .expect("temporary index phase");
        transaction.candidate_ready(tree).expect("candidate phase");
        transaction
            .reconciliation_candidate(
                sha2::Sha256::digest(&original_index).into(),
                fs::read(&temporary_index).expect("candidate index"),
            )
            .expect("reconciliation candidate");
        transaction
            .revalidate_before_commit(state.head, Some(state.index))
            .expect("revalidation");

        let journal_store = MemoryJournal::default();
        let journal_snapshot = Arc::clone(&journal_store.record);
        let scope = RecoveryScope::worktree(
            RecoveryPath::unix(
                fixture
                    .worktree
                    .join(".git")
                    .as_os_str()
                    .as_bytes()
                    .to_vec(),
            )
            .expect("scope"),
            RecoveryPath::unix(
                fixture
                    .worktree
                    .join(".git")
                    .as_os_str()
                    .as_bytes()
                    .to_vec(),
            )
            .expect("scope"),
        );
        let writer = GitCliTemporaryIndexCommitWriter::new(TokioProcessRunner, location.clone())
            .with_environment(fixture.isolated_environment());
        let observer = GitCliCommitSideEffectObserver::new(TokioProcessRunner, location.clone());
        let publisher = ExternalIndexMutationPublisher {
            fixture: fixture.worktree.join(".git"),
            inner: GitCliIndexReconciliationPublisher::new(location),
        };
        let coordinator = PartialCommitCoordinator::new(
            writer,
            PartialCommitJournal::new(journal_store, scope),
            observer,
            publisher,
        );
        let index_before_external_write = fs::read(&real_index).expect("real index");
        let operation = OperationId::new();
        let lease = temporary_lease(&temporary_index, operation);
        let error = coordinator
            .commit(
                target,
                operation,
                &mut transaction,
                &lease,
                partial_context(operation),
            )
            .await
            .expect_err("external Index mutation must require recovery");
        assert_eq!(error.kind, GitErrorKind::IndexChangedExternally);
        assert_eq!(transaction.phase, PartialCommitPhase::RecoveryRequired);
        assert!(matches!(
            journal_snapshot
                .lock()
                .expect("journal lock")
                .as_ref()
                .and_then(|record| record.result.as_ref()),
            Some(JournalResult::RecoveryRequired { .. })
        ));
        let index_after_external_write = fs::read(&real_index).expect("real index");
        assert_ne!(index_after_external_write, index_before_external_write);
        assert!(index_after_external_write.ends_with(b"external-index-write"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn required_signing_failure_is_classified_without_changing_head_or_index() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("signed.txt", b"base\n")
            .expect("initial commit");
        fixture
            .write_file("signed.txt", b"changed\n")
            .expect("edit");
        fixture.stage_file("signed.txt").expect("stage");
        let head_before = fixture.head_oid().expect("head").expect("committed HEAD");
        let fake_gpg = fixture.temp_dir.path().join("failing-gpg");
        fs::write(&fake_gpg, b"#!/bin/sh\nexit 1\n").expect("fake gpg");
        fs::set_permissions(&fake_gpg, fs::Permissions::from_mode(0o755)).expect("executable gpg");
        let mut configure = Command::new("git");
        configure
            .args(["config", "--global", "gpg.program"])
            .arg(&fake_gpg)
            .current_dir(&fixture.worktree);
        for (name, value) in fixture.isolated_environment() {
            configure.env(name, value);
        }
        assert!(configure.status().expect("git starts").success());

        let writer = GitCliCommitWriter::new(
            TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        )
        .with_environment(fixture.isolated_environment());
        let (progress, _receiver) = mpsc::channel(4);
        let error = writer
            .commit(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                StandardCommitRequest {
                    options: CommitOptions {
                        message: b"signed commit\n".to_vec(),
                        amend: false,
                        allow_empty: false,
                        signoff: false,
                        author: None,
                        committer: None,
                        signing: SigningMode::Require,
                    },
                    expected_revision: SnapshotRevision::new(1),
                    expected_head: None,
                    expected_index: None,
                },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect_err("required signing must fail");
        assert_eq!(error.kind, GitErrorKind::SigningFailed);
        assert_eq!(
            fixture.head_oid().expect("head").expect("HEAD"),
            head_before
        );
    }
}
