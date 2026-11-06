//! Lossless domain values produced by Git status readers.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

use crate::{ObjectId, RepoPath};

/// A raw Git ref name as reported by a machine-readable command.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RefName(Vec<u8>);

impl RefName {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, RefNameError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(RefNameError::Empty);
        }
        if let Some(index) = bytes.iter().position(|byte| *byte == 0) {
            return Err(RefNameError::ContainsNul { index });
        }
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn display_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }
}

impl fmt::Debug for RefName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("RefName").field(&self.0).finish()
    }
}

impl Serialize for RefName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for RefName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        Self::new(bytes).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RefNameError {
    #[error("Git ref name must not be empty")]
    Empty,
    #[error("Git ref name contains NUL at byte {index}")]
    ContainsNul { index: usize },
}

/// Per-side change kind from the porcelain v2 `XY` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChangeKind {
    Unmodified,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
}

/// A non-zero Git file mode stored as its numeric octal value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileMode(u32);

impl FileMode {
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Detailed porcelain v2 submodule state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubmoduleState {
    pub is_submodule: bool,
    pub commit_changed: bool,
    pub tracked_content_modified: bool,
    pub untracked_content: bool,
}

impl SubmoduleState {
    pub const NOT_SUBMODULE: Self = Self {
        is_submodule: false,
        commit_changed: false,
        tracked_content_modified: false,
        untracked_content: false,
    };
}

/// An ordinary tracked status record (`1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrdinaryChange {
    pub path: RepoPath,
    pub index_status: ChangeKind,
    pub worktree_status: ChangeKind,
    pub submodule: SubmoduleState,
    pub head_mode: Option<FileMode>,
    pub index_mode: Option<FileMode>,
    pub worktree_mode: Option<FileMode>,
    pub head_oid: Option<ObjectId>,
    pub index_oid: Option<ObjectId>,
}

/// Whether a type-2 similarity record is a rename or copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SimilarityKind {
    Rename,
    Copy,
}

/// Rename/copy similarity in the inclusive 0..=100 range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Similarity {
    pub kind: SimilarityKind,
    pub percent: u8,
}

/// A renamed or copied tracked status record (`2`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenamedChange {
    pub path: RepoPath,
    pub original_path: RepoPath,
    pub index_status: ChangeKind,
    pub worktree_status: ChangeKind,
    pub submodule: SubmoduleState,
    pub head_mode: Option<FileMode>,
    pub index_mode: Option<FileMode>,
    pub worktree_mode: Option<FileMode>,
    pub head_oid: Option<ObjectId>,
    pub index_oid: Option<ObjectId>,
    pub similarity: Similarity,
}

/// Stable classification for the seven unmerged XY combinations emitted by Git.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConflictKind {
    BothDeleted,
    AddedByUs,
    DeletedByThem,
    AddedByThem,
    DeletedByUs,
    BothAdded,
    BothModified,
}

/// An unmerged status record (`u`) with all three index stages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmergedChange {
    pub path: RepoPath,
    pub conflict: ConflictKind,
    pub submodule: SubmoduleState,
    pub stage1_mode: Option<FileMode>,
    pub stage2_mode: Option<FileMode>,
    pub stage3_mode: Option<FileMode>,
    pub worktree_mode: Option<FileMode>,
    pub stage1_oid: Option<ObjectId>,
    pub stage2_oid: Option<ObjectId>,
    pub stage3_oid: Option<ObjectId>,
}

/// One status entry in the order emitted by Git.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatusEntry {
    Ordinary(OrdinaryChange),
    Renamed(RenamedChange),
    Unmerged(UnmergedChange),
    Untracked(RepoPath),
    Ignored(RepoPath),
}

/// `branch.oid` header value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchOid {
    Initial,
    Object(ObjectId),
}

/// `branch.head` header value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchHead {
    Detached,
    Named(RefName),
}

/// Optional branch headers emitted by `git status --branch`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchStatus {
    pub oid: Option<BranchOid>,
    pub head: Option<BranchHead>,
    pub upstream: Option<RefName>,
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
}

/// Complete parsed porcelain v2 status payload.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusData {
    pub branch: BranchStatus,
    pub entries: Vec<StatusEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_name_round_trip_revalidates_raw_bytes() {
        let name = RefName::new(b"refs/heads/feature\xff".to_vec())
            .expect("non-UTF-8 ref bytes should be retained");
        let encoded = serde_json::to_string(&name).expect("ref name should serialize");
        let decoded: RefName = serde_json::from_str(&encoded).expect("ref name should deserialize");
        assert_eq!(decoded, name);

        assert_eq!(RefName::new(Vec::new()), Err(RefNameError::Empty));
        assert_eq!(
            RefName::new(b"refs/heads/bad\0name".to_vec()),
            Err(RefNameError::ContainsNul { index: 14 })
        );
    }
}
