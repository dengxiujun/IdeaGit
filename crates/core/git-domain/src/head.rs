//! HEAD and upstream branch state.

use serde::{Deserialize, Serialize};

use crate::{ObjectId, RefName};

/// Current repository HEAD state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeadState {
    /// `HEAD` names a branch that does not have a first commit yet.
    Unborn { branch: RefName },
    /// `HEAD` names a branch and resolves to the branch tip.
    Attached { branch: RefName, oid: ObjectId },
    /// `HEAD` points directly at a commit rather than a local branch.
    Detached { oid: ObjectId },
}

/// Upstream relation for the currently attached local branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchTracking {
    /// Fully qualified remote-tracking or local upstream ref.
    pub upstream: RefName,
    /// Commits reachable from `HEAD` but not from the upstream.
    pub ahead: u64,
    /// Commits reachable from the upstream but not from `HEAD`.
    pub behind: u64,
}
