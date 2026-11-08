//! Application-owned changelists.  These values never masquerade as Git Index state.

use async_trait::async_trait;
use git_domain::{
    ChangeSelection, FileIdentity, ObjectId, OperationId, SelectionFingerprint, SessionTarget,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreviewState {
    Ready,
    Confirmed,
    Expired,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangelistPreview {
    pub operation_id: OperationId,
    pub target: SessionTarget,
    pub expected_head: Option<ObjectId>,
    pub expected_index: Option<ObjectId>,
    pub candidate_tree: ObjectId,
    pub selections: Vec<SelectionFingerprint>,
    pub state: PreviewState,
}

impl ChangelistPreview {
    pub fn new(
        operation_id: OperationId,
        target: SessionTarget,
        expected_head: Option<ObjectId>,
        expected_index: Option<ObjectId>,
        candidate_tree: ObjectId,
        selections: Vec<SelectionFingerprint>,
    ) -> Result<Self, ChangelistError> {
        if selections.is_empty() {
            return Err(ChangelistError::EmptyPreview);
        }
        if selections.iter().any(|fingerprint| {
            fingerprint.expected_head != expected_head
                || fingerprint.expected_index != expected_index
        }) {
            return Err(ChangelistError::PreviewIdentityMismatch);
        }
        Ok(Self {
            operation_id,
            target,
            expected_head,
            expected_index,
            candidate_tree,
            selections,
            state: PreviewState::Ready,
        })
    }

    #[allow(clippy::result_large_err)]
    pub fn confirm(
        &mut self,
        target: SessionTarget,
        head: Option<ObjectId>,
        index: Option<ObjectId>,
    ) -> Result<ObjectId, ChangelistError> {
        if self.state != PreviewState::Ready
            || self.target != target
            || self.expected_head != head
            || self.expected_index != index
        {
            self.state = PreviewState::Expired;
            return Err(ChangelistError::PreviewExpired);
        }
        self.state = PreviewState::Confirmed;
        Ok(self.candidate_tree)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChangelistId(Uuid);

impl ChangelistId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ChangelistId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BindingId(Uuid);

impl BindingId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BindingId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingState {
    Valid,
    Stale,
    Ambiguous,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangelistBinding {
    pub id: BindingId,
    pub target: SessionTarget,
    pub file: FileIdentity,
    pub selection: ChangeSelection,
    pub fingerprint: SelectionFingerprint,
    pub state: BindingState,
}

impl ChangelistBinding {
    pub fn new(
        target: SessionTarget,
        file: FileIdentity,
        selection: ChangeSelection,
        fingerprint: SelectionFingerprint,
    ) -> Result<Self, ChangelistError> {
        if file.target != target || selection.target() != target {
            return Err(ChangelistError::TargetMismatch);
        }
        Ok(Self {
            id: BindingId::new(),
            target,
            file,
            selection,
            fingerprint,
            state: BindingState::Valid,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FingerprintObservation {
    pub target: SessionTarget,
    pub head: Option<git_domain::ObjectId>,
    pub index: Option<git_domain::ObjectId>,
    pub diff_generation: git_domain::SnapshotRevision,
}

#[derive(Debug, Default)]
pub struct SelectionFingerprintEngine;

impl SelectionFingerprintEngine {
    pub fn compute(
        observation: FingerprintObservation,
        selection: &ChangeSelection,
        content: &[u8],
    ) -> Result<SelectionFingerprint, ChangelistError> {
        if selection.target() != observation.target {
            return Err(ChangelistError::TargetMismatch);
        }
        Ok(SelectionFingerprint {
            expected_head: observation.head,
            expected_index: observation.index,
            diff_generation: observation.diff_generation,
            content_digest: digest(selection, content)?,
        })
    }

    pub fn verify(
        binding: &ChangelistBinding,
        observation: FingerprintObservation,
        content: &[u8],
    ) -> Result<BindingState, ChangelistError> {
        if binding.target != observation.target || binding.selection.target() != observation.target
        {
            return Err(ChangelistError::TargetMismatch);
        }
        let expected = &binding.fingerprint;
        if expected.expected_head != observation.head
            || expected.expected_index != observation.index
            || expected.diff_generation != observation.diff_generation
        {
            return Ok(BindingState::Stale);
        }
        if expected.content_digest != digest(&binding.selection, content)? {
            return Ok(BindingState::Ambiguous);
        }
        Ok(BindingState::Valid)
    }
}

fn digest(selection: &ChangeSelection, content: &[u8]) -> Result<Vec<u8>, ChangelistError> {
    let selection =
        serde_json::to_vec(selection).map_err(|_| ChangelistError::FingerprintEncoding)?;
    let mut hasher = Sha256::new();
    hasher.update((selection.len() as u64).to_le_bytes());
    hasher.update(selection);
    hasher.update(content);
    Ok(hasher.finalize().to_vec())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changelist {
    pub id: ChangelistId,
    pub target: SessionTarget,
    pub schema_version: u32,
    pub name: String,
    pub description: String,
    pub is_default: bool,
    pub bindings: Vec<ChangelistBinding>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangelistBundle {
    pub target: SessionTarget,
    pub schema_version: u32,
    pub revision: u64,
    pub lists: Vec<Changelist>,
}

impl ChangelistBundle {
    #[must_use]
    pub fn empty(target: SessionTarget) -> Self {
        Self {
            target,
            schema_version: CURRENT_SCHEMA_VERSION,
            revision: 0,
            lists: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ChangelistError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ChangelistError::UnsupportedSchema);
        }
        if self
            .lists
            .iter()
            .any(|list| list.target != self.target || list.schema_version != self.schema_version)
        {
            return Err(ChangelistError::TargetMismatch);
        }
        Ok(())
    }
}

#[async_trait]
pub trait ChangelistStore: Send + Sync {
    async fn load(
        &self,
        target: SessionTarget,
    ) -> Result<ChangelistBundle, ChangelistPersistenceError>;
    async fn save(
        &self,
        expected_revision: u64,
        bundle: ChangelistBundle,
    ) -> Result<(), ChangelistPersistenceError>;
}

impl Changelist {
    pub fn new(
        target: SessionTarget,
        name: impl Into<String>,
        is_default: bool,
        updated_at: i64,
    ) -> Result<Self, ChangelistError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ChangelistError::EmptyName);
        }
        Ok(Self {
            id: ChangelistId::new(),
            target,
            schema_version: CURRENT_SCHEMA_VERSION,
            name,
            description: String::new(),
            is_default,
            bindings: Vec::new(),
            updated_at,
        })
    }

    pub fn add_binding(&mut self, binding: ChangelistBinding) -> Result<(), ChangelistError> {
        if binding.target != self.target {
            return Err(ChangelistError::TargetMismatch);
        }
        self.bindings.push(binding);
        Ok(())
    }

    pub fn mark_binding(
        &mut self,
        id: BindingId,
        state: BindingState,
    ) -> Result<(), ChangelistError> {
        let binding = self
            .bindings
            .iter_mut()
            .find(|binding| binding.id == id)
            .ok_or(ChangelistError::BindingNotFound)?;
        binding.state = state;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ChangelistError {
    #[error("changelist name must not be empty")]
    EmptyName,
    #[error("changelist binding belongs to a different repository session")]
    TargetMismatch,
    #[error("changelist binding was not found")]
    BindingNotFound,
    #[error("selection fingerprint could not be encoded")]
    FingerprintEncoding,
    #[error("preview requires at least one selection fingerprint")]
    EmptyPreview,
    #[error("preview identity does not match its selection fingerprints")]
    PreviewIdentityMismatch,
    #[error("preview is expired, cancelled, already confirmed, or has stale repository identity")]
    PreviewExpired,
    #[error("unsupported changelist schema")]
    UnsupportedSchema,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ChangelistPersistenceError {
    #[error("changelist persistence I/O failed: {0}")]
    Io(String),
    #[error("changelist persistence encoding failed: {0}")]
    Encoding(String),
    #[error("changelist revision changed externally")]
    RevisionConflict,
    #[error("changelist bundle is invalid: {0}")]
    InvalidBundle(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{
        FileKind, LineSpan, OperationId, RepoPath, RepositoryId, SnapshotRevision, WorktreeId,
    };

    fn make_target(generation: u64) -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), generation)
    }
    fn binding(target: SessionTarget) -> ChangelistBinding {
        let path = RepoPath::try_from("src/file.rs").expect("valid path");
        let file = FileIdentity {
            target,
            path: path.clone(),
            previous_path: None,
            kind: FileKind::Regular,
        };
        let selection = ChangeSelection::Lines {
            file: file.clone(),
            hunk: 1,
            ranges: vec![LineSpan { start: 0, end: 1 }],
        };
        ChangelistBinding::new(
            target,
            file,
            selection,
            SelectionFingerprint {
                expected_head: None,
                expected_index: None,
                diff_generation: SnapshotRevision::new(2),
                content_digest: vec![1, 2, 3],
            },
        )
        .expect("binding should be valid")
    }

    #[test]
    fn creates_default_changelist_and_tracks_binding_state() {
        let target = make_target(1);
        let mut list = Changelist::new(target, "Feature", true, 42).expect("list should be valid");
        let binding = binding(target);
        let id = binding.id;
        list.add_binding(binding).expect("binding should be added");
        list.mark_binding(id, BindingState::Stale)
            .expect("state should update");
        assert!(list.is_default);
        assert_eq!(list.bindings[0].state, BindingState::Stale);
    }

    #[test]
    fn rejects_empty_names_and_cross_session_bindings() {
        let first = make_target(1);
        assert_eq!(
            Changelist::new(first, " ", false, 0),
            Err(ChangelistError::EmptyName)
        );
        let mut list = Changelist::new(first, "Default", true, 0).expect("list should be valid");
        assert_eq!(
            list.add_binding(binding(make_target(2))),
            Err(ChangelistError::TargetMismatch)
        );
    }

    #[test]
    fn fingerprint_binds_selection_content_and_repository_state() {
        let target = make_target(1);
        let mut bound = binding(target);
        let observation = FingerprintObservation {
            target,
            head: None,
            index: None,
            diff_generation: SnapshotRevision::new(2),
        };
        bound.fingerprint =
            SelectionFingerprintEngine::compute(observation, &bound.selection, b"selected bytes")
                .expect("fingerprint should compute");
        assert_eq!(
            SelectionFingerprintEngine::verify(&bound, observation, b"selected bytes"),
            Ok(BindingState::Valid)
        );
        assert_eq!(
            SelectionFingerprintEngine::verify(&bound, observation, b"changed bytes"),
            Ok(BindingState::Ambiguous)
        );
    }

    #[test]
    fn fingerprint_marks_state_changes_stale_and_rejects_foreign_target() {
        let target = make_target(1);
        let binding = binding(target);
        let observation = FingerprintObservation {
            target,
            head: None,
            index: None,
            diff_generation: SnapshotRevision::new(2),
        };
        assert_eq!(
            SelectionFingerprintEngine::verify(
                &binding,
                FingerprintObservation {
                    diff_generation: SnapshotRevision::new(3),
                    ..observation
                },
                b"bytes",
            ),
            Ok(BindingState::Stale)
        );
        assert_eq!(
            SelectionFingerprintEngine::compute(
                FingerprintObservation {
                    target: make_target(2),
                    ..observation
                },
                &binding.selection,
                b"bytes",
            ),
            Err(ChangelistError::TargetMismatch)
        );
    }

    #[test]
    fn preview_expires_on_identity_change_and_cannot_be_reused() {
        let target = make_target(1);
        let binding = binding(target);
        let mut preview = ChangelistPreview::new(
            OperationId::new(),
            target,
            None,
            None,
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
                .parse()
                .expect("tree"),
            vec![binding.fingerprint],
        )
        .expect("preview should be valid");
        assert_eq!(
            preview.confirm(
                target,
                None,
                Some(
                    "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
                        .parse()
                        .expect("index")
                )
            ),
            Err(ChangelistError::PreviewExpired)
        );
        assert_eq!(preview.state, PreviewState::Expired);
        assert_eq!(
            preview.confirm(target, None, None),
            Err(ChangelistError::PreviewExpired)
        );
    }
}
