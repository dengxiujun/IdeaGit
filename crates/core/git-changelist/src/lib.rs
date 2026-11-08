//! Application-owned changelists.  These values never masquerade as Git Index state.

use git_domain::{ChangeSelection, FileIdentity, SelectionFingerprint, SessionTarget};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{FileKind, LineSpan, RepoPath, RepositoryId, SnapshotRevision, WorktreeId};

    fn target(generation: u64) -> SessionTarget {
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
        let target = target(1);
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
        let first = target(1);
        assert_eq!(
            Changelist::new(first, " ", false, 0),
            Err(ChangelistError::EmptyName)
        );
        let mut list = Changelist::new(first, "Default", true, 0).expect("list should be valid");
        assert_eq!(
            list.add_binding(binding(target(2))),
            Err(ChangelistError::TargetMismatch)
        );
    }
}
