//! Configuration, workspace data, journals, and cache persistence.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use git_domain::{
    GitError, GitErrorKind, GitResult, JOURNAL_SCHEMA_VERSION, JournalRecord,
    LEGACY_JOURNAL_SCHEMA_VERSION, OperationId, RecoveryScope,
};
use git_ports::{JournalStore, PendingJournalRecords};
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
        if records.iter().any(|record| match record.schema_version {
            LEGACY_JOURNAL_SCHEMA_VERSION => record.recovery_scope.is_some(),
            JOURNAL_SCHEMA_VERSION => record.recovery_scope.is_none(),
            _ => true,
        }) {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "unsupported or invalid Journal schema",
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
        validate_current_record(&record)?;
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
        validate_current_record(&record)?;
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

    async fn pending(&self, scope: &RecoveryScope) -> GitResult<PendingJournalRecords> {
        let _guard = self.write_lock.lock().await;
        let mut pending = PendingJournalRecords::default();
        for record in self
            .read_all()
            .await?
            .into_iter()
            .filter(JournalRecord::needs_recovery)
        {
            if record.is_legacy() {
                pending.orphaned.push(record);
            } else if record.recovery_scope.as_ref() == Some(scope) {
                pending.scoped.push(record);
            }
        }
        Ok(pending)
    }
}

#[allow(clippy::result_large_err)]
fn validate_current_record(record: &JournalRecord) -> GitResult<()> {
    if record.schema_version != JOURNAL_SCHEMA_VERSION || record.recovery_scope.is_none() {
        return Err(GitError::new(
            GitErrorKind::Internal,
            "new Journal writes require the current schema and a recovery scope",
        ));
    }
    Ok(())
}

fn storage_error(context: &str, error: impl std::fmt::Display) -> GitError {
    GitError::new(GitErrorKind::PermissionDenied, context).with_diagnostic(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{GitCommand, RecoveryPath, RepoPath, RepositoryId, SessionTarget, WorktreeId};
    use tempfile::TempDir;

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn scope(path: &[u8]) -> RecoveryScope {
        RecoveryScope::worktree(
            RecoveryPath::unix(path.to_vec()).expect("scope path"),
            RecoveryPath::unix(path.to_vec()).expect("scope path"),
        )
    }

    fn record(target: SessionTarget, scope: RecoveryScope) -> JournalRecord {
        JournalRecord::intent(
            OperationId::new(),
            GitCommand::StageFiles {
                target,
                paths: vec![RepoPath::try_from("file.txt").expect("valid test path")],
            },
            scope,
        )
    }

    #[tokio::test]
    async fn journal_survives_store_reopen_and_filters_pending_scope() {
        let directory = TempDir::new().expect("temp directory should exist");
        let path = directory.path().join("journal.json");
        let target = target();
        let scope = scope(b"/repo/.git");
        let original = record(target, scope.clone());
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
        let pending = reopened.pending(&scope).await.expect("pending should read");
        assert_eq!(pending.scoped.len(), 1);
        assert!(pending.orphaned.is_empty());
        let mut completed = original;
        completed.phase = git_domain::JournalPhase::Completed;
        completed.result = Some(git_domain::JournalResult::Completed);
        reopened
            .replace(completed)
            .await
            .expect("replace should persist");
        let pending = reopened.pending(&scope).await.expect("pending should read");
        assert!(pending.scoped.is_empty());
        assert!(pending.orphaned.is_empty());
    }

    #[tokio::test]
    async fn duplicate_and_missing_records_are_rejected_without_overwrite() {
        let directory = TempDir::new().expect("temp directory should exist");
        let store = JsonJournalStore::new(directory.path().join("journal.json"));
        let original = record(target(), scope(b"/repo/.git"));
        store
            .append(original.clone())
            .await
            .expect("append should persist");
        assert!(store.append(original.clone()).await.is_err());
        assert!(
            store
                .replace(record(target(), scope(b"/repo/.git")))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn legacy_pending_records_are_preserved_as_orphaned() {
        let directory = TempDir::new().expect("temp directory should exist");
        let path = directory.path().join("journal.json");
        let current_scope = scope(b"/repo/.git");
        let current = record(target(), current_scope.clone());
        let mut legacy = serde_json::to_value(record(target(), scope(b"/old/.git")))
            .expect("record should serialize");
        let object = legacy.as_object_mut().expect("record should be an object");
        object.insert(
            "schema_version".to_owned(),
            serde_json::Value::from(LEGACY_JOURNAL_SCHEMA_VERSION),
        );
        object.remove("recovery_scope");
        let mut completed_record = record(target(), scope(b"/old/.git"));
        completed_record.phase = git_domain::JournalPhase::Completed;
        completed_record.result = Some(git_domain::JournalResult::Completed);
        let mut completed =
            serde_json::to_value(completed_record).expect("completed record should serialize");
        let completed_object = completed
            .as_object_mut()
            .expect("completed record should be an object");
        completed_object.insert(
            "schema_version".to_owned(),
            serde_json::Value::from(LEGACY_JOURNAL_SCHEMA_VERSION),
        );
        completed_object.remove("recovery_scope");
        tokio::fs::write(
            &path,
            serde_json::to_vec_pretty(&vec![legacy, completed]).expect("legacy JSON"),
        )
        .await
        .expect("legacy Journal should be written");

        let store = JsonJournalStore::new(&path);
        let pending = store
            .pending(&current_scope)
            .await
            .expect("legacy Journal should remain readable");
        assert!(pending.scoped.is_empty());
        assert_eq!(pending.orphaned.len(), 1);
        assert!(pending.orphaned[0].is_legacy());

        store
            .append(current)
            .await
            .expect("current record should append without deleting legacy data");
        assert_eq!(store.read_all().await.expect("Journal read").len(), 3);
    }

    #[tokio::test]
    async fn current_schema_without_scope_and_unknown_schema_are_rejected() {
        let directory = TempDir::new().expect("temp directory should exist");
        let path = directory.path().join("journal.json");
        let current_record = record(target(), scope(b"/repo/.git"));
        let mut invalid = serde_json::to_value(current_record).expect("record should serialize");
        invalid
            .as_object_mut()
            .expect("record object")
            .remove("recovery_scope");
        tokio::fs::write(
            &path,
            serde_json::to_vec(&vec![invalid]).expect("invalid JSON"),
        )
        .await
        .expect("invalid Journal should be written");
        assert!(JsonJournalStore::new(&path).read_all().await.is_err());

        let mut unknown = serde_json::to_value(record(target(), scope(b"/repo/.git")))
            .expect("record should serialize");
        unknown
            .as_object_mut()
            .expect("record object")
            .insert("schema_version".to_owned(), serde_json::Value::from(99));
        tokio::fs::write(
            &path,
            serde_json::to_vec(&vec![unknown]).expect("unknown JSON"),
        )
        .await
        .expect("unknown Journal should be written");
        assert!(JsonJournalStore::new(&path).read_all().await.is_err());
    }
}
