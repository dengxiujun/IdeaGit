//! Repository discovery contracts and normalized adapter locations.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use git_domain::{GitResult, RecoveryPath, RecoveryScope, RecoveryScopeError};

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

    /// Builds the persistent recovery identity from Git-discovered, normalized native paths.
    pub fn recovery_scope(&self) -> Result<RecoveryScope, RecoveryScopeError> {
        let common_dir = recovery_path(&self.common_dir)?;
        let git_dir = recovery_path(&self.git_dir)?;
        Ok(if self.is_bare {
            RecoveryScope::bare(common_dir, git_dir)
        } else {
            RecoveryScope::worktree(common_dir, git_dir)
        })
    }
}

#[cfg(unix)]
fn recovery_path(path: &Path) -> Result<RecoveryPath, RecoveryScopeError> {
    use std::os::unix::ffi::OsStrExt as _;

    RecoveryPath::unix(path.as_os_str().as_bytes().to_vec())
}

#[cfg(windows)]
fn recovery_path(path: &Path) -> Result<RecoveryPath, RecoveryScopeError> {
    use std::os::windows::ffi::OsStrExt as _;

    RecoveryPath::windows(path.as_os_str().encode_wide().collect::<Vec<_>>())
}

#[cfg(not(any(unix, windows)))]
fn recovery_path(_path: &Path) -> Result<RecoveryPath, RecoveryScopeError> {
    Err(RecoveryScopeError::UnsupportedPlatform)
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

    #[test]
    fn recovery_scope_uses_common_and_concrete_git_directories() {
        let primary = RepositoryLocation::worktree("repo", "repo/.git", "repo/.git")
            .recovery_scope()
            .expect("primary scope");
        let linked =
            RepositoryLocation::worktree("linked", "repo/.git/worktrees/linked", "repo/.git")
                .recovery_scope()
                .expect("linked scope");
        let bare = RepositoryLocation::bare("repo.git", "repo.git")
            .recovery_scope()
            .expect("bare scope");
        assert_ne!(primary, linked);
        assert_ne!(primary, bare);
        assert!(bare.is_bare());
    }

    #[cfg(unix)]
    #[test]
    fn recovery_scope_preserves_non_utf8_native_paths() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt as _, path::PathBuf};

        let common = PathBuf::from(OsString::from_vec(vec![b'/', b'r', 0x80]));
        let git_dir = PathBuf::from(OsString::from_vec(vec![b'/', b'g', 0xff]));
        let location = RepositoryLocation::worktree("worktree", git_dir, common);
        let scope = location.recovery_scope().expect("native scope");
        let json = serde_json::to_vec(&scope).expect("scope should serialize");
        assert!(!String::from_utf8_lossy(&json).contains('\u{fffd}'));
    }
}
