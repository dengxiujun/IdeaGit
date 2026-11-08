//! Read-only three-version content boundary used by conflict resolution.

use async_trait::async_trait;
use git_domain::{FileKind, FileMode, GitResult, ObjectId, RepoPath};

use crate::{ExecutionContext, RepositoryLocation};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileVersions {
    pub head: Option<VersionDescriptor>,
    pub index: Option<VersionDescriptor>,
    pub worktree: Option<VersionDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionDescriptor {
    pub kind: FileKind,
    pub oid: Option<ObjectId>,
    pub mode: Option<FileMode>,
    pub byte_length: Option<u64>,
}

/// Reads file identities without modifying the worktree or Index.
#[async_trait]
pub trait VersionReader: Send + Sync {
    async fn read(
        &self,
        location: &RepositoryLocation,
        path: &RepoPath,
        context: ExecutionContext,
    ) -> GitResult<FileVersions>;
}
