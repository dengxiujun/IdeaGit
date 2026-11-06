//! Structured domain errors shared by application, adapters, and UI projections.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{OperationId, SessionTarget};

/// Result type used by Git domain and port contracts.
pub type GitResult<T> = Result<T, GitError>;

/// Stable classification for Git failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitErrorKind {
    RepositoryNotFound,
    InvalidRepository,
    GitUnavailable,
    UnsupportedGitVersion,
    DirtyWorktree,
    IndexLocked,
    ReferenceChanged,
    InvalidPath,
    ObjectNotFound,
    AuthenticationRequired,
    AuthenticationFailed,
    NetworkFailure,
    MergeConflict,
    OperationAlreadyInProgress,
    HookRejected,
    SigningFailed,
    PermissionDenied,
    TimedOut,
    Cancelled,
    UnsupportedFeature,
    OutputTooLarge,
    StaleSelection,
    AmbiguousHunk,
    PatchCheckFailed,
    IndexChangedExternally,
    HeadChangedExternally,
    UnsupportedFileKind,
    UnsupportedEncoding,
    HooksMutatedRepository,
    Internal,
}

impl GitErrorKind {
    /// Returns the stable machine-readable error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::RepositoryNotFound => "repository_not_found",
            Self::InvalidRepository => "invalid_repository",
            Self::GitUnavailable => "git_unavailable",
            Self::UnsupportedGitVersion => "unsupported_git_version",
            Self::DirtyWorktree => "dirty_worktree",
            Self::IndexLocked => "index_locked",
            Self::ReferenceChanged => "reference_changed",
            Self::InvalidPath => "invalid_path",
            Self::ObjectNotFound => "object_not_found",
            Self::AuthenticationRequired => "authentication_required",
            Self::AuthenticationFailed => "authentication_failed",
            Self::NetworkFailure => "network_failure",
            Self::MergeConflict => "merge_conflict",
            Self::OperationAlreadyInProgress => "operation_already_in_progress",
            Self::HookRejected => "hook_rejected",
            Self::SigningFailed => "signing_failed",
            Self::PermissionDenied => "permission_denied",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::UnsupportedFeature => "unsupported_feature",
            Self::OutputTooLarge => "output_too_large",
            Self::StaleSelection => "stale_selection",
            Self::AmbiguousHunk => "ambiguous_hunk",
            Self::PatchCheckFailed => "patch_check_failed",
            Self::IndexChangedExternally => "index_changed_externally",
            Self::HeadChangedExternally => "head_changed_externally",
            Self::UnsupportedFileKind => "unsupported_file_kind",
            Self::UnsupportedEncoding => "unsupported_encoding",
            Self::HooksMutatedRepository => "hooks_mutated_repository",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for GitErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// Safe follow-up actions that an application may expose for a failure.
///
/// Availability is determined after inspecting actual Git state; this enum does not authorize an
/// operation or imply that an action is safe to execute without a new preflight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    RefreshOnly,
    ContinueMerge,
    AbortMerge,
    ContinueRebase,
    SkipRebase,
    AbortRebase,
    ContinueCherryPick,
    AbortCherryPick,
    InspectRepository,
}

/// A structured, serializable Git failure.
///
/// `message` and `diagnostic` must already be redacted at the adapter boundary. This type never
/// stores retryability as a boolean; callers use actual outcomes and `recovery_actions` instead.
#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize, Deserialize)]
#[error("{kind}: {message}")]
pub struct GitError {
    pub kind: GitErrorKind,
    pub message: String,
    pub diagnostic: Option<String>,
    pub target: Option<SessionTarget>,
    pub operation_id: Option<OperationId>,
    pub recovery_actions: Vec<RecoveryAction>,
}

impl GitError {
    /// Creates an error without optional diagnostic or execution context.
    #[must_use]
    pub fn new(kind: GitErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            diagnostic: None,
            target: None,
            operation_id: None,
            recovery_actions: Vec::new(),
        }
    }

    /// Adds an already-redacted diagnostic.
    #[must_use]
    pub fn with_diagnostic(mut self, diagnostic: impl Into<String>) -> Self {
        self.diagnostic = Some(diagnostic.into());
        self
    }

    /// Adds repository-session context.
    #[must_use]
    pub fn with_target(mut self, target: SessionTarget) -> Self {
        self.target = Some(target);
        self
    }

    /// Adds operation context.
    #[must_use]
    pub fn with_operation(mut self, operation_id: OperationId) -> Self {
        self.operation_id = Some(operation_id);
        self
    }

    /// Replaces the safe actions offered after inspecting actual Git state.
    #[must_use]
    pub fn with_recovery_actions(
        mut self,
        recovery_actions: impl IntoIterator<Item = RecoveryAction>,
    ) -> Self {
        self.recovery_actions = recovery_actions.into_iter().collect();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RepositoryId, WorktreeId};

    #[test]
    fn error_preserves_structured_context_and_stable_code() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 3);
        let operation = OperationId::new();
        let error = GitError::new(GitErrorKind::IndexLocked, "index is locked")
            .with_diagnostic("redacted lock owner")
            .with_target(target)
            .with_operation(operation)
            .with_recovery_actions([
                RecoveryAction::RefreshOnly,
                RecoveryAction::InspectRepository,
            ]);

        assert_eq!(error.kind.code(), "index_locked");
        assert_eq!(error.target, Some(target));
        assert_eq!(error.operation_id, Some(operation));
        assert_eq!(
            error.recovery_actions,
            [
                RecoveryAction::RefreshOnly,
                RecoveryAction::InspectRepository
            ]
        );
        assert_eq!(error.to_string(), "index_locked: index is locked");

        let json = serde_json::to_string(&error).expect("Git error should serialize");
        let decoded: GitError = serde_json::from_str(&json).expect("Git error should deserialize");
        assert_eq!(decoded, error);
    }

    #[test]
    fn new_error_does_not_claim_retry_or_recovery() {
        let error = GitError::new(GitErrorKind::Internal, "unexpected failure");

        assert!(error.diagnostic.is_none());
        assert!(error.target.is_none());
        assert!(error.operation_id.is_none());
        assert!(error.recovery_actions.is_empty());
    }
}
