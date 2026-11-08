//! Application facade, repository sessions, queries, events, and projections.
//!
//! Repository sessions are opened through [`RepositoryRegistry`]. The registry reuses one live
//! session for the same normalized worktree location, while linked worktrees share a logical
//! repository identity through their common Git directory.

mod candidate_preview;
mod refresh;
mod selection_preview;
mod session;

pub use candidate_preview::{CandidateTreePreview, CandidateTreePreviewService};
pub use refresh::{
    RefreshHealth, RefreshScheduler, RefreshSchedulerConfig, RefreshState, RefreshTrigger,
    ScheduledSnapshotRebuilder, SnapshotRefreshService,
};
pub use selection_preview::{SelectionPatchPreview, SelectionPatchPreviewService};
pub use session::{
    RepositoryRegistry, RepositorySessionHandle, RepositorySessionView, SessionRegistryError,
};
