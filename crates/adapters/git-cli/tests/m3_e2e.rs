use std::{fs, process::Command, str::FromStr};

use git_cli::{
    GitCliCloneWriter, GitCliCommitWriter, GitCliConflictResolutionWriter,
    GitCliConflictStageReader, GitCliFileHistoryReader, GitCliInteractiveRebasePlanReader,
    GitCliInteractiveRebaseWriter, GitCliMergeWriter, GitCliRemoteAdapter, GitCliSequencerReader,
    GitCliSequencerWriter, GitCliShelfWriter, TokioProcessRunner,
};
use git_domain::{
    CommitOptions, MergeCommand, OperationId, RefName, RemoteCommand, RepoPath, RepositoryId,
    SequencerCommand, SessionTarget, SigningMode, SnapshotRevision, StandardCommitRequest,
    WorktreeId,
};
use git_fixture::TestRepository;
use git_ports::{
    ApplyResolutionRequest, BlameReader, CloneRequest, CloneWriter, CommitWriter,
    ConflictResolutionWriter, ConflictStageReader, ExecutionContext, FileHistoryReader,
    IndexWriter, InteractiveRebasePlanReader, InteractiveRebaseWriter, MergeWriter, RemoteWriter,
    SequencerReader, SequencerWriter, ShelfWriter, UnshelveRequest,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn context() -> ExecutionContext {
    let (progress, _receiver) = mpsc::channel(4);
    ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
}

fn target() -> SessionTarget {
    SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
}

fn location(root: &std::path::Path) -> git_ports::RepositoryLocation {
    git_ports::RepositoryLocation::worktree(root, root.join(".git"), root.join(".git"))
}

fn git(root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-C", root.to_str().expect("utf8 path")])
        .args(args)
        .output()
        .expect("git should start");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn m5_gate_file_history_and_blame_use_real_repository() {
    let fixture = TestRepository::init().expect("fixture");
    fixture.configure_local_identity().expect("identity");
    git(&fixture.worktree, &["config", "core.autocrlf", "false"]);
    fixture
        .commit_file("tracked.txt", b"first\n")
        .expect("first commit");
    fixture
        .commit_file("tracked.txt", b"first\nsecond\n")
        .expect("second commit");

    let reader = GitCliFileHistoryReader::new(TokioProcessRunner, location(&fixture.worktree));
    let path = RepoPath::try_from("tracked.txt").expect("path");
    let history = reader
        .history(
            git_domain::FileHistoryQuery {
                target: target(),
                path: path.clone(),
                limit: 10,
                skip: 0,
            },
            context(),
        )
        .await
        .expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].path, path);
    assert!(history[0].parent.is_some());
    assert!(!history[0].subject.is_empty());

    let blame = reader
        .blame(target(), path, context())
        .await
        .expect("blame");
    assert_eq!(blame.len(), 2);
    assert_eq!(blame[0].content, b"first".to_vec());
    assert_eq!(blame[1].content, b"second".to_vec());
    assert_ne!(blame[0].commit, blame[1].commit);
}

