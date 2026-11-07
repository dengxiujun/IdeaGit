//! Application facade, repository sessions, queries, events, and projections.
//!
//! Repository sessions are opened through [`RepositoryRegistry`]. The registry reuses one live
//! session for the same normalized worktree location, while linked worktrees share a logical
//! repository identity through their common Git directory.

mod refresh;
mod session;

pub use refresh::{
    RefreshHealth, RefreshScheduler, RefreshSchedulerConfig, RefreshState, RefreshTrigger,
    SnapshotRefreshService,
};
pub use session::{
    RepositoryRegistry, RepositorySessionHandle, RepositorySessionView, SessionRegistryError,
};
