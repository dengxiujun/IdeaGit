use git_cli::{
    GitCliBranchWriter, GitCliCommitWriter, GitCliHistoryReader, GitCliIndexWriter,
    TokioProcessRunner,
};
use git_domain::{
    BranchCommand, CommitOptions, OperationId, RepoPath, RepositoryId, SessionTarget, SigningMode,
    SnapshotRevision, StandardCommitRequest, WorktreeId,
};
use git_fixture::TestRepository;
use git_ports::{
    BranchWriter, CommitWriter, ExecutionContext, HistoryReader, IndexMutation, IndexWriter,
    RepositoryLocation,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn context() -> ExecutionContext {
    let (progress, _receiver) = mpsc::channel(4);
    ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
}

#[tokio::test]
async fn m2_stage_commit_branch_log_round_trip() {
    let fixture = TestRepository::init().expect("fixture");
    fixture
        .commit_file("base.txt", b"base")
        .expect("initial commit");
    fixture
        .write_file("feature.txt", b"feature")
        .expect("write feature");
    let location = RepositoryLocation::worktree(
        &fixture.worktree,
        fixture.worktree.join(".git"),
        fixture.worktree.join(".git"),
    );
    let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
    GitCliIndexWriter::new(TokioProcessRunner, location.clone())
        .apply(
            IndexMutation::Stage {
                target,
                paths: vec![RepoPath::try_from("feature.txt").expect("path")],
            },
            context(),
        )
        .await
        .expect("stage");
    let commit = GitCliCommitWriter::new(TokioProcessRunner, location.clone())
        .with_environment(fixture.isolated_environment());
    let outcome = commit
        .commit(
            target,
            StandardCommitRequest {
                options: CommitOptions {
                    message: b"feature commit\n".to_vec(),
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
            context(),
        )
        .await
        .expect("commit");
    assert_eq!(outcome.parents.len(), 1);
    let branch = GitCliBranchWriter::new(TokioProcessRunner, location.clone());
    branch
        .apply(
            target,
            BranchCommand::Create {
                name: git_domain::RefName::new(b"feature/demo".to_vec()).expect("branch"),
            },
            context(),
        )
        .await
        .expect("create branch");
    branch
        .apply(
            target,
            BranchCommand::Checkout {
                name: git_domain::RefName::new(b"main".to_vec()).expect("branch"),
            },
            context(),
        )
        .await
        .expect("checkout branch");
    let page = GitCliHistoryReader::new(TokioProcessRunner, location)
        .log(
            target,
            git_domain::LogQuery { limit: 2, skip: 0 },
            context(),
        )
        .await
        .expect("log");
    assert!(page.commits.iter().any(|item| item.id == outcome.commit));
}