#[tokio::test]
async fn m3_gate_clone_edit_commit_push_fetch_pull_merge_conflict_resolve_continue() {
    let source = TestRepository::init().expect("source fixture");
    source.configure_local_identity().expect("source identity");
    git(&source.worktree, &["config", "core.autocrlf", "false"]);
    source.commit_file("README.md", b"base\n").expect("base");
    let remote = source.create_local_remote().expect("remote");

    let clone_root = source.temp_dir.path().join("gate-clone");
    GitCliCloneWriter::new(TokioProcessRunner)
        .clone_repository(
            CloneRequest {
                source: remote.path().to_string_lossy().into_owned(),
                destination: git_domain::LocalPath::new(clone_root.clone()),
            },
            context(),
        )
        .await
        .expect("clone");
    git(&clone_root, &["config", "user.name", "IdeaGit Gate"]);
    git(
        &clone_root,
        &["config", "user.email", "gate@ideagit.invalid"],
    );
    git(&clone_root, &["config", "core.autocrlf", "false"]);

    let clone_location = location(&clone_root);
    let clone_target = target();
    fs::write(clone_root.join("remote.txt"), b"from clone\n").expect("edit");
    git_cli::GitCliIndexWriter::new(TokioProcessRunner, clone_location.clone())
        .apply(
            git_ports::IndexMutation::Stage {
                target: clone_target,
                paths: vec![RepoPath::try_from("remote.txt").expect("path")],
            },
            context(),
        )
        .await
        .expect("stage");
    GitCliCommitWriter::new(TokioProcessRunner, clone_location.clone())
        .commit(
            clone_target,
            StandardCommitRequest {
                options: CommitOptions {
                    message: b"remote change\n".to_vec(),
                    amend: false,
                    allow_empty: false,
                    signoff: false,
                    author: None,
                    committer: None,
                    signing: SigningMode::Disable,
                },
                expected_revision: SnapshotRevision::INITIAL,
                expected_head: None,
                expected_index: None,
            },
            context(),
        )
        .await
        .expect("commit");
    let remote_adapter = GitCliRemoteAdapter::new(TokioProcessRunner, clone_location.clone());
    remote_adapter
        .apply(
            clone_target,
            RemoteCommand::Push {
                remote: Some(RefName::new(b"origin".to_vec()).expect("remote")),
                branch: None,
                force_with_lease: false,
            },
            context(),
        )
        .await
        .expect("push");

    let source_location = location(&source.worktree);
    let source_target = target();
    let source_remote = GitCliRemoteAdapter::new(TokioProcessRunner, source_location.clone());
    source_remote
        .apply(
            source_target,
            RemoteCommand::Fetch { remote: None },
            context(),
        )
        .await
        .expect("fetch");
    source_remote
        .apply(
            source_target,
            RemoteCommand::Pull {
                remote: None,
                branch: None,
            },
            context(),
        )
        .await
        .expect("pull");
    assert_eq!(
        fs::read(source.worktree.join("remote.txt")).expect("pulled file"),
        b"from clone\n"
    );

    git(&source.worktree, &["switch", "-c", "gate-conflict"]);
    fs::write(source.worktree.join("conflict.txt"), b"theirs\n").expect("theirs edit");
    git(&source.worktree, &["add", "--", "conflict.txt"]);
    git(&source.worktree, &["commit", "-m", "theirs"]);
    git(&source.worktree, &["switch", "main"]);
    fs::write(source.worktree.join("conflict.txt"), b"ours\n").expect("ours edit");
    git(&source.worktree, &["add", "--", "conflict.txt"]);
    git(&source.worktree, &["commit", "-m", "ours"]);
    GitCliMergeWriter::new(TokioProcessRunner, source_location.clone())
        .apply(
            source_target,
            MergeCommand {
                source: RefName::new(b"gate-conflict".to_vec()).expect("branch"),
                no_ff: false,
            },
            context(),
        )
        .await
        .expect_err("merge should stop at conflict");

    let path = RepoPath::try_from("conflict.txt").expect("path");
    let stages = GitCliConflictStageReader::new(TokioProcessRunner)
        .read(&source_location, context())
        .await
        .expect("stages");
    let conflict = stages
        .iter()
        .find(|item| item.path == path)
        .expect("conflict path");
    let current = fs::read(source.worktree.join("conflict.txt")).expect("conflict worktree");
    GitCliConflictResolutionWriter::new(TokioProcessRunner, source_location.clone())
        .apply(
            ApplyResolutionRequest {
                target: source_target,
                path,
                expected_conflict_revision: SnapshotRevision::INITIAL,
                expected_stage_oids: git_ports::ConflictStages {
                    stage1: conflict.stage1_oid,
                    stage2: conflict.stage2_oid,
                    stage3: conflict.stage3_oid,
                },
                expected_worktree_digest: git_ports::worktree_digest(&current),
                mode: 0o100644,
                result: b"resolved\n".to_vec(),
            },
            context(),
        )
        .await
        .expect("resolve and stage");
    GitCliSequencerWriter::new(TokioProcessRunner, source_location)
        .apply(source_target, SequencerCommand::ContinueMerge, context())
        .await
        .expect("continue merge");
    assert_eq!(
        fs::read(source.worktree.join("conflict.txt")).expect("resolved file"),
        b"resolved\n"
    );
}

