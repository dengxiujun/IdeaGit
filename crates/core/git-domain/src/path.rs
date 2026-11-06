//! Separate value types for Git repository paths and operating-system paths.

use std::{borrow::Cow, fmt, path::Path, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

/// A validated path as stored and reported by Git, preserved as raw bytes.
///
/// `RepoPath` is always relative to a repository root. It is not an OS path and deliberately has no
/// conversion to [`LocalPath`]; adapters own platform-specific decoding and representability checks.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoPath(Vec<u8>);

impl RepoPath {
    /// Validates and owns raw Git path bytes.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, RepoPathError> {
        let bytes = bytes.into();
        validate_repo_path(&bytes)?;
        Ok(Self(bytes))
    }

    /// Returns the original raw Git path bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the value without decoding or normalizing its bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Returns a lossy string intended only for display and diagnostics.
    #[must_use]
    pub fn display_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }
}

impl fmt::Debug for RepoPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("RepoPath").field(&self.0).finish()
    }
}

impl TryFrom<Vec<u8>> for RepoPath {
    type Error = RepoPathError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&[u8]> for RepoPath {
    type Error = RepoPathError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for RepoPath {
    type Error = RepoPathError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value.as_bytes())
    }
}

impl Serialize for RepoPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for RepoPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        Self::new(bytes).map_err(de::Error::custom)
    }
}

/// Validation failures for raw repository-relative Git paths.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RepoPathError {
    /// An empty path cannot identify a repository entry.
    #[error("repository path is empty")]
    Empty,

    /// Git path fields cannot contain NUL because NUL delimits machine-readable output.
    #[error("repository path contains NUL at byte {index}")]
    ContainsNul { index: usize },

    /// Repository paths must not start at a filesystem root.
    #[error("repository path must be relative")]
    Absolute,

    /// Empty components would be normalized by a filesystem and lose identity.
    #[error("repository path contains an empty component at byte {index}")]
    EmptyComponent { index: usize },

    /// A current-directory component would be normalized away.
    #[error("repository path contains a current-directory component at byte {index}")]
    CurrentDirectory { index: usize },

    /// A parent component could escape the repository root.
    #[error("repository path contains parent traversal at byte {index}")]
    ParentTraversal { index: usize },
}

fn validate_repo_path(bytes: &[u8]) -> Result<(), RepoPathError> {
    if bytes.is_empty() {
        return Err(RepoPathError::Empty);
    }
    if bytes[0] == b'/' {
        return Err(RepoPathError::Absolute);
    }
    if let Some(index) = bytes.iter().position(|byte| *byte == 0) {
        return Err(RepoPathError::ContainsNul { index });
    }

    let mut offset = 0;
    for component in bytes.split(|byte| *byte == b'/') {
        let error = match component {
            [] => Some(RepoPathError::EmptyComponent { index: offset }),
            b"." => Some(RepoPathError::CurrentDirectory { index: offset }),
            b".." => Some(RepoPathError::ParentTraversal { index: offset }),
            _ => None,
        };
        if let Some(error) = error {
            return Err(error);
        }
        offset += component.len() + 1;
    }
    Ok(())
}

/// An operating-system-native path kept distinct from raw Git path bytes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalPath(PathBuf);

impl LocalPath {
    /// Wraps an OS-native path without textual conversion.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// Borrows the native path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Consumes the wrapper without changing the native path representation.
    #[must_use]
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

impl AsRef<Path> for LocalPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl From<PathBuf> for LocalPath {
    fn from(value: PathBuf) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_special_and_non_utf8_repo_paths() {
        let special = RepoPath::new(b"dir with spaces/tab\tline\n:name".to_vec())
            .expect("special path should remain valid");
        assert_eq!(special.as_bytes(), b"dir with spaces/tab\tline\n:name");

        let non_utf8 = RepoPath::new(vec![b'f', b'o', 0x80]).expect("raw bytes should be valid");
        let json = serde_json::to_string(&non_utf8).expect("raw path should serialize");
        let decoded: RepoPath = serde_json::from_str(&json).expect("raw path should deserialize");
        assert_eq!(decoded.as_bytes(), &[b'f', b'o', 0x80]);
    }

    #[test]
    fn rejects_nul_absolute_and_traversing_paths() {
        assert_eq!(RepoPath::new(Vec::new()), Err(RepoPathError::Empty));
        assert_eq!(
            RepoPath::new(b"bad\0path".to_vec()),
            Err(RepoPathError::ContainsNul { index: 3 })
        );
        assert_eq!(
            RepoPath::new(b"/absolute".to_vec()),
            Err(RepoPathError::Absolute)
        );
        assert_eq!(
            RepoPath::new(b"a/../b".to_vec()),
            Err(RepoPathError::ParentTraversal { index: 2 })
        );
        assert_eq!(
            RepoPath::new(b"a/./b".to_vec()),
            Err(RepoPathError::CurrentDirectory { index: 2 })
        );
        assert_eq!(
            RepoPath::new(b"a//b".to_vec()),
            Err(RepoPathError::EmptyComponent { index: 2 })
        );
    }

    #[test]
    fn rejects_invalid_paths_during_deserialization() {
        let invalid =
            serde_json::to_string(&b"../outside".to_vec()).expect("test bytes should serialize");
        assert!(serde_json::from_str::<RepoPath>(&invalid).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn local_path_preserves_native_non_utf8_encoding() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let native = PathBuf::from(OsString::from_vec(vec![b'f', b'o', 0x80]));
        let local = LocalPath::new(native.clone());
        assert_eq!(local.into_path_buf(), native);
    }
}
