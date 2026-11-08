//! Domain model and persistence contract for application-owned Shelf entries.

use async_trait::async_trait;
use git_domain::{ObjectId, RepoPath, SessionTarget};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ShelfId(Uuid);

impl ShelfId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ShelfId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShelvedFile {
    pub path: RepoPath,
    pub patch: Vec<u8>,
    pub content_digest: Vec<u8>,
}

impl ShelvedFile {
    pub fn new(
        path: RepoPath,
        patch: Vec<u8>,
        content_digest: Vec<u8>,
    ) -> Result<Self, ShelfError> {
        if patch.is_empty() {
            return Err(ShelfError::EmptyPatch);
        }
        if content_digest.is_empty() {
            return Err(ShelfError::MissingDigest);
        }
        Ok(Self {
            path,
            patch,
            content_digest,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchBundle {
    pub schema_version: u32,
    pub files: Vec<ShelvedFile>,
}

impl PatchBundle {
    pub fn new(files: Vec<ShelvedFile>) -> Result<Self, ShelfError> {
        let bundle = Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            files,
        };
        bundle.validate()?;
        Ok(bundle)
    }

    pub fn validate(&self) -> Result<(), ShelfError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ShelfError::UnsupportedSchema);
        }
        if self.files.is_empty() {
            return Err(ShelfError::EmptyBundle);
        }
        let mut paths = std::collections::HashSet::new();
        for file in &self.files {
            if !paths.insert(file.path.clone()) {
                return Err(ShelfError::DuplicatePath);
            }
            if file.patch.is_empty() {
                return Err(ShelfError::EmptyPatch);
            }
            if file.content_digest.is_empty() {
                return Err(ShelfError::MissingDigest);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShelfEntry {
    pub id: ShelfId,
    pub target: SessionTarget,
    pub name: String,
    pub created_at: i64,
    pub base_head: Option<ObjectId>,
    pub bundle: PatchBundle,
}

impl ShelfEntry {
    pub fn new(
        target: SessionTarget,
        name: impl Into<String>,
        created_at: i64,
        base_head: Option<ObjectId>,
        bundle: PatchBundle,
    ) -> Result<Self, ShelfError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ShelfError::EmptyName);
        }
        bundle.validate()?;
        Ok(Self {
            id: ShelfId::new(),
            target,
            name,
            created_at,
            base_head,
            bundle,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShelfBundle {
    pub target: SessionTarget,
    pub schema_version: u32,
    pub revision: u64,
    pub entries: Vec<ShelfEntry>,
}

impl ShelfBundle {
    #[must_use]
    pub fn empty(target: SessionTarget) -> Self {
        Self {
            target,
            schema_version: CURRENT_SCHEMA_VERSION,
            revision: 0,
            entries: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ShelfError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ShelfError::UnsupportedSchema);
        }
        let mut ids = std::collections::HashSet::new();
        for entry in &self.entries {
            if entry.target != self.target || !ids.insert(entry.id) {
                return Err(ShelfError::InvalidEntry);
            }
            entry.bundle.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ShelfError {
    #[error("shelf name must not be empty")]
    EmptyName,
    #[error("shelf patch bundle must contain at least one file")]
    EmptyBundle,
    #[error("shelf patch must not be empty")]
    EmptyPatch,
    #[error("shelf file content digest must not be empty")]
    MissingDigest,
    #[error("shelf bundle contains a duplicate path")]
    DuplicatePath,
    #[error("shelf schema is unsupported")]
    UnsupportedSchema,
    #[error("shelf entry is invalid or belongs to another session")]
    InvalidEntry,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ShelfPersistenceError {
    #[error("shelf persistence I/O failed: {0}")]
    Io(String),
    #[error("shelf persistence encoding failed: {0}")]
    Encoding(String),
    #[error("shelf revision changed externally")]
    RevisionConflict,
    #[error("shelf bundle is invalid: {0}")]
    InvalidBundle(String),
}

#[async_trait]
pub trait ShelfStore: Send + Sync {
    async fn load(&self, target: SessionTarget) -> Result<ShelfBundle, ShelfPersistenceError>;
    async fn save(
        &self,
        expected_revision: u64,
        bundle: ShelfBundle,
    ) -> Result<(), ShelfPersistenceError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{RepositoryId, WorktreeId};

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn file(path: &str) -> ShelvedFile {
        ShelvedFile::new(
            RepoPath::try_from(path).expect("path"),
            b"diff".to_vec(),
            vec![1],
        )
        .expect("file")
    }

    #[test]
    fn rejects_empty_and_duplicate_patch_entries() {
        assert_eq!(PatchBundle::new(Vec::new()), Err(ShelfError::EmptyBundle));
        assert_eq!(
            PatchBundle::new(vec![file("a.txt"), file("a.txt")]),
            Err(ShelfError::DuplicatePath)
        );
    }

    #[test]
    fn validates_entry_identity_and_schema() {
        let target = target();
        let bundle = PatchBundle::new(vec![file("a.txt")]).expect("bundle");
        assert_eq!(
            ShelfEntry::new(target, " ", 0, None, bundle.clone()),
            Err(ShelfError::EmptyName)
        );
        let mut shelf = ShelfBundle::empty(target);
        shelf
            .entries
            .push(ShelfEntry::new(target, "saved", 0, None, bundle).expect("entry"));
        shelf.schema_version = 99;
        assert_eq!(shelf.validate(), Err(ShelfError::UnsupportedSchema));
    }
}
