//! Immutable repository snapshots and deterministic snapshot construction.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    BranchHead, BranchOid, BranchStatus, BranchTracking, ChangeKind, ConflictKind, FileMode,
    HeadState, ObjectId, OperationId, RepoPath, RepositoryId, SessionTarget, Similarity,
    SnapshotRevision, StatusData, StatusEntry, SubmoduleState, UnmergedChange, WorktreeId,
};

/// Whether worktree status data is meaningful for this snapshot.
///
/// Bare repositories use [`Self::Unavailable`]; their empty `changes` and `untracked` collections
/// must not be interpreted as a clean worktree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeSnapshotState {
    /// Status was read for a concrete worktree.
    Available,
    /// The session is bare and therefore has no worktree status capability.
    Unavailable,
}

/// The first detected reason why separately collected read data is stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotStaleReason {
    /// `HEAD` changed between the guard reads around status collection.
    HeadChangedDuringRead,
    /// Porcelain branch headers do not describe the final guarded `HEAD`.
    StatusHeadMismatch,
    /// Porcelain tracking headers and the independently read branch relation disagree.
    TrackingMismatch,
}

/// Consistency of data collected during one snapshot refresh attempt.
///
/// A stale snapshot remains explicit data that a refresh scheduler may diagnose or replace. It is
/// never evidence of one atomic Git state and must not authorize a write operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotConsistency {
    /// Guard reads and independent branch headers agree.
    Consistent,
    /// At least one consistency guard failed.
    Stale(SnapshotStaleReason),
}

impl SnapshotConsistency {
    /// Returns whether all available consistency guards agreed.
    #[must_use]
    pub const fn is_consistent(self) -> bool {
        matches!(self, Self::Consistent)
    }
}

/// Conflict stages retained for an unmerged file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictMetadata {
    /// Stable classification of the porcelain v2 unmerged `XY` pair.
    pub kind: ConflictKind,
    /// Stage 1 (merge base) mode, if present.
    pub stage1_mode: Option<FileMode>,
    /// Stage 2 mode, if present.
    pub stage2_mode: Option<FileMode>,
    /// Stage 3 mode, if present.
    pub stage3_mode: Option<FileMode>,
    /// Stage 1 (merge base) object ID, if present.
    pub stage1_oid: Option<ObjectId>,
    /// Stage 2 object ID, if present.
    pub stage2_oid: Option<ObjectId>,
    /// Stage 3 object ID, if present.
    pub stage3_oid: Option<ObjectId>,
}

/// One tracked or conflicted path in a repository snapshot.
///
/// Untracked paths are stored separately on [`RepositorySnapshot`]. Ignored records are not part
/// of the snapshot contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// Current repository-relative path.
    pub path: RepoPath,
    /// Original path for a rename or copy.
    pub original_path: Option<RepoPath>,
    /// Change between `HEAD` and the Index.
    pub index_status: ChangeKind,
    /// Change between the Index and worktree.
    pub worktree_status: ChangeKind,
    /// Object ID from `HEAD`, when the status record provides one.
    pub head_oid: Option<ObjectId>,
    /// Object ID from the Index, when the status record provides one.
    pub index_oid: Option<ObjectId>,
    /// Mode from `HEAD`, when present.
    pub head_mode: Option<FileMode>,
    /// Mode from the Index, when present.
    pub index_mode: Option<FileMode>,
    /// Mode from the worktree, when present.
    pub worktree_mode: Option<FileMode>,
    /// Detailed submodule state without flattening it into a display string.
    pub submodule: SubmoduleState,
    /// Rename or copy score for a porcelain type-2 record.
    pub similarity: Option<Similarity>,
    /// All conflict stages for an unmerged record.
    pub conflict: Option<ConflictMetadata>,
}

impl FileChange {
    /// Returns whether the Index differs from `HEAD`.
    #[must_use]
    pub const fn is_staged(&self) -> bool {
        !matches!(self.index_status, ChangeKind::Unmodified)
    }

    /// Returns whether the worktree differs from the Index.
    #[must_use]
    pub const fn is_unstaged(&self) -> bool {
        !matches!(self.worktree_status, ChangeKind::Unmodified)
    }

    /// Returns whether the entry carries unmerged index stages.
    #[must_use]
    pub const fn is_conflicted(&self) -> bool {
        self.conflict.is_some()
    }
}

