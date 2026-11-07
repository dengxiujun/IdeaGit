//! Repository session ownership and registry lifecycle.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

use git_domain::{RepositoryId, SessionTarget, WorktreeId};
use git_ports::RepositoryLocation;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// A read-only view of one live repository session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositorySessionView {
    target: SessionTarget,
    location: RepositoryLocation,
    open_handles: usize,
}

impl RepositorySessionView {
    /// Returns the generation-qualified session target.
    #[must_use]
    pub const fn target(&self) -> SessionTarget {
        self.target
    }

    /// Returns the normalized location used to identify the session.
    #[must_use]
    pub const fn location(&self) -> &RepositoryLocation {
        &self.location
    }

    /// Returns the number of current openers sharing the session.
    #[must_use]
    pub const fn open_handles(&self) -> usize {
        self.open_handles
    }
}

/// One caller's ownership of a live repository session.
///
/// Opening the same normalized location again creates another handle to the same target. Dropping
/// or explicitly closing a handle releases only that caller. The final release closes the session
/// and cancels its background-task token.
#[derive(Debug)]
pub struct RepositorySessionHandle {
    core: Arc<RegistryCore>,
    key: SessionKey,
    target: SessionTarget,
    released: bool,
}

impl RepositorySessionHandle {
    /// Returns the generation-qualified target owned by this handle.
    #[must_use]
    pub const fn target(&self) -> SessionTarget {
        self.target
    }

    fn release(&mut self) -> Result<(), SessionRegistryError> {
        if self.released {
            return Ok(());
        }
        self.core.release(&self.key, self.target)?;
        self.released = true;
        Ok(())
    }
}

impl Drop for RepositorySessionHandle {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

/// Thread-safe registry for live repository sessions.
///
/// `RepositoryLocation` values must come from repository discovery, which canonicalizes the paths
/// reported by Git. A common Git directory owns one [`RepositoryId`]; a concrete worktree (or bare
/// repository) owns one live session generation at a time.
#[derive(Clone, Debug, Default)]
pub struct RepositoryRegistry {
    core: Arc<RegistryCore>,
}

impl RepositoryRegistry {
    /// Creates an empty session registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens or reuses a session for a discovered repository location.
    pub fn open(
        &self,
        location: RepositoryLocation,
    ) -> Result<RepositorySessionHandle, SessionRegistryError> {
        let key = SessionKey::from_location(&location);
        let target = {
            let mut state = self.core.lock()?;
            if let Some(session) = state.sessions.get_mut(&key) {
                session.open_handles = session
                    .open_handles
                    .checked_add(1)
                    .ok_or(SessionRegistryError::HandleCountExhausted)?;
                session.target
            } else {
                let repository = state.repository_id(location.common_dir());
                let generation = state.next_generation()?;
                let target = match location.worktree_root() {
                    Some(_) => SessionTarget::worktree(repository, WorktreeId::new(), generation),
                    None => SessionTarget::bare(repository, generation),
                };
                state.sessions.insert(
                    key.clone(),
                    SessionEntry {
                        target,
                        location: location.clone(),
                        open_handles: 1,
                        cancellation: CancellationToken::new(),
                    },
                );
                target
            }
        };

        Ok(RepositorySessionHandle {
            core: Arc::clone(&self.core),
            key,
            target,
            released: false,
        })
    }

    /// Explicitly releases one caller's handle.
    pub fn close(&self, mut handle: RepositorySessionHandle) -> Result<(), SessionRegistryError> {
        if !Arc::ptr_eq(&self.core, &handle.core) {
            return Err(SessionRegistryError::ForeignHandle);
        }
        handle.release()
    }

    /// Returns a current session view, rejecting closed or superseded generations.
    pub fn session(
        &self,
        target: SessionTarget,
    ) -> Result<RepositorySessionView, SessionRegistryError> {
        let state = self.core.lock()?;
        let session = state
            .sessions
            .values()
            .find(|session| session.target == target)
            .ok_or(SessionRegistryError::SessionNotActive(target))?;
        Ok(session.view())
    }

