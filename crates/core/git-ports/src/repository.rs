//! Repository discovery contracts and normalized adapter locations.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use git_domain::GitResult;

use crate::ExecutionContext;

/// Git-reported and filesystem-normalized repository paths.
///
/// A bare repository has no worktree root. Linked worktrees have distinct `git_dir` values while
/// sharing a `common_dir` with the primary worktree.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RepositoryLocation {
    worktree_root: Option<PathBuf>,
    git_dir: PathBuf,
    common_dir: PathBuf,
    is_bare: bool,
}

impl RepositoryLocation {
    /// Creates a location for a repository with a concrete worktree.
    #[must_use]
    pub fn worktree(
        worktree_root: impl Into<PathBuf>,
        git_dir: impl Into<PathBuf>,
        common_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            worktree_root: Some(worktree_root.into()),
            git_dir: git_dir.into(),
            common_dir: common_dir.into(),
            is_bare: false,
        }
    }

    /// Creates a location for a bare repository without inventing an empty worktree path.
    #[must_use]
    pub fn bare(git_dir: impl Into<PathBuf>, common_dir: impl Into<PathBuf>) -> Self {
        Self {
            worktree_root: None,
            git_dir: git_dir.into(),
            common_dir: common_dir.into(),
            is_bare: true,
        }
    }

    #[must_use]
    pub fn worktree_root(&self) -> Option<&Path> {
        self.worktree_root.as_deref()
    }

    #[must_use]
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    #[must_use]
    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

    #[must_use]
    pub const fn is_bare(&self) -> bool {
        self.is_bare
    }
}

/// Discovers repository paths through the configured Git implementation.
#[async_trait]
pub trait RepositoryReader: Send + Sync {
    async fn discover(
        &self,
        path: PathBuf,
        context: ExecutionContext,
    ) -> GitResult<RepositoryLocation>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_location_cannot_expose_a_fake_worktree() {
        let bare = RepositoryLocation::bare("repo.git", "repo.git");
        assert!(bare.is_bare());
        assert!(bare.worktree_root().is_none());

        let worktree = RepositoryLocation::worktree("work", "work/.git", "work/.git");
        assert!(!worktree.is_bare());
        assert_eq!(worktree.worktree_root(), Some(Path::new("work")));
    }
}
