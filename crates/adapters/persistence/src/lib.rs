//! Configuration, workspace data, journals, and cache persistence.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use git_domain::{
    GitError, GitErrorKind, GitResult, JOURNAL_SCHEMA_VERSION, JournalRecord, OperationId,
    SessionTarget,
};
use git_ports::JournalStore;
use tokio::sync::Mutex;

/// Atomic JSON-file Journal store.
#[derive(Clone, Debug)]
pub struct JsonJournalStore {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl JsonJournalStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    #[allow(clippy::result_large_err)]
    async fn read_all(&self) -> GitResult<Vec<JournalRecord>> {
        let bytes = match tokio::fs::read(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(storage_error("read Journal", error)),
        };
        let records: Vec<JournalRecord> = serde_json::from_slice(&bytes)
            .map_err(|error| storage_error("decode Journal", error))?;
        if records
            .iter()
            .any(|record| record.schema_version != JOURNAL_SCHEMA_VERSION)
        {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "unsupported Journal schema",
            ));
        }
        Ok(records)
    }

    #[allow(clippy::result_large_err)]
    async fn write_all(&self, records: &[JournalRecord]) -> GitResult<()> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| storage_error("create Journal directory", error))?;
        let bytes = serde_json::to_vec_pretty(records)
            .map_err(|error| storage_error("encode Journal", error))?;
        let mut temporary = self.path.as_os_str().to_os_string();
        temporary.push(format!(".{}.tmp", std::process::id()));
        let temporary = PathBuf::from(temporary);
        tokio::fs::write(&temporary, bytes)
            .await
            .map_err(|error| storage_error("write Journal temporary file", error))?;
        if let Err(error) = tokio::fs::rename(&temporary, &self.path).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(storage_error("replace Journal", error));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl JournalStore for JsonJournalStore {
    async fn append(&self, record: JournalRecord) -> GitResult<()> {
        let _guard = self.write_lock.lock().await;
        let mut records = self.read_all().await?;
        if records
            .iter()
            .any(|existing| existing.operation_id == record.operation_id)
        {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "Journal operation already exists",
            ));
        }
        records.push(record);
        self.write_all(&records).await
    }

    async fn replace(&self, record: JournalRecord) -> GitResult<()> {
        let _guard = self.write_lock.lock().await;
        let mut records = self.read_all().await?;
        let Some(existing) = records
            .iter_mut()
            .find(|existing| existing.operation_id == record.operation_id)
        else {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "Journal operation not found",
            ));
        };
        *existing = record;
        self.write_all(&records).await
    }

    async fn get(&self, operation_id: OperationId) -> GitResult<Option<JournalRecord>> {
        let _guard = self.write_lock.lock().await;
        Ok(self
            .read_all()
            .await?
            .into_iter()
            .find(|record| record.operation_id == operation_id))
    }

    async fn pending(&self, target: SessionTarget) -> GitResult<Vec<JournalRecord>> {
        let _guard = self.write_lock.lock().await;
        Ok(self
            .read_all()
            .await?
            .into_iter()
            .filter(|record| record.target == target && record.needs_recovery())
            .collect())
    }
}

fn storage_error(context: &str, error: impl std::fmt::Display) -> GitError {
    GitError::new(GitErrorKind::PermissionDenied, context).with_diagnostic(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{GitCommand, RepoPath, RepositoryId, WorktreeId};
    use tempfile::TempDir;

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn record(target: SessionTarget) -> JournalRecord {
        JournalRecord::intent(
            OperationId::new(),
            GitCommand::StageFiles {
                target,
                paths: vec![RepoPath::try_from("file.txt").expect("valid test path")],
            },
        )
    }

    #[tokio::test]
    async fn journal_survives_store_reopen_and_filters_pending_target() {
        let directory = TempDir::new().expect("temp directory should exist");
        let path = directory.path().join("journal.json");
        let target = target();
        let original = record(target);
        let id = original.operation_id;
        let store = JsonJournalStore::new(&path);
        store
            .append(original.clone())
            .await
            .expect("append should persist");
        let reopened = JsonJournalStore::new(&path);
        assert_eq!(
            reopened.get(id).await.expect("read should work"),
            Some(original.clone())
        );
        assert_eq!(
            reopened
                .pending(target)
                .await
                .expect("pending should read")
                .len(),
            1
        );
        let mut completed = original;
        completed.phase = git_domain::JournalPhase::Completed;
        completed.result = Some(git_domain::JournalResult::Completed);
        reopened
            .replace(completed)
            .await
            .expect("replace should persist");
        assert!(
            reopened
                .pending(target)
                .await
                .expect("pending should read")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn duplicate_and_missing_records_are_rejected_without_overwrite() {
        let directory = TempDir::new().expect("temp directory should exist");
        let store = JsonJournalStore::new(directory.path().join("journal.json"));
        let original = record(target());
        store
            .append(original.clone())
            .await
            .expect("append should persist");
        assert!(store.append(original.clone()).await.is_err());
        assert!(store.replace(record(target())).await.is_err());
    }
}
