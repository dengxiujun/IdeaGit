use std::path::PathBuf;

use async_trait::async_trait;
use git_domain::{GitResult, ObjectId, OperationId};

use crate::{ExecutionContext, RepositoryLocation};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporaryIndexLease {
    pub path: PathBuf,
    pub transaction_id: OperationId,
}

impl Drop for TemporaryIndexLease {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporaryIndexContext {
    pub operation_id: OperationId,
    pub path: PathBuf,
    pub expected_head: Option<ObjectId>,
    pub original_index_digest: [u8; 32],
    pub original_index_backup: Option<PathBuf>,
    pub candidate_tree: Option<ObjectId>,
}

#[async_trait]
pub trait TemporaryIndexManager: Send + Sync {
    async fn create(
        &self,
        repository: &RepositoryLocation,
        operation: OperationId,
        context: ExecutionContext,
    ) -> GitResult<TemporaryIndexLease>;
}
