//! Candidate previews and commit transactions invoked by the operation coordinator.
//!
//! INFRA-001 skeleton only; behavior is implemented by subsequent tracked tasks.

use git_domain::{GitError, GitErrorKind, GitResult, ObjectId, SelectionCommitRequest};

/// Durable state of a partial-commit transaction. The states deliberately mirror the CCD
/// journal phases and never imply that a real commit is atomic with Index reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartialCommitPhase {
    IntentRecorded,
    SnapshotCaptured,
    TempIndexPrepared,
    PreviewReady,
    PreflightRevalidated,
    CommitInProgress,
    CommitObserved,
    ReconcilingIndex,
    Completed,
    RecoveryRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartialCommitTransaction {
    pub request: SelectionCommitRequest,
    pub expected_head: Option<ObjectId>,
    pub expected_index: Option<ObjectId>,
    pub candidate_tree: Option<ObjectId>,
    pub observed_commit: Option<ObjectId>,
    pub phase: PartialCommitPhase,
}

impl PartialCommitTransaction {
    pub fn begin(request: SelectionCommitRequest) -> GitResult<Self> {
        if request.selections.is_empty() {
            return Err(GitError::new(
                GitErrorKind::InvalidPath,
                "partial commit requires at least one selection",
            ));
        }
        if request.selections.len() != request.fingerprints.len() {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "every partial commit selection requires one fingerprint",
            ));
        }
        let target = request.selections[0].target();
        if request
            .selections
            .iter()
            .any(|selection| selection.target() != target)
        {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "partial commit selections must share one session target",
            ));
        }
        if request
            .fingerprints
            .iter()
            .any(|fingerprint| fingerprint.expected_head != request.fingerprints[0].expected_head)
        {
            return Err(GitError::new(
                GitErrorKind::HeadChangedExternally,
                "partial commit fingerprints disagree about expected HEAD",
            ));
        }
        Ok(Self {
            request,
            expected_head: None,
            expected_index: None,
            candidate_tree: None,
            observed_commit: None,
            phase: PartialCommitPhase::IntentRecorded,
        })
    }

    pub fn candidate_ready(&mut self, tree: ObjectId) -> GitResult<()> {
        if !matches!(
            self.phase,
            PartialCommitPhase::TempIndexPrepared | PartialCommitPhase::PreviewReady
        ) {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "candidate tree is not valid in the current transaction phase",
            ));
        }
        self.candidate_tree = Some(tree);
        self.phase = PartialCommitPhase::PreviewReady;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{
        ChangeSelection, FileIdentity, FileKind, RepoPath, RepositoryId, SelectionFingerprint,
        SessionTarget, SnapshotRevision, WorktreeId,
    };

    fn request() -> SelectionCommitRequest {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let file = FileIdentity {
            target,
            path: RepoPath::try_from("file.txt").expect("path"),
            previous_path: None,
            kind: FileKind::Regular,
        };
        SelectionCommitRequest {
            options: git_domain::CommitOptions {
                message: b"partial".to_vec(),
                amend: false,
                allow_empty: false,
                signoff: false,
                author: None,
                committer: None,
                signing: Default::default(),
            },
            selections: vec![ChangeSelection::WholeFile { file }],
            fingerprints: vec![SelectionFingerprint {
                expected_head: None,
                expected_index: None,
                diff_generation: SnapshotRevision::new(1),
                content_digest: vec![1],
            }],
            expected_revision: SnapshotRevision::new(1),
        }
    }

    #[test]
    fn rejects_mismatched_selection_fingerprints() {
        let mut value = request();
        value.fingerprints.clear();
        assert_eq!(
            PartialCommitTransaction::begin(value).unwrap_err().kind,
            GitErrorKind::StaleSelection
        );
    }

    #[test]
    fn candidate_tree_is_only_accepted_after_temp_index_phase() {
        let mut transaction = PartialCommitTransaction::begin(request()).expect("valid request");
        let tree: ObjectId = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
            .parse()
            .expect("tree");
        assert_eq!(
            transaction.candidate_ready(tree).unwrap_err().kind,
            GitErrorKind::Internal
        );
    }
}