    /// Returns a cancellation token scoped to the current session generation.
    ///
    /// CORE-008 may derive child tokens for watcher and refresh tasks. The token is cancelled when
    /// the final session handle is released.
    pub fn cancellation_token(
        &self,
        target: SessionTarget,
    ) -> Result<CancellationToken, SessionRegistryError> {
        let state = self.core.lock()?;
        state
            .sessions
            .values()
            .find(|session| session.target == target)
            .map(|session| session.cancellation.clone())
            .ok_or(SessionRegistryError::SessionNotActive(target))
    }
}

/// Errors produced by repository session lifecycle operations.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SessionRegistryError {
    /// The registry mutex was poisoned by a panicking holder.
    #[error("repository registry state is unavailable")]
    StateUnavailable,
    /// A handle was passed to a registry that did not create it.
    #[error("repository session handle belongs to another registry")]
    ForeignHandle,
    /// The target was closed or belongs to an older session generation.
    #[error("repository session target is not active: {0:?}")]
    SessionNotActive(SessionTarget),
    /// The session generation counter cannot advance without reuse.
    #[error("repository session generation counter is exhausted")]
    GenerationExhausted,
    /// The number of shared openers cannot be represented.
    #[error("repository session handle count is exhausted")]
    HandleCountExhausted,
}

#[derive(Debug, Default)]
struct RegistryCore {
    state: Mutex<RegistryState>,
}

