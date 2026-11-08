//! M6 PERF-001 benchmark for pure UI projections.
//!
//! Run with `cargo test -p git-ui-model --test m6_perf_ui -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use git_domain::{
    ChangeKind, FileChange, FileMode, HeadState, ObjectFormat, ObjectId, OperationId, RepoPath,
    RepositoryId, RepositorySnapshot, SessionTarget, SnapshotConsistency, SnapshotRevision,
    SubmoduleState, WorktreeId, WorktreeSnapshotState,
};
use git_ui_model::{
    BranchPopupViewModel, BranchRowViewModel, BranchSectionViewModel, ChangesTreeModel,
};

const SAMPLES: usize = 21;
const ENTRY_COUNT: usize = 1_000;

fn oid() -> ObjectId {
    ObjectId::from_bytes(ObjectFormat::Sha1, &[7; 20]).expect("valid test oid")
}

fn target() -> SessionTarget {
    SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
}

fn percentile(samples: &mut [Duration], percentile: usize) -> Duration {
    samples.sort_unstable();
    samples[((samples.len() - 1) * percentile).div_ceil(100)]
}

fn snapshot() -> RepositorySnapshot {
    let changes = (0..ENTRY_COUNT)
        .map(|index| FileChange {
            path: RepoPath::try_from(format!("src/file-{index}.rs").as_str()).expect("path"),
            original_path: None,
            index_status: ChangeKind::Modified,
            worktree_status: ChangeKind::Modified,
            head_oid: None,
            index_oid: None,
            head_mode: None::<FileMode>,
            index_mode: None::<FileMode>,
            worktree_mode: None::<FileMode>,
            submodule: SubmoduleState::NOT_SUBMODULE,
            similarity: None,
            conflict: None,
        })
        .collect();
    RepositorySnapshot {
        repository: target().repository(),
        worktree: target().worktree_id(),
        revision: SnapshotRevision::new(1),
        head: HeadState::Detached { oid: oid() },
        operation: None::<OperationId>,
        changes,
        untracked: Vec::new(),
        ahead: None,
        behind: None,
        worktree_state: WorktreeSnapshotState::Available,
        consistency: SnapshotConsistency::Consistent,
    }
}

fn branch_popup() -> BranchPopupViewModel {
    let rows = (0..ENTRY_COUNT)
        .map(|index| {
            let name = format!("feature/benchmark-{index}");
            let reference = git_domain::RefName::new(name.as_bytes().to_vec()).expect("ref");
            BranchRowViewModel {
                reference,
                display_name: name,
                is_current: index == 0,
                is_favorite: index % 10 == 0,
                ahead: Some(index as u64),
                behind: Some((index / 2) as u64),
            }
        })
        .collect();
    BranchPopupViewModel {
        target: target(),
        search_query: "benchmark-9".to_owned(),
        selected_branch: None,
        sections: vec![BranchSectionViewModel {
            title: "Local".to_owned(),
            rows,
        }],
        loading: false,
        error: None,
    }
}

#[test]
#[ignore = "scheduled M6 benchmark"]
fn perf_001_ui_projection_baseline() {
    let snapshot = snapshot();
    let popup = branch_popup();
    let mut changes_samples = Vec::with_capacity(SAMPLES);
    let mut branch_samples = Vec::with_capacity(SAMPLES);

    for _ in 0..SAMPLES {
        let started = Instant::now();
        let model = ChangesTreeModel::from_snapshot(target(), &snapshot);
        assert_eq!(
            model
                .group(git_ui_model::ChangeGroup::Staged)
                .unwrap()
                .entries
                .len(),
            ENTRY_COUNT
        );
        changes_samples.push(started.elapsed());

        let started = Instant::now();
        let visible = popup.visible_sections();
        assert!(!visible.is_empty());
        branch_samples.push(started.elapsed());
    }

    println!("environment.os={}", std::env::consts::OS);
    println!("environment.arch={}", std::env::consts::ARCH);
    println!("dataset.entries={ENTRY_COUNT}");
    println!(
        "changes_tree_ms.p50={:.3} p95={:.3}",
        percentile(&mut changes_samples, 50).as_secs_f64() * 1000.0,
        percentile(&mut changes_samples, 95).as_secs_f64() * 1000.0
    );
    println!(
        "branch_popup_ms.p50={:.3} p95={:.3}",
        percentile(&mut branch_samples, 50).as_secs_f64() * 1000.0,
        percentile(&mut branch_samples, 95).as_secs_f64() * 1000.0
    );
    println!("peak_memory=not_collected");
}
