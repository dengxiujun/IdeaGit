//! In-process resource locks for one operation coordinator.
//!
//! These locks serialize application operations. They are not Git's filesystem locks and must
//! never be held while manually creating or reserving `.git/index.lock`.

use std::{cmp::Ordering, collections::HashMap, sync::Arc};

use git_domain::{RepositoryId, WorktreeId};
use thiserror::Error;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

/// Scope of a resource lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceScope {
    CommonGitDir(RepositoryId),
    Worktree(WorktreeId),
}

/// Mutable Git resource protected by the coordinator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Refs,
    Index,
    WorktreeFiles,
    Sequencer,
}

/// Stable key for one application resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceKey {
    pub scope: ResourceScope,
    pub kind: ResourceKind,
}

impl Ord for ResourceKey {
    fn cmp(&self, other: &Self) -> Ordering {
        scope_order(self.scope)
            .cmp(&scope_order(other.scope))
            .then_with(|| kind_order(self.kind).cmp(&kind_order(other.kind)))
    }
}

impl PartialOrd for ResourceKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn scope_order(scope: ResourceScope) -> (u8, [u8; 16]) {
    match scope {
        ResourceScope::CommonGitDir(id) => (0, *id.as_uuid().as_bytes()),
        ResourceScope::Worktree(id) => (1, *id.as_uuid().as_bytes()),
    }
}

const fn kind_order(kind: ResourceKind) -> u8 {
    match kind {
        ResourceKind::Refs => 0,
        ResourceKind::Index => 1,
        ResourceKind::WorktreeFiles => 2,
        ResourceKind::Sequencer => 3,
    }
}

/// RAII collection of locks acquired in stable order.
#[derive(Debug)]
pub struct ResourceLock {
    keys: Vec<ResourceKey>,
    _guards: Vec<OwnedMutexGuard<()>>,
}

impl ResourceLock {
    /// Returns the unique keys held by this guard collection.
    #[must_use]
    pub fn keys(&self) -> &[ResourceKey] {
        &self.keys
    }
}

/// Bounded-lifetime in-process resource lock manager.
#[derive(Clone, Debug, Default)]
pub struct ResourceLockManager {
    resources: Arc<std::sync::Mutex<HashMap<ResourceKey, Arc<Mutex<()>>>>>,
}

impl ResourceLockManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquires all keys in a deterministic order, avoiding multi-resource deadlocks.
    pub async fn acquire<I>(&self, keys: I) -> Result<ResourceLock, LockError>
    where
        I: IntoIterator<Item = ResourceKey>,
    {
        self.acquire_with_cancellation(keys, CancellationToken::new())
            .await
    }

    /// Acquires all keys in deterministic order or returns when cancellation is observed.
    pub async fn acquire_with_cancellation<I>(
        &self,
        keys: I,
        cancellation: CancellationToken,
    ) -> Result<ResourceLock, LockError>
    where
        I: IntoIterator<Item = ResourceKey>,
    {
        let mut keys: Vec<_> = keys.into_iter().collect();
        keys.sort_unstable();
        keys.dedup();
        let mut guards = Vec::with_capacity(keys.len());
        for key in &keys {
            let resource = self.resource(*key)?;
            let guard = tokio::select! {
                () = cancellation.cancelled() => return Err(LockError::Cancelled),
                guard = resource.lock_owned() => guard,
            };
            guards.push(guard);
        }
        Ok(ResourceLock {
            keys,
            _guards: guards,
        })
    }

    fn resource(&self, key: ResourceKey) -> Result<Arc<Mutex<()>>, LockError> {
        let mut resources = self
            .resources
            .lock()
            .map_err(|_| LockError::StateUnavailable)?;
        Ok(Arc::clone(
            resources
                .entry(key)
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        ))
    }
}

/// Lock acquisition failures.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum LockError {
    #[error("resource lock manager state is unavailable")]
    StateUnavailable,
    #[error("resource lock acquisition cancelled")]
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{RepositoryId, WorktreeId};
    use tokio::time::{Duration, timeout};

    fn refs() -> ResourceKey {
        ResourceKey {
            scope: ResourceScope::CommonGitDir(RepositoryId::new()),
            kind: ResourceKind::Refs,
        }
    }

    fn index() -> ResourceKey {
        ResourceKey {
            scope: ResourceScope::Worktree(WorktreeId::new()),
            kind: ResourceKind::Index,
        }
    }

    #[tokio::test]
    async fn same_resource_is_serialized_and_independent_resources_progress() {
        let manager = ResourceLockManager::new();
        let key = refs();
        let held = manager
            .acquire([key])
            .await
            .expect("first lock should work");
        let waiting = manager.clone();
        let blocked = tokio::spawn(async move { waiting.acquire([key]).await });
        assert!(timeout(Duration::from_millis(20), blocked).await.is_err());
        drop(held);
        assert!(manager.acquire([key]).await.is_ok());
        assert!(manager.acquire([index()]).await.is_ok());
    }

    #[tokio::test]
    async fn cancellation_interrupts_waiting_for_a_held_resource() {
        let manager = ResourceLockManager::new();
        let key = refs();
        let held = manager
            .acquire([key])
            .await
            .expect("first lock should work");
        let cancellation = CancellationToken::new();
        let waiting = manager.clone();
        let token = cancellation.clone();
        let task =
            tokio::spawn(async move { waiting.acquire_with_cancellation([key], token).await });
        cancellation.cancel();
        assert!(matches!(
            task.await.expect("lock task should finish"),
            Err(LockError::Cancelled)
        ));
        drop(held);
    }

    #[test]
    fn resource_keys_have_a_total_order_across_scopes_and_kinds() {
        let repository = RepositoryId::new();
        let worktree = WorktreeId::new();
        let mut keys = [
            ResourceKey {
                scope: ResourceScope::Worktree(worktree),
                kind: ResourceKind::Sequencer,
            },
            ResourceKey {
                scope: ResourceScope::CommonGitDir(repository),
                kind: ResourceKind::Refs,
            },
            ResourceKey {
                scope: ResourceScope::Worktree(worktree),
                kind: ResourceKind::Index,
            },
        ];
        keys.sort_unstable();
        assert!(matches!(keys[0].scope, ResourceScope::CommonGitDir(_)));
        assert_eq!(keys[1].kind, ResourceKind::Index);
        assert_eq!(keys[2].kind, ResourceKind::Sequencer);
    }
}
