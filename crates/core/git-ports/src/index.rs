//! Safe Index mutation port for Stage and Unstage.

use async_trait::async_trait;
use git_domain::{GitResult, RepoPath, SessionTarget};

use crate::ExecutionContext;

/// Typed Index mutation requested by the application layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexMutation {
    Stage {
        target: SessionTarget,
        paths: Vec<RepoPath>,
    },
    Unstage {
        target: SessionTarget,
        paths: Vec<RepoPath>,
    },
}

impl IndexMutation {
    #[must_use]
    pub const fn target(&self) -> SessionTarget {
        match self {
            Self::Stage { target, .. } | Self::Unstage { target, .. } => *target,
        }
    }

    /// Rejects an empty path list before any Git process is spawned.
    pub fn validate(&self) -> Result<(), IndexMutationError> {
        let paths = match self {
            Self::Stage { paths, .. } | Self::Unstage { paths, .. } => paths,
        };
        if paths.is_empty() {
            Err(IndexMutationError::EmptyPaths)
        } else {
            Ok(())
        }
    }
}

/// Index mutation validation failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IndexMutationError {
    #[error("stage or unstage requires at least one path")]
    EmptyPaths,
}

/// Adapter boundary for literal-path Stage/Unstage operations.
#[async_trait]
pub trait IndexWriter: Send + Sync {
    async fn apply(&self, mutation: IndexMutation, context: ExecutionContext) -> GitResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{RepoPath, RepositoryId, WorktreeId};

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    #[test]
    fn stage_and_unstage_keep_literal_paths_typed() {
        let path = RepoPath::try_from("dir/[name].txt").expect("valid repo path");
        let stage = IndexMutation::Stage {
            target: target(),
            paths: vec![path.clone()],
        };
        let unstage = IndexMutation::Unstage {
            target: stage.target(),
            paths: vec![path],
        };
        assert_eq!(stage.validate(), Ok(()));
        assert_eq!(unstage.validate(), Ok(()));
    }

    #[test]
    fn empty_paths_are_rejected_before_execution() {
        let mutation = IndexMutation::Stage {
            target: target(),
            paths: Vec::new(),
        };
        assert_eq!(mutation.validate(), Err(IndexMutationError::EmptyPaths));
    }
}
