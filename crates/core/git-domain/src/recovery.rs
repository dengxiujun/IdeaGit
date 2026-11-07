//! Persistent repository identity used only for cross-process recovery.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Lossless platform-native path identity stored in the local operation Journal.
///
/// This value is never constructed from a UI/display string. Unix paths retain their original
/// bytes; Windows paths retain their UTF-16 code units.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RecoveryPath(RecoveryPathValue);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "encoding", content = "units", rename_all = "snake_case")]
enum RecoveryPathValue {
    Unix(Vec<u8>),
    Windows(Vec<u16>),
}

impl RecoveryPath {
    /// Creates a lossless Unix path identity.
    pub fn unix(bytes: impl Into<Vec<u8>>) -> Result<Self, RecoveryScopeError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(RecoveryScopeError::EmptyPath);
        }
        Ok(Self(RecoveryPathValue::Unix(bytes)))
    }

    /// Creates a lossless Windows path identity.
    pub fn windows(units: impl Into<Vec<u16>>) -> Result<Self, RecoveryScopeError> {
        let units = units.into();
        if units.is_empty() {
            return Err(RecoveryScopeError::EmptyPath);
        }
        Ok(Self(RecoveryPathValue::Windows(units)))
    }
}

impl fmt::Debug for RecoveryPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            RecoveryPathValue::Unix(bytes) => formatter
                .debug_struct("RecoveryPath")
                .field("encoding", &"unix")
                .field("units", &bytes.len())
                .finish_non_exhaustive(),
            RecoveryPathValue::Windows(units) => formatter
                .debug_struct("RecoveryPath")
                .field("encoding", &"windows")
                .field("units", &units.len())
                .finish_non_exhaustive(),
        }
    }
}

/// Persistent recovery scope independent from a process-local [`crate::SessionTarget`].
///
/// A concrete Git directory is retained in addition to the shared common directory so linked
/// worktrees never share Index recovery records.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RecoveryScope {
    kind: RecoveryScopeKind,
    common_dir: RecoveryPath,
    git_dir: RecoveryPath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RecoveryScopeKind {
    Worktree,
    Bare,
}

impl RecoveryScope {
    #[must_use]
    pub fn worktree(common_dir: RecoveryPath, git_dir: RecoveryPath) -> Self {
        Self {
            kind: RecoveryScopeKind::Worktree,
            common_dir,
            git_dir,
        }
    }

    #[must_use]
    pub fn bare(common_dir: RecoveryPath, git_dir: RecoveryPath) -> Self {
        Self {
            kind: RecoveryScopeKind::Bare,
            common_dir,
            git_dir,
        }
    }

    #[must_use]
    pub const fn is_bare(&self) -> bool {
        matches!(self.kind, RecoveryScopeKind::Bare)
    }
}

impl fmt::Debug for RecoveryScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryScope")
            .field("kind", &if self.is_bare() { "bare" } else { "worktree" })
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum RecoveryScopeError {
    #[error("recovery-scope path is empty")]
    EmptyPath,
    #[error("recovery-scope path encoding is unsupported on this platform")]
    UnsupportedPlatform,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_round_trip_preserves_non_utf8_unix_bytes() {
        let scope = RecoveryScope::worktree(
            RecoveryPath::unix(vec![b'/', b'r', 0x80]).expect("non-empty path"),
            RecoveryPath::unix(vec![b'/', b'g', 0xff]).expect("non-empty path"),
        );
        let json = serde_json::to_vec(&scope).expect("scope should serialize");
        let decoded: RecoveryScope =
            serde_json::from_slice(&json).expect("scope should deserialize");
        assert_eq!(decoded, scope);
        assert!(!String::from_utf8_lossy(&json).contains('\u{fffd}'));
    }

    #[test]
    fn scope_distinguishes_linked_worktrees_and_bare_repositories() {
        let common = RecoveryPath::unix(b"/repo/.git".to_vec()).expect("path");
        let primary = RecoveryScope::worktree(
            common.clone(),
            RecoveryPath::unix(b"/repo/.git".to_vec()).expect("path"),
        );
        let linked = RecoveryScope::worktree(
            common.clone(),
            RecoveryPath::unix(b"/repo/.git/worktrees/linked".to_vec()).expect("path"),
        );
        let bare = RecoveryScope::bare(common.clone(), common);
        assert_ne!(primary, linked);
        assert_ne!(primary, bare);
        assert!(bare.is_bare());
    }

    #[test]
    fn empty_native_path_is_rejected() {
        assert_eq!(
            RecoveryPath::unix(Vec::new()),
            Err(RecoveryScopeError::EmptyPath)
        );
        assert_eq!(
            RecoveryPath::windows(Vec::new()),
            Err(RecoveryScopeError::EmptyPath)
        );
    }
}
