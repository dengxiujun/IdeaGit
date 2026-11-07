//! Repository change notifications and invalidation.
//!
//! Filesystem events are hints only. Consumers must refresh authoritative Git state and use
//! [`WatchSignal::RescanRequired`] as a request for an unconditional reconciliation.

use std::path::{Path, PathBuf};

use git_ports::RepositoryLocation;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use thiserror::Error;
use tokio::sync::mpsc;

/// A content-free hint that repository state may have changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchSignal {
    /// At least one relevant filesystem event was observed.
    Changed,
    /// Events were lost or the backend reported an error; perform a full refresh.
    RescanRequired,
}

/// Owns native filesystem watches and a bounded signal receiver.
#[derive(Debug)]
pub struct RepositoryWatch {
    _watcher: RecommendedWatcher,
    receiver: mpsc::Receiver<WatchSignal>,
}

impl RepositoryWatch {
    /// Starts watching one discovered repository location.
    ///
    /// Worktree repositories watch both the worktree and common Git directory. Bare repositories
    /// watch their common Git directory. Duplicate roots are registered only once.
    pub fn start(
        location: &RepositoryLocation,
        capacity: usize,
    ) -> Result<Self, RepositoryWatchError> {
        if capacity == 0 {
            return Err(RepositoryWatchError::ZeroCapacity);
        }
        let (sender, receiver) = mpsc::channel(capacity);
        let roots = watch_roots(location);
        let relevant_roots = roots.clone();
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            let signal = match result {
                Ok(event)
                    if event
                        .paths
                        .iter()
                        .any(|path| relevant(&relevant_roots, path)) =>
                {
                    WatchSignal::Changed
                }
                Ok(_) => return,
                Err(_) => WatchSignal::RescanRequired,
            };
            if let Err(mpsc::error::TrySendError::Full(_)) = sender.try_send(signal) {
                let _ = sender.try_send(WatchSignal::RescanRequired);
            }
        })?;

        for root in &roots {
            watcher.watch(root, RecursiveMode::Recursive)?;
        }
        Ok(Self {
            _watcher: watcher,
            receiver,
        })
    }

    /// Receives the next possible-change signal.
    pub async fn recv(&mut self) -> Option<WatchSignal> {
        self.receiver.recv().await
    }
}

/// Errors that prevent a native repository watch from starting.
#[derive(Debug, Error)]
pub enum RepositoryWatchError {
    #[error("repository watch channel capacity must be greater than zero")]
    ZeroCapacity,
    #[error("native repository watcher failed: {0}")]
    Notify(#[from] notify::Error),
}

fn watch_roots(location: &RepositoryLocation) -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(2);
    if let Some(worktree) = location.worktree_root() {
        roots.push(worktree.to_path_buf());
    }
    if !roots
        .iter()
        .any(|root| location.common_dir().starts_with(root))
    {
        roots.push(location.common_dir().to_path_buf());
    }
    roots
}

fn relevant(roots: &[PathBuf], path: &Path) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_and_linked_common_dir_are_watched_without_duplicates() {
        let ordinary = RepositoryLocation::worktree("repo", "repo/.git", "repo/.git");
        assert_eq!(watch_roots(&ordinary), vec![PathBuf::from("repo")]);

        let linked =
            RepositoryLocation::worktree("linked", "repo/.git/worktrees/linked", "repo/.git");
        assert_eq!(
            watch_roots(&linked),
            vec![PathBuf::from("linked"), PathBuf::from("repo/.git")]
        );
        assert!(relevant(
            &watch_roots(&linked),
            Path::new("linked/src/lib.rs")
        ));
        assert!(relevant(
            &watch_roots(&linked),
            Path::new("repo/.git/refs/heads/main")
        ));
        assert!(!relevant(&watch_roots(&linked), Path::new("other/file")));
    }

    #[test]
    fn bare_repository_watches_only_its_common_dir() {
        let bare = RepositoryLocation::bare("repo.git", "repo.git");
        assert_eq!(watch_roots(&bare), vec![PathBuf::from("repo.git")]);
    }

    #[test]
    fn zero_capacity_is_rejected_before_touching_the_filesystem() {
        let location = RepositoryLocation::bare("missing.git", "missing.git");
        assert!(matches!(
            RepositoryWatch::start(&location, 0),
            Err(RepositoryWatchError::ZeroCapacity)
        ));
    }
}