/// Immutable state published for one repository-session refresh generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositorySnapshot {
    /// Logical repository identity derived from the normalized common Git directory.
    pub repository: RepositoryId,
    /// Concrete worktree identity, or `None` for a bare repository.
    pub worktree: Option<WorktreeId>,
    /// Monotonic application revision scoped to the owning session generation.
    pub revision: SnapshotRevision,
    /// Final guarded `HEAD` observation for this refresh attempt.
    pub head: HeadState,
    /// Operation associated with the refresh, when one triggered it.
    pub operation: Option<OperationId>,
    /// Tracked, renamed/copied, and conflicted entries in Git output order.
    pub changes: Vec<FileChange>,
    /// Untracked paths in Git output order.
    pub untracked: Vec<RepoPath>,
    /// Commits ahead of the configured upstream, when tracking exists.
    pub ahead: Option<u64>,
    /// Commits behind the configured upstream, when tracking exists.
    pub behind: Option<u64>,
    /// Whether a worktree status read applies to this target.
    pub worktree_state: WorktreeSnapshotState,
    /// Cross-read consistency result; stale snapshots cannot authorize writes.
    pub consistency: SnapshotConsistency,
}

/// Data collected between two `HEAD` guard reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRead {
    head_before: HeadState,
    head_after: HeadState,
    status: Option<StatusData>,
    tracking: Option<BranchTracking>,
}

impl SnapshotRead {
    /// Creates a refresh input for a concrete worktree.
    #[must_use]
    pub fn worktree(
        head_before: HeadState,
        status: StatusData,
        tracking: Option<BranchTracking>,
        head_after: HeadState,
    ) -> Self {
        Self {
            head_before,
            head_after,
            status: Some(status),
            tracking,
        }
    }

    /// Creates a refresh input for a bare repository without inventing worktree status.
    #[must_use]
    pub fn bare(
        head_before: HeadState,
        tracking: Option<BranchTracking>,
        head_after: HeadState,
    ) -> Self {
        Self {
            head_before,
            head_after,
            status: None,
            tracking,
        }
    }
}

/// Pure builder that assigns one revision and projects reader DTOs into a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotBuilder {
    target: SessionTarget,
    previous_revision: SnapshotRevision,
    operation: Option<OperationId>,
}

impl SnapshotBuilder {
    /// Creates a builder for one live session generation.
    #[must_use]
    pub const fn new(target: SessionTarget, previous_revision: SnapshotRevision) -> Self {
        Self {
            target,
            previous_revision,
            operation: None,
        }
    }

    /// Associates the refresh with the operation that triggered it.
    #[must_use]
    pub const fn with_operation(mut self, operation: OperationId) -> Self {
        self.operation = Some(operation);
        self
    }

    /// Builds the next immutable snapshot or rejects an invalid target/read combination.
    pub fn build(self, read: SnapshotRead) -> Result<RepositorySnapshot, SnapshotBuildError> {
        let revision = self
            .previous_revision
            .checked_next()
            .ok_or(SnapshotBuildError::RevisionExhausted)?;

        let consistency = read_consistency(
            &read.head_before,
            &read.head_after,
            read.status.as_ref().map(|status| &status.branch),
            read.tracking.as_ref(),
        );
        let (worktree_state, changes, untracked) = match (self.target.is_bare(), read.status) {
            (false, Some(status)) => {
                let (changes, untracked) = project_status_entries(status.entries);
                (WorktreeSnapshotState::Available, changes, untracked)
            }
            (true, None) => (WorktreeSnapshotState::Unavailable, Vec::new(), Vec::new()),
            (false, None) => return Err(SnapshotBuildError::WorktreeStatusRequired),
            (true, Some(_)) => return Err(SnapshotBuildError::WorktreeStatusForbidden),
        };
        let (ahead, behind) = read.tracking.map_or((None, None), |tracking| {
            (Some(tracking.ahead), Some(tracking.behind))
        });

        Ok(RepositorySnapshot {
            repository: self.target.repository(),
            worktree: self.target.worktree_id(),
            revision,
            head: read.head_after,
            operation: self.operation,
            changes,
            untracked,
            ahead,
            behind,
            worktree_state,
            consistency,
        })
    }
}

