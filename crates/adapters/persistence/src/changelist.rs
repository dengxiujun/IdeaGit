use std::path::{Path, PathBuf};

use async_trait::async_trait;
use git_changelist::{ChangelistBundle, ChangelistPersistenceError, ChangelistStore};
use git_domain::SessionTarget;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct JsonChangelistStore {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl JsonChangelistStore {
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
    ) -> Result<ChangelistBundle, ChangelistPersistenceError> {
        let bytes = match tokio::fs::read(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ChangelistBundle::empty(target));
            }
            Err(error) => return Err(ChangelistPersistenceError::Io(error.to_string())),
        };
        let bundle: ChangelistBundle = serde_json::from_slice(&bytes)
            .map_err(|error| ChangelistPersistenceError::Encoding(error.to_string()))?;
        if bundle.target != target {
            return Err(ChangelistPersistenceError::InvalidBundle(
                "bundle target mismatch".into(),
            ));
        }
        bundle
            .validate()
            .map_err(|error| ChangelistPersistenceError::InvalidBundle(error.to_string()))?;
        Ok(bundle)
    }
}

#[async_trait]
impl ChangelistStore for JsonChangelistStore {
    async fn load(
        &self,
        target: SessionTarget,
    ) -> Result<ChangelistBundle, ChangelistPersistenceError> {
        self.read_bundle(target).await
    }

    async fn save(
        &self,
        expected_revision: u64,
        bundle: ChangelistBundle,
    ) -> Result<(), ChangelistPersistenceError> {
        bundle
            .validate()
            .map_err(|error| ChangelistPersistenceError::InvalidBundle(error.to_string()))?;
        let _guard = self.write_lock.lock().await;
        let current = self.read_bundle(bundle.target).await?;
        if current.revision != expected_revision
            || bundle.revision != expected_revision.saturating_add(1)
        {
            return Err(ChangelistPersistenceError::RevisionConflict);
        }
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| ChangelistPersistenceError::Io(error.to_string()))?;
        if tokio::fs::try_exists(&self.path)
            .await
            .map_err(|error| ChangelistPersistenceError::Io(error.to_string()))?
        {
            tokio::fs::copy(&self.path, self.path.with_extension("json.bak"))
                .await
                .map_err(|error| ChangelistPersistenceError::Io(error.to_string()))?;
        }
        let bytes = serde_json::to_vec_pretty(&bundle)
            .map_err(|error| ChangelistPersistenceError::Encoding(error.to_string()))?;
        let mut temporary = self.path.as_os_str().to_os_string();
        temporary.push(format!(".{}.tmp", std::process::id()));
        let temporary = PathBuf::from(temporary);
        tokio::fs::write(&temporary, bytes)
            .await
            .map_err(|error| ChangelistPersistenceError::Io(error.to_string()))?;
        if let Err(error) = tokio::fs::rename(&temporary, &self.path).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(ChangelistPersistenceError::Io(error.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_changelist::Changelist;
    use git_domain::{RepositoryId, WorktreeId};
    use tempfile::TempDir;

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    #[tokio::test]
    async fn round_trips_empty_and_non_empty_bundles_with_backup() {
        let directory = TempDir::new().expect("temp directory");
        let path = directory.path().join("changelists.json");
        let store = JsonChangelistStore::new(&path);
        let target = target();
        assert_eq!(
            store.load(target).await.expect("load should work"),
            ChangelistBundle::empty(target)
        );
        let mut bundle = ChangelistBundle::empty(target);
        bundle.revision = 1;
        bundle
            .lists
            .push(Changelist::new(target, "Default", true, 1).expect("list should be valid"));
        store
            .save(0, bundle.clone())
            .await
            .expect("save should work");
        let mut next = bundle.clone();
        next.revision = 2;
        store
            .save(1, next.clone())
            .await
            .expect("second save should work");
        assert_eq!(store.load(target).await.expect("load should work"), next);
        assert!(path.with_extension("json.bak").exists());
    }

    #[tokio::test]
    async fn rejects_revision_conflicts_without_overwriting_current_data() {
        let directory = TempDir::new().expect("temp directory");
        let path = directory.path().join("changelists.json");
        let store = JsonChangelistStore::new(&path);
        let target = target();
        let mut bundle = ChangelistBundle::empty(target);
        bundle.revision = 1;
        store
            .save(0, bundle.clone())
            .await
            .expect("save should work");
        let mut conflicting = bundle.clone();
        conflicting.revision = 2;
        assert_eq!(
            store.save(0, conflicting).await,
            Err(ChangelistPersistenceError::RevisionConflict)
        );
        assert_eq!(store.load(target).await.expect("load should work"), bundle);
    }
}
