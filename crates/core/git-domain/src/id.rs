//! Strong identities used across repository sessions and operations.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Allocates a new random identity.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Wraps an existing UUID without changing it.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the underlying UUID.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

uuid_id!(
    RepositoryId,
    "Logical identity assigned to one normalized common Git directory."
);
uuid_id!(WorktreeId, "Identity of one concrete Git worktree.");
uuid_id!(OperationId, "Identity of one application operation.");

/// Identifies one live generation of a repository session.
///
/// Bare repositories use `None` for `worktree`. A generation is never reused after its session is
/// closed, allowing consumers to reject results produced by an obsolete session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionTarget {
    repository: RepositoryId,
    worktree: Option<WorktreeId>,
    session_generation: u64,
}

impl SessionTarget {
    /// Creates a target for a worktree-backed repository session.
    #[must_use]
    pub const fn worktree(
        repository: RepositoryId,
        worktree: WorktreeId,
        session_generation: u64,
    ) -> Self {
        Self {
            repository,
            worktree: Some(worktree),
            session_generation,
        }
    }

    /// Creates a target for a bare repository session.
    #[must_use]
    pub const fn bare(repository: RepositoryId, session_generation: u64) -> Self {
        Self {
            repository,
            worktree: None,
            session_generation,
        }
    }

    /// Returns the logical repository identity.
    #[must_use]
    pub const fn repository(self) -> RepositoryId {
        self.repository
    }

    /// Returns the concrete worktree identity, if the session has a worktree.
    #[must_use]
    pub const fn worktree_id(self) -> Option<WorktreeId> {
        self.worktree
    }

    /// Returns the session generation.
    #[must_use]
    pub const fn session_generation(self) -> u64 {
        self.session_generation
    }

    /// Returns whether this target represents a bare repository session.
    #[must_use]
    pub const fn is_bare(self) -> bool {
        self.worktree.is_none()
    }
}

/// Monotonic revision scoped to a [`SessionTarget`].
///
/// A revision is an application generation, not proof of an atomic Git snapshot.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SnapshotRevision(u64);

impl SnapshotRevision {
    /// The initial revision before the first published snapshot.
    pub const INITIAL: Self = Self(0);

    /// Creates a revision from its stored numeric value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric revision value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next revision, or `None` if the counter is exhausted.
    #[must_use]
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use super::*;

    #[test]
    fn identity_types_remain_distinct_and_round_trip() {
        assert_ne!(TypeId::of::<RepositoryId>(), TypeId::of::<WorktreeId>());
        assert_ne!(TypeId::of::<RepositoryId>(), TypeId::of::<OperationId>());

        let repository = RepositoryId::new();
        let encoded = serde_json::to_string(&repository).expect("repository ID should serialize");
        let decoded = serde_json::from_str(&encoded).expect("repository ID should deserialize");
        assert_eq!(repository, decoded);
    }

    #[test]
    fn session_target_distinguishes_worktrees_from_bare_repositories() {
        let repository = RepositoryId::new();
        let worktree = WorktreeId::new();
        let target = SessionTarget::worktree(repository, worktree, 7);

        assert_eq!(target.repository(), repository);
        assert_eq!(target.worktree_id(), Some(worktree));
        assert_eq!(target.session_generation(), 7);
        assert!(!target.is_bare());

        let bare = SessionTarget::bare(repository, 8);
        assert_eq!(bare.worktree_id(), None);
        assert!(bare.is_bare());
    }

    #[test]
    fn snapshot_revision_does_not_wrap() {
        assert_eq!(
            SnapshotRevision::INITIAL.checked_next(),
            Some(SnapshotRevision::new(1))
        );
        assert_eq!(SnapshotRevision::new(u64::MAX).checked_next(), None);
    }
}
