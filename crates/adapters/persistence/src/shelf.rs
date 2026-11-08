use std::path::{Path, PathBuf};

use async_trait::async_trait;
use git_domain::SessionTarget;
use git_shelf::{ShelfBundle, ShelfPersistenceError, ShelfStore};
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct JsonShelfStore {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl JsonShelfStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Mutex::new(()),
        }
    }

    async fn read_bundle(
        &self,
        target: SessionTarget,
    ) -> Result<ShelfBundle, ShelfPersistenceError> {
        let bytes = match tokio::fs::read(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ShelfBundle::empty(target));
            }
            Err(error) => return Err(ShelfPersistenceError::Io(error.to_string())),
        };
        let bundle: ShelfBundle = serde_json::from_slice(&bytes)
            .map_err(|error| ShelfPersistenceError::Encoding(error.to_string()))?;
        if bundle.target != target {
            return Err(ShelfPersistenceError::InvalidBundle(
                "bundle target mismatch".into(),
            ));
        }
        bundle
            .validate()
            .map_err(|error| ShelfPersistenceError::InvalidBundle(error.to_string()))?;
        Ok(bundle)
    }
}

#[async_trait]
impl ShelfStore for JsonShelfStore {
    async fn load(&self, target: SessionTarget) -> Result<ShelfBundle, ShelfPersistenceError> {
        self.read_bundle(target).await
    }

    async fn save(
        &self,
        expected_revision: u64,
        bundle: ShelfBundle,
    ) -> Result<(), ShelfPersistenceError> {
        bundle
            .validate()
            .map_err(|error| ShelfPersistenceError::InvalidBundle(error.to_string()))?;
        let _guard = self.write_lock.lock().await;
        let current = self.read_bundle(bundle.target).await?;
        if current.revision != expected_revision
            || bundle.revision != expected_revision.saturating_add(1)
        {
            return Err(ShelfPersistenceError::RevisionConflict);
        }
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| ShelfPersistenceError::Io(error.to_string()))?;
        if tokio::fs::try_exists(&self.path)
            .await
            .map_err(|error| ShelfPersistenceError::Io(error.to_string()))?
        {
            tokio::fs::copy(&self.path, self.path.with_extension("json.bak"))
                .await
                .map_err(|error| ShelfPersistenceError::Io(error.to_string()))?;
        }
        let bytes = serde_json::to_vec_pretty(&bundle)
            .map_err(|error| ShelfPersistenceError::Encoding(error.to_string()))?;
        let mut temporary = self.path.as_os_str().to_os_string();
        temporary.push(format!(".{}.tmp", std::process::id()));
        let temporary = PathBuf::from(temporary);
        tokio::fs::write(&temporary, bytes)
            .await
            .map_err(|error| ShelfPersistenceError::Io(error.to_string()))?;
        if let Err(error) = tokio::fs::rename(&temporary, &self.path).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(ShelfPersistenceError::Io(error.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{RepoPath, RepositoryId, SessionTarget, WorktreeId};
    use git_shelf::{PatchBundle, ShelfEntry, ShelvedFile};
    use tempfile::TempDir;

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn entry(target: SessionTarget) -> ShelfEntry {
        let file = ShelvedFile::new(
            RepoPath::try_from("src/lib.rs").expect("path"),
            b"diff".to_vec(),
            vec![1],
        )
        .expect("file");
        ShelfEntry::new(
            target,
            "before refactor",
            42,
            None,
            PatchBundle::new(vec![file]).expect("bundle"),
        )
        .expect("entry")
    }

    #[tokio::test]
    async fn persists_bundle_with_backup_and_reopens() {
        let directory = TempDir::new().expect("directory");
        let path = directory.path().join("shelf.json");
        let store = JsonShelfStore::new(&path);
        let target = target();
        let mut bundle = ShelfBundle::empty(target);
        bundle.revision = 1;
        bundle.entries.push(entry(target));
        store.save(0, bundle.clone()).await.expect("save");
        let mut next = bundle.clone();
        next.revision = 2;
        store.save(1, next.clone()).await.expect("second save");
        assert_eq!(store.load(target).await.expect("load"), next);
        assert!(path.with_extension("json.bak").exists());
    }

    #[tokio::test]
    async fn revision_conflict_preserves_existing_shelf() {
        let directory = TempDir::new().expect("directory");
        let path = directory.path().join("shelf.json");
        let store = JsonShelfStore::new(&path);
        let target = target();
        let mut bundle = ShelfBundle::empty(target);
        bundle.revision = 1;
        bundle.entries.push(entry(target));
        store.save(0, bundle.clone()).await.expect("save");
        assert_eq!(
            store.save(0, bundle.clone()).await,
            Err(ShelfPersistenceError::RevisionConflict)
        );
        assert_eq!(store.load(target).await.expect("load"), bundle);
    }
}