impl RegistryCore {
    fn lock(&self) -> Result<MutexGuard<'_, RegistryState>, SessionRegistryError> {
        self.state
            .lock()
            .map_err(|_| SessionRegistryError::StateUnavailable)
    }

    fn release(&self, key: &SessionKey, target: SessionTarget) -> Result<(), SessionRegistryError> {
        let mut state = self.lock()?;
        let session = state
            .sessions
            .get_mut(key)
            .filter(|session| session.target == target)
            .ok_or(SessionRegistryError::SessionNotActive(target))?;
        session.open_handles -= 1;
        if session.open_handles == 0 {
            session.cancellation.cancel();
            state.sessions.remove(key);
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct RegistryState {
    repository_ids: HashMap<PathBuf, RepositoryId>,
    sessions: HashMap<SessionKey, SessionEntry>,
    last_generation: u64,
}

impl RegistryState {
    fn repository_id(&mut self, common_dir: &std::path::Path) -> RepositoryId {
        *self
            .repository_ids
            .entry(common_dir.to_path_buf())
            .or_default()
    }

    fn next_generation(&mut self) -> Result<u64, SessionRegistryError> {
        self.last_generation = self
            .last_generation
            .checked_add(1)
            .ok_or(SessionRegistryError::GenerationExhausted)?;
        Ok(self.last_generation)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SessionKey {
    common_dir: PathBuf,
    worktree_root: Option<PathBuf>,
}

impl SessionKey {
    fn from_location(location: &RepositoryLocation) -> Self {
        Self {
            common_dir: location.common_dir().to_path_buf(),
            worktree_root: location.worktree_root().map(PathBuf::from),
        }
    }
}

#[derive(Debug)]
struct SessionEntry {
    target: SessionTarget,
    location: RepositoryLocation,
    open_handles: usize,
    cancellation: CancellationToken,
}

impl SessionEntry {
    fn view(&self) -> RepositorySessionView {
        RepositorySessionView {
            target: self.target,
            location: self.location.clone(),
            open_handles: self.open_handles,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn worktree(root: &str, git_dir: &str, common_dir: &str) -> RepositoryLocation {
        RepositoryLocation::worktree(root, git_dir, common_dir)
    }

    #[test]
    fn repeated_open_reuses_session_until_last_handle_closes() {
        let registry = RepositoryRegistry::new();
        let first = registry
            .open(worktree("repo", "repo/.git", "repo/.git"))
            .expect("first session should open");
        let second = registry
            .open(worktree("repo", "repo/.git", "repo/.git"))
            .expect("same session should reopen");
        let target = first.target();

        assert_eq!(second.target(), target);
        assert_eq!(
            registry
                .session(target)
                .expect("session should exist")
                .open_handles(),
            2
        );

        registry.close(first).expect("first handle should close");
        assert_eq!(
            registry
                .session(target)
                .expect("session should remain")
                .open_handles(),
            1
        );
        registry.close(second).expect("last handle should close");
        assert_eq!(
            registry.session(target),
            Err(SessionRegistryError::SessionNotActive(target))
        );
    }

    #[test]
    fn linked_worktrees_share_repository_but_not_session() {
        let registry = RepositoryRegistry::new();
        let primary = registry
            .open(worktree("repo", "repo/.git", "repo/.git"))
            .expect("primary worktree should open");
        let linked = registry
            .open(worktree(
                "linked",
                "repo/.git/worktrees/linked",
                "repo/.git",
            ))
            .expect("linked worktree should open");

        assert_eq!(primary.target().repository(), linked.target().repository());
        assert_ne!(
            primary.target().worktree_id(),
            linked.target().worktree_id()
        );
        assert_ne!(primary.target(), linked.target());
    }

    #[test]
    fn repositories_remain_isolated_and_bare_has_no_worktree() {
        let registry = RepositoryRegistry::new();
        let first = registry
            .open(worktree("one", "one/.git", "one/.git"))
            .expect("first repository should open");
        let second = registry
            .open(worktree("two", "two/.git", "two/.git"))
            .expect("second repository should open");
        let bare = registry
            .open(RepositoryLocation::bare("bare.git", "bare.git"))
            .expect("bare repository should open");

        assert_ne!(first.target().repository(), second.target().repository());
        assert!(bare.target().is_bare());
        assert_eq!(
            registry
                .session(bare.target())
                .expect("bare session should exist")
                .location()
                .worktree_root(),
            None
        );
    }

    #[test]
    fn reopening_after_close_uses_a_new_generation_and_cancels_old_tasks() {
        let registry = RepositoryRegistry::new();
        let handle = registry
            .open(worktree("repo", "repo/.git", "repo/.git"))
            .expect("session should open");
        let old_target = handle.target();
        let cancellation = registry
            .cancellation_token(old_target)
            .expect("token should exist");
        registry.close(handle).expect("session should close");

        assert!(cancellation.is_cancelled());
        let reopened = registry
            .open(worktree("repo", "repo/.git", "repo/.git"))
            .expect("session should reopen");
        assert_eq!(reopened.target().repository(), old_target.repository());
        assert!(reopened.target().session_generation() > old_target.session_generation());
        assert_eq!(
            registry.session(old_target),
            Err(SessionRegistryError::SessionNotActive(old_target))
        );
    }

    #[test]
    fn dropping_a_handle_releases_it_and_foreign_registry_rejects_it() {
        let registry = RepositoryRegistry::new();
        let handle = registry
            .open(worktree("repo", "repo/.git", "repo/.git"))
            .expect("session should open");
        let target = handle.target();
        let foreign = RepositoryRegistry::new();
        assert_eq!(
            foreign.close(handle),
            Err(SessionRegistryError::ForeignHandle)
        );
        assert_eq!(
            registry.session(target),
            Err(SessionRegistryError::SessionNotActive(target))
        );
    }

    #[test]
    fn session_view_preserves_discovered_paths() {
        let registry = RepositoryRegistry::new();
        let handle = registry
            .open(worktree("repo", "repo/git-dir", "repo/common"))
            .expect("session should open");
        let view = registry
            .session(handle.target())
            .expect("session should be visible");

        assert_eq!(view.location().worktree_root(), Some(Path::new("repo")));
        assert_eq!(view.location().git_dir(), Path::new("repo/git-dir"));
        assert_eq!(view.location().common_dir(), Path::new("repo/common"));
    }
}