/// Invalid inputs that prevent snapshot construction.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum SnapshotBuildError {
    /// The session revision counter cannot advance without wrapping.
    #[error("snapshot revision counter is exhausted")]
    RevisionExhausted,
    /// A worktree target must include one status read.
    #[error("worktree snapshot requires status data")]
    WorktreeStatusRequired,
    /// A bare target cannot claim worktree status data.
    #[error("bare snapshot must not contain worktree status data")]
    WorktreeStatusForbidden,
}

fn read_consistency(
    head_before: &HeadState,
    head_after: &HeadState,
    status: Option<&BranchStatus>,
    tracking: Option<&BranchTracking>,
) -> SnapshotConsistency {
    if head_before != head_after {
        return SnapshotConsistency::Stale(SnapshotStaleReason::HeadChangedDuringRead);
    }
    if let Some(status) = status
        && !status_head_matches(status, head_after)
    {
        return SnapshotConsistency::Stale(SnapshotStaleReason::StatusHeadMismatch);
    }
    if !tracking_matches(head_after, status, tracking) {
        return SnapshotConsistency::Stale(SnapshotStaleReason::TrackingMismatch);
    }
    SnapshotConsistency::Consistent
}

fn status_head_matches(status: &BranchStatus, head: &HeadState) -> bool {
    let oid_matches = match (&status.oid, head) {
        (Some(BranchOid::Initial), HeadState::Unborn { .. }) => true,
        (Some(BranchOid::Object(actual)), HeadState::Attached { oid, .. })
        | (Some(BranchOid::Object(actual)), HeadState::Detached { oid }) => actual == oid,
        _ => false,
    };
    let name_matches = match (&status.head, head) {
        (Some(BranchHead::Named(actual)), HeadState::Unborn { branch })
        | (Some(BranchHead::Named(actual)), HeadState::Attached { branch, .. }) => actual == branch,
        (Some(BranchHead::Detached), HeadState::Detached { .. }) => true,
        _ => false,
    };
    oid_matches && name_matches
}

fn tracking_matches(
    head: &HeadState,
    status: Option<&BranchStatus>,
    tracking: Option<&BranchTracking>,
) -> bool {
    if !matches!(head, HeadState::Attached { .. }) && tracking.is_some() {
        return false;
    }
    let Some(status) = status else {
        return true;
    };
    match tracking {
        Some(tracking) => {
            status.upstream.is_some()
                && status.ahead == Some(tracking.ahead)
                && status.behind == Some(tracking.behind)
        }
        None => status.upstream.is_none() && status.ahead.is_none() && status.behind.is_none(),
    }
}

fn project_status_entries(entries: Vec<StatusEntry>) -> (Vec<FileChange>, Vec<RepoPath>) {
    let mut changes = Vec::with_capacity(entries.len());
    let mut untracked = Vec::new();
    for entry in entries {
        match entry {
            StatusEntry::Ordinary(change) => changes.push(FileChange {
                path: change.path,
                original_path: None,
                index_status: change.index_status,
                worktree_status: change.worktree_status,
                head_oid: change.head_oid,
                index_oid: change.index_oid,
                head_mode: change.head_mode,
                index_mode: change.index_mode,
                worktree_mode: change.worktree_mode,
                submodule: change.submodule,
                similarity: None,
                conflict: None,
            }),
            StatusEntry::Renamed(change) => changes.push(FileChange {
                path: change.path,
                original_path: Some(change.original_path),
                index_status: change.index_status,
                worktree_status: change.worktree_status,
                head_oid: change.head_oid,
                index_oid: change.index_oid,
                head_mode: change.head_mode,
                index_mode: change.index_mode,
                worktree_mode: change.worktree_mode,
                submodule: change.submodule,
                similarity: Some(change.similarity),
                conflict: None,
            }),
            StatusEntry::Unmerged(change) => changes.push(project_unmerged(change)),
            StatusEntry::Untracked(path) => untracked.push(path),
            StatusEntry::Ignored(_) => {}
        }
    }
    changes.shrink_to_fit();
    untracked.shrink_to_fit();
    (changes, untracked)
}

fn project_unmerged(change: UnmergedChange) -> FileChange {
    let (index_status, worktree_status) = conflict_statuses(change.conflict);
    FileChange {
        path: change.path,
        original_path: None,
        index_status,
        worktree_status,
        head_oid: None,
        index_oid: None,
        head_mode: None,
        index_mode: None,
        worktree_mode: change.worktree_mode,
        submodule: change.submodule,
        similarity: None,
        conflict: Some(ConflictMetadata {
            kind: change.conflict,
            stage1_mode: change.stage1_mode,
            stage2_mode: change.stage2_mode,
            stage3_mode: change.stage3_mode,
            stage1_oid: change.stage1_oid,
            stage2_oid: change.stage2_oid,
            stage3_oid: change.stage3_oid,
        }),
    }
}