#[tokio::test]
async fn m3_gate_rejects_external_conflict_edit_without_overwrite() {
    let fixture = TestRepository::init().expect("fixture");
    fixture
        .configure_local_identity()
        .expect("fixture identity");
    fixture
        .create_conflict("conflict.txt", b"base\n", b"ours\n", b"theirs\n")
        .expect("conflict");
    let location = location(&fixture.worktree);
    let path = RepoPath::try_from("conflict.txt").expect("path");
    let stages = GitCliConflictStageReader::new(TokioProcessRunner)
        .read(&location, context())
        .await
        .expect("stages");
    let conflict = stages.iter().find(|item| item.path == path).expect("path");
    let observed = fs::read(fixture.worktree.join("conflict.txt")).expect("worktree");
    fs::write(fixture.worktree.join("conflict.txt"), b"external edit\n").expect("external edit");

    let error = GitCliConflictResolutionWriter::new(TokioProcessRunner, location)
        .apply(
            ApplyResolutionRequest {
                target: target(),
                path,
                expected_conflict_revision: SnapshotRevision::INITIAL,
                expected_stage_oids: git_ports::ConflictStages {
                    stage1: conflict.stage1_oid,
                    stage2: conflict.stage2_oid,
                    stage3: conflict.stage3_oid,
                },
                expected_worktree_digest: git_ports::worktree_digest(&observed),
                mode: 0o100644,
                result: b"must not overwrite\n".to_vec(),
            },
            context(),
        )
        .await
        .expect_err("external edit must invalidate resolution");
    assert_eq!(error.kind, git_domain::GitErrorKind::StaleSelection);
    assert_eq!(
        fs::read(fixture.worktree.join("conflict.txt")).expect("preserved edit"),
        b"external edit\n"
    );
}

#[tokio::test]
async fn rebase_abort_clears_conflict_without_claiming_success() {
    let fixture = TestRepository::init().expect("fixture");
    fixture
        .configure_local_identity()
        .expect("fixture identity");
    git(&fixture.worktree, &["config", "core.autocrlf", "false"]);
    fixture
        .commit_file("conflict.txt", b"base\n")
        .expect("base");

    git(&fixture.worktree, &["checkout", "-b", "topic"]);
    fs::write(fixture.worktree.join("conflict.txt"), b"topic\n").expect("topic edit");
    git(&fixture.worktree, &["add", "--", "conflict.txt"]);
    git(&fixture.worktree, &["commit", "-m", "topic"]);

    git(&fixture.worktree, &["checkout", "main"]);
    fs::write(fixture.worktree.join("conflict.txt"), b"main\n").expect("main edit");
    git(&fixture.worktree, &["add", "--", "conflict.txt"]);
    git(&fixture.worktree, &["commit", "-m", "main"]);
    git(&fixture.worktree, &["checkout", "topic"]);

    let rebase = Command::new("git")
        .args([
            "-C",
            fixture.worktree.to_str().expect("utf8 path"),
            "rebase",
            "main",
        ])
        .output()
        .expect("rebase should start");
    assert!(!rebase.status.success(), "rebase should stop at conflict");

    let snapshot = GitCliSequencerReader
        .read(&location(&fixture.worktree), context())
        .await
        .expect("rebase state should be readable");
    assert!(snapshot.active);
    assert_eq!(snapshot.kind, git_domain::SequencerKind::Rebase);
    assert!(snapshot.current.is_some());
    assert!(snapshot.total.is_some());

    let location = location(&fixture.worktree);
    GitCliSequencerWriter::new(TokioProcessRunner, location)
        .apply(target(), SequencerCommand::AbortRebase, context())
        .await
        .expect("abort rebase");

    let status = Command::new("git")
        .args([
            "-C",
            fixture.worktree.to_str().expect("utf8 path"),
            "status",
            "--porcelain",
        ])
        .output()
        .expect("status should run");
    assert!(status.status.success());
    assert!(
        status.stdout.is_empty(),
        "abort must restore a clean worktree"
    );
    assert_eq!(
        fs::read(fixture.worktree.join("conflict.txt")).expect("topic file"),
        b"topic\n"
    );
}

