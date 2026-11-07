use git_cli::{GitCliHeadReader, GitCliRepositoryReader, GitCliStatusReader, TokioProcessRunner};
use git_domain::{
    OperationId, RepositoryId, SessionTarget, SnapshotBuilder, SnapshotConsistency, SnapshotRead,
    SnapshotRevision, SnapshotStaleReason, WorktreeId,
};
use git_fixture::TestRepository;
use git_ports::{
    BranchReader, EnvironmentVariable, ExecutionContext, HeadReader, ProcessProgress,
    RepositoryReader, StatusOptions, StatusReader,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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

#[tokio::test]
async fn builds_a_consistent_snapshot_from_real_git_readers() {
    let fixture = TestRepository::init().expect("fixture should initialize");
    fixture
        .commit_file("tracked.txt", b"base\n")
        .expect("base commit should succeed");
    fixture
        .create_local_remote()
        .expect("tracking remote should be created");
    fixture
        .write_file("tracked.txt", b"modified\n")
        .expect("tracked file should change");
    fixture
        .write_file("untracked.txt", b"new\n")
        .expect("untracked file should be created");

    let environment = isolated_environment(&fixture);
    let location = GitCliRepositoryReader::new(TokioProcessRunner)
        .with_environment(environment.clone())
        .discover(fixture.worktree.clone(), context())
        .await
        .expect("fixture repository should be discovered");
    let head_reader =
        GitCliHeadReader::new(TokioProcessRunner).with_environment(environment.clone());
    let status_reader = GitCliStatusReader::new(TokioProcessRunner).with_environment(environment);

    let head_before = head_reader
        .read_head(&location, context())
        .await
        .expect("HEAD guard should be readable");
    let status = status_reader
        .status(&location, StatusOptions::default(), context())
        .await
        .expect("status should be readable");
    let tracking = head_reader
        .read_tracking(&location, context())
        .await
        .expect("tracking state should be readable");
    let head_after = head_reader
        .read_head(&location, context())
        .await
        .expect("final HEAD guard should be readable");

    let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
    let snapshot = SnapshotBuilder::new(target, SnapshotRevision::INITIAL)
        .build(SnapshotRead::worktree(
            head_before,
            status,
            tracking,
            head_after,
        ))
        .expect("reader output should build a snapshot");

    assert_eq!(snapshot.consistency, SnapshotConsistency::Consistent);
    assert_eq!(snapshot.revision, SnapshotRevision::new(1));
    assert_eq!((snapshot.ahead, snapshot.behind), (Some(0), Some(0)));
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].path.as_bytes(), b"tracked.txt");
    assert!(snapshot.changes[0].is_unstaged());
    assert_eq!(snapshot.untracked.len(), 1);
    assert_eq!(snapshot.untracked[0].as_bytes(), b"untracked.txt");
}

#[tokio::test]
async fn detects_a_real_head_change_between_guard_reads() {
    let fixture = TestRepository::init().expect("fixture should initialize");
    fixture
        .commit_file("tracked.txt", b"base\n")
        .expect("base commit should succeed");
    let environment = isolated_environment(&fixture);
    let location = GitCliRepositoryReader::new(TokioProcessRunner)
        .with_environment(environment.clone())
        .discover(fixture.worktree.clone(), context())
        .await
        .expect("fixture repository should be discovered");
    let head_reader =
        GitCliHeadReader::new(TokioProcessRunner).with_environment(environment.clone());
    let status_reader = GitCliStatusReader::new(TokioProcessRunner).with_environment(environment);

    let head_before = head_reader
        .read_head(&location, context())
        .await
        .expect("initial HEAD should be readable");
    fixture
        .commit_file("external.txt", b"external change\n")
        .expect("external commit should succeed");
    let status = status_reader
        .status(&location, StatusOptions::default(), context())
        .await
        .expect("status should be readable after the change");
    let tracking = head_reader
        .read_tracking(&location, context())
        .await
        .expect("tracking state should be readable");
    let head_after = head_reader
        .read_head(&location, context())
        .await
        .expect("changed HEAD should be readable");

    let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
    let snapshot = SnapshotBuilder::new(target, SnapshotRevision::INITIAL)
        .build(SnapshotRead::worktree(
            head_before,
            status,
            tracking,
            head_after,
        ))
        .expect("stale refresh should remain explicit snapshot data");

    assert_eq!(
        snapshot.consistency,
        SnapshotConsistency::Stale(SnapshotStaleReason::HeadChangedDuringRead)
    );
}