const fn conflict_statuses(kind: ConflictKind) -> (ChangeKind, ChangeKind) {
    match kind {
        ConflictKind::BothDeleted => (ChangeKind::Deleted, ChangeKind::Deleted),
        ConflictKind::AddedByUs => (ChangeKind::Added, ChangeKind::Unmerged),
        ConflictKind::DeletedByThem => (ChangeKind::Unmerged, ChangeKind::Deleted),
        ConflictKind::AddedByThem => (ChangeKind::Unmerged, ChangeKind::Added),
        ConflictKind::DeletedByUs => (ChangeKind::Deleted, ChangeKind::Unmerged),
        ConflictKind::BothAdded => (ChangeKind::Added, ChangeKind::Added),
        ConflictKind::BothModified => (ChangeKind::Unmerged, ChangeKind::Unmerged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ObjectFormat, OrdinaryChange, RefName, RenamedChange, SimilarityKind, StatusEntry,
    };

    fn oid(byte: u8) -> ObjectId {
        ObjectId::from_bytes(ObjectFormat::Sha1, &[byte; 20]).expect("test OID should be valid")
    }

    fn path(value: &[u8]) -> RepoPath {
        RepoPath::new(value.to_vec()).expect("test path should be valid")
    }

    fn branch(value: &[u8]) -> RefName {
        RefName::new(value.to_vec()).expect("test ref should be valid")
    }

    fn attached() -> HeadState {
        HeadState::Attached {
            branch: branch(b"main"),
            oid: oid(1),
        }
    }

    fn matching_branch() -> BranchStatus {
        BranchStatus {
            oid: Some(BranchOid::Object(oid(1))),
            head: Some(BranchHead::Named(branch(b"main"))),
            upstream: Some(branch(b"origin/main")),
            ahead: Some(2),
            behind: Some(3),
        }
    }

    fn tracking() -> BranchTracking {
        BranchTracking {
            upstream: branch(b"refs/remotes/origin/main"),
            ahead: 2,
            behind: 3,
        }
    }

    #[test]
    fn projects_complete_status_without_losing_conflict_or_path_metadata() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 4);
        let operation = OperationId::new();
        let ordinary = OrdinaryChange {
            path: path(b"ordinary.txt"),
            index_status: ChangeKind::Modified,
            worktree_status: ChangeKind::Deleted,
            submodule: SubmoduleState::NOT_SUBMODULE,
            head_mode: Some(FileMode::new(0o100644)),
            index_mode: Some(FileMode::new(0o100755)),
            worktree_mode: None,
            head_oid: Some(oid(2)),
            index_oid: Some(oid(3)),
        };
        let renamed = RenamedChange {
            path: path(b"renamed.txt"),
            original_path: path(b"original.txt"),
            index_status: ChangeKind::Renamed,
            worktree_status: ChangeKind::Modified,
            submodule: SubmoduleState::NOT_SUBMODULE,
            head_mode: Some(FileMode::new(0o100644)),
            index_mode: Some(FileMode::new(0o100644)),
            worktree_mode: Some(FileMode::new(0o100644)),
            head_oid: Some(oid(4)),
            index_oid: Some(oid(5)),
            similarity: Similarity {
                kind: SimilarityKind::Rename,
                percent: 87,
            },
        };
        let unmerged = UnmergedChange {
            path: path(b"conflict.txt"),
            conflict: ConflictKind::DeletedByThem,
            submodule: SubmoduleState::NOT_SUBMODULE,
            stage1_mode: Some(FileMode::new(0o100644)),
            stage2_mode: Some(FileMode::new(0o100644)),
            stage3_mode: None,
            worktree_mode: Some(FileMode::new(0o100644)),
            stage1_oid: Some(oid(6)),
            stage2_oid: Some(oid(7)),
            stage3_oid: None,
        };
        let status = StatusData {
            branch: matching_branch(),
            entries: vec![
                StatusEntry::Ordinary(ordinary.clone()),
                StatusEntry::Renamed(renamed.clone()),
                StatusEntry::Untracked(path(b"new.txt")),
                StatusEntry::Ignored(path(b"ignored.log")),
                StatusEntry::Unmerged(unmerged.clone()),
            ],
        };

        let snapshot = SnapshotBuilder::new(target, SnapshotRevision::new(9))
            .with_operation(operation)
            .build(SnapshotRead::worktree(
                attached(),
                status,
                Some(tracking()),
                attached(),
            ))
            .expect("consistent read should build");

        assert_eq!(snapshot.repository, target.repository());
        assert_eq!(snapshot.worktree, target.worktree_id());
        assert_eq!(snapshot.revision, SnapshotRevision::new(10));
        assert_eq!(snapshot.operation, Some(operation));
        assert_eq!(snapshot.worktree_state, WorktreeSnapshotState::Available);
        assert_eq!(snapshot.consistency, SnapshotConsistency::Consistent);
        assert_eq!((snapshot.ahead, snapshot.behind), (Some(2), Some(3)));
        assert_eq!(snapshot.untracked, vec![path(b"new.txt")]);
        assert_eq!(snapshot.changes.len(), 3);

        let first = &snapshot.changes[0];
        assert_eq!(first.path, ordinary.path);
        assert_eq!(first.head_oid, ordinary.head_oid);
        assert!(first.is_staged());
        assert!(first.is_unstaged());

        let second = &snapshot.changes[1];
        assert_eq!(second.original_path.as_ref(), Some(&renamed.original_path));
        assert_eq!(second.similarity, Some(renamed.similarity));

        let third = &snapshot.changes[2];
        assert_eq!(
            (third.index_status, third.worktree_status),
            (ChangeKind::Unmerged, ChangeKind::Deleted)
        );
        let conflict = third
            .conflict
            .as_ref()
            .expect("conflict metadata should exist");
        assert_eq!(conflict.kind, unmerged.conflict);
        assert_eq!(conflict.stage2_oid, unmerged.stage2_oid);
    }

    #[test]
    fn marks_head_status_and_tracking_races_stale() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let status = || StatusData {
            branch: matching_branch(),
            entries: Vec::new(),
        };

        let changed = SnapshotBuilder::new(target, SnapshotRevision::INITIAL)
            .build(SnapshotRead::worktree(
                attached(),
                status(),
                Some(tracking()),
                HeadState::Detached { oid: oid(8) },
            ))
            .expect("stale data should remain explicit");
        assert_eq!(
            changed.consistency,
            SnapshotConsistency::Stale(SnapshotStaleReason::HeadChangedDuringRead)
        );

        let mut wrong_head = status();
        wrong_head.branch.oid = Some(BranchOid::Object(oid(9)));
        let mismatched = SnapshotBuilder::new(target, SnapshotRevision::INITIAL)
            .build(SnapshotRead::worktree(
                attached(),
                wrong_head,
                Some(tracking()),
                attached(),
            ))
            .expect("stale data should remain explicit");
        assert_eq!(
            mismatched.consistency,
            SnapshotConsistency::Stale(SnapshotStaleReason::StatusHeadMismatch)
        );

        let mut wrong_tracking = tracking();
        wrong_tracking.behind = 99;
        let mismatched = SnapshotBuilder::new(target, SnapshotRevision::INITIAL)
            .build(SnapshotRead::worktree(
                attached(),
                status(),
                Some(wrong_tracking),
                attached(),
            ))
            .expect("stale data should remain explicit");
        assert_eq!(
            mismatched.consistency,
            SnapshotConsistency::Stale(SnapshotStaleReason::TrackingMismatch)
        );
    }

    #[test]
    fn validates_unborn_and_detached_status_headers() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let unborn = HeadState::Unborn {
            branch: branch(b"main"),
        };
        let unborn_status = StatusData {
            branch: BranchStatus {
                oid: Some(BranchOid::Initial),
                head: Some(BranchHead::Named(branch(b"main"))),
                ..BranchStatus::default()
            },
            entries: Vec::new(),
        };
        let snapshot = SnapshotBuilder::new(target, SnapshotRevision::INITIAL)
            .build(SnapshotRead::worktree(
                unborn.clone(),
                unborn_status,
                None,
                unborn,
            ))
            .expect("unborn snapshot should build");
        assert!(snapshot.consistency.is_consistent());

        let detached = HeadState::Detached { oid: oid(5) };
        let detached_status = StatusData {
            branch: BranchStatus {
                oid: Some(BranchOid::Object(oid(5))),
                head: Some(BranchHead::Detached),
                ..BranchStatus::default()
            },
            entries: Vec::new(),
        };
        let snapshot = SnapshotBuilder::new(target, snapshot.revision)
            .build(SnapshotRead::worktree(
                detached.clone(),
                detached_status,
                None,
                detached,
            ))
            .expect("detached snapshot should build");
        assert_eq!(snapshot.revision, SnapshotRevision::new(2));
        assert!(snapshot.consistency.is_consistent());
    }

    #[test]
    fn preserves_every_unmerged_xy_pair_in_snapshot_projection() {
        let cases = [
            (
                ConflictKind::BothDeleted,
                (ChangeKind::Deleted, ChangeKind::Deleted),
            ),
            (
                ConflictKind::AddedByUs,
                (ChangeKind::Added, ChangeKind::Unmerged),
            ),
            (
                ConflictKind::DeletedByThem,
                (ChangeKind::Unmerged, ChangeKind::Deleted),
            ),
            (
                ConflictKind::AddedByThem,
                (ChangeKind::Unmerged, ChangeKind::Added),
            ),
            (
                ConflictKind::DeletedByUs,
                (ChangeKind::Deleted, ChangeKind::Unmerged),
            ),
            (
                ConflictKind::BothAdded,
                (ChangeKind::Added, ChangeKind::Added),
            ),
            (
                ConflictKind::BothModified,
                (ChangeKind::Unmerged, ChangeKind::Unmerged),
            ),
        ];

        for (kind, expected) in cases {
            assert_eq!(conflict_statuses(kind), expected);
        }
    }

    #[test]
    fn bare_snapshot_is_not_reported_as_a_clean_worktree() {
        let target = SessionTarget::bare(RepositoryId::new(), 2);
        let snapshot = SnapshotBuilder::new(target, SnapshotRevision::INITIAL)
            .build(SnapshotRead::bare(attached(), None, attached()))
            .expect("bare snapshot should build");

        assert_eq!(snapshot.worktree, None);
        assert_eq!(snapshot.worktree_state, WorktreeSnapshotState::Unavailable);
        assert!(snapshot.changes.is_empty());
        assert!(snapshot.untracked.is_empty());
        assert!(snapshot.consistency.is_consistent());
    }

    #[test]
    fn rejects_target_kind_mismatch_and_revision_exhaustion() {
        let worktree = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let bare = SessionTarget::bare(RepositoryId::new(), 1);
        assert_eq!(
            SnapshotBuilder::new(worktree, SnapshotRevision::INITIAL).build(SnapshotRead::bare(
                attached(),
                None,
                attached()
            )),
            Err(SnapshotBuildError::WorktreeStatusRequired)
        );
        assert_eq!(
            SnapshotBuilder::new(bare, SnapshotRevision::INITIAL).build(SnapshotRead::worktree(
                attached(),
                StatusData {
                    branch: matching_branch(),
                    entries: Vec::new(),
                },
                Some(tracking()),
                attached(),
            )),
            Err(SnapshotBuildError::WorktreeStatusForbidden)
        );
        assert_eq!(
            SnapshotBuilder::new(worktree, SnapshotRevision::new(u64::MAX)).build(
                SnapshotRead::worktree(
                    attached(),
                    StatusData {
                        branch: matching_branch(),
                        entries: Vec::new(),
                    },
                    Some(tracking()),
                    attached(),
                )
            ),
            Err(SnapshotBuildError::RevisionExhausted)
        );
    }

    #[test]
    fn snapshot_round_trip_preserves_identity_and_raw_paths() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 9);
        let snapshot = SnapshotBuilder::new(target, SnapshotRevision::INITIAL)
            .build(SnapshotRead::worktree(
                attached(),
                StatusData {
                    branch: matching_branch(),
                    entries: vec![StatusEntry::Untracked(path(b"raw\xff-name"))],
                },
                Some(tracking()),
                attached(),
            ))
            .expect("snapshot should build");

        let encoded = serde_json::to_string(&snapshot).expect("snapshot should serialize");
        let decoded: RepositorySnapshot =
            serde_json::from_str(&encoded).expect("snapshot should deserialize");
        assert_eq!(decoded, snapshot);
    }
}