#[tokio::test]
async fn shelf_unshelve_applies_a_real_patch_after_preflight() {
    let fixture = TestRepository::init().expect("fixture");
    fixture.configure_local_identity().expect("identity");
    git(&fixture.worktree, &["config", "core.autocrlf", "false"]);
    fixture
        .commit_file("shelf.txt", b"base\n")
        .expect("base commit");
    fs::write(fixture.worktree.join("shelf.txt"), b"base\nfrom shelf\n").expect("edit");
    let patch = Command::new("git")
        .args([
            "-C",
            fixture.worktree.to_str().expect("utf8"),
            "diff",
            "--",
            "shelf.txt",
        ])
        .output()
        .expect("git diff should start");
    assert!(patch.status.success());
    fs::write(fixture.worktree.join("shelf.txt"), b"base\n").expect("restore base");

    GitCliShelfWriter::new(TokioProcessRunner)
        .unshelve(
            &location(&fixture.worktree),
            UnshelveRequest {
                target: target(),
                base_head: None,
                current_head: None,
                worktree_dirty: false,
                patches: vec![patch.stdout],
            },
            context(),
        )
        .await
        .expect("unshelve should apply");
    assert_eq!(
        fs::read(fixture.worktree.join("shelf.txt")).expect("read file"),
        b"base\nfrom shelf\n"
    );
}

#[tokio::test]
async fn interactive_rebase_plan_reads_linear_commits_in_git_order() {
    let fixture = TestRepository::init().expect("fixture");
    fixture.configure_local_identity().expect("identity");
    git(&fixture.worktree, &["config", "core.autocrlf", "false"]);
    fixture
        .commit_file("base.txt", b"base\n")
        .expect("base commit");
    let upstream = Command::new("git")
        .args([
            "-C",
            fixture.worktree.to_str().expect("utf8 path"),
            "rev-parse",
            "HEAD",
        ])
        .output()
        .expect("rev-parse should run");
    let upstream = git_domain::ObjectId::from_str(String::from_utf8_lossy(&upstream.stdout).trim())
        .expect("upstream oid");
    fixture
        .commit_file("first.txt", b"first\n")
        .expect("first commit");
    fixture
        .commit_file("second.txt", b"second\n")
        .expect("second commit");

    let plan =
        GitCliInteractiveRebasePlanReader::new(TokioProcessRunner, location(&fixture.worktree))
            .plan(target(), upstream, context())
            .await
            .expect("plan should be read");

    assert_eq!(plan.upstream, upstream);
    assert_eq!(plan.entries.len(), 2);
    assert!(
        plan.entries
            .iter()
            .all(|entry| entry.action == git_domain::RebaseAction::Pick)
    );
    assert_eq!(
        plan.entries[0].message_override.as_deref(),
        Some("fixture commit")
    );
    assert_eq!(
        plan.entries[1].message_override.as_deref(),
        Some("fixture commit")
    );

    let todo = plan.render_todo().expect("plan should render");
    let reparsed = git_domain::InteractiveRebasePlan::parse_todo(target(), upstream, &todo)
        .expect("rendered todo should parse");
    assert_eq!(reparsed.upstream, plan.upstream);
    assert_eq!(reparsed.entries, plan.entries);

    let mut execution = plan;
    execution.entries[1].action = git_domain::RebaseAction::Drop;
    execution.entries[1].message_override = None;
    GitCliInteractiveRebaseWriter::new(TokioProcessRunner, location(&fixture.worktree))
        .apply(target(), execution, context())
        .await
        .expect("interactive rebase should execute");
    assert!(!fixture.worktree.join("second.txt").exists());
    assert!(fixture.worktree.join("first.txt").exists());
}
