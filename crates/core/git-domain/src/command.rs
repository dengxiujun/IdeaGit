//! Strongly typed commands accepted by the application write boundary.
//!
//! Commands carry exactly one [`SessionTarget`]. They describe intent only; adapters and the
//! operation coordinator remain responsible for preflight, locking, execution, and reconcile.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{ObjectId, RefName, RepoPath, SessionTarget, SnapshotRevision};

/// Stable identity for a temporary edited-index session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EditedIndexSessionId(Uuid);

impl EditedIndexSessionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EditedIndexSessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// A repository file identity used by selection commands.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileIdentity {
    pub target: SessionTarget,
    pub path: RepoPath,
    pub previous_path: Option<RepoPath>,
    pub kind: FileKind,
}

/// File kind required to interpret a selection safely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileKind {
    Regular,
    Symlink,
    Submodule,
}

/// A content selection for a future partial-stage/commit implementation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeSelection {
    WholeFile {
        file: FileIdentity,
    },
    Hunks {
        file: FileIdentity,
        ids: Vec<u32>,
    },
    Lines {
        file: FileIdentity,
        hunk: u32,
        ranges: Vec<LineSpan>,
    },
}

impl ChangeSelection {
    #[must_use]
    pub fn target(&self) -> SessionTarget {
        match self {
            Self::WholeFile { file } | Self::Hunks { file, .. } | Self::Lines { file, .. } => {
                file.target
            }
        }
    }
}

/// Zero-based, half-open line range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineSpan {
    pub start: u32,
    pub end: u32,
}

/// Strong fingerprint binding a selection to the observed repository state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionFingerprint {
    pub expected_head: Option<ObjectId>,
    pub expected_index: Option<ObjectId>,
    pub diff_generation: SnapshotRevision,
    pub content_digest: Vec<u8>,
}

/// Commit signing behavior; no option silently bypasses Git hooks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SigningMode {
    #[default]
    UseGitConfig,
    Disable,
    Require,
}

/// Standard Index commit options.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitOptions {
    pub message: Vec<u8>,
    pub amend: bool,
    pub allow_empty: bool,
    pub signoff: bool,
    pub author: Option<Vec<u8>>,
    pub committer: Option<Vec<u8>>,
    pub signing: SigningMode,
}

/// Standard commit bound to a previously observed HEAD/Index revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardCommitRequest {
    pub options: CommitOptions,
    pub expected_revision: SnapshotRevision,
    pub expected_head: Option<ObjectId>,
    pub expected_index: Option<ObjectId>,
}

/// Selection commit request; unlike a standard commit it carries explicit selections and proofs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionCommitRequest {
    pub options: CommitOptions,
    pub selections: Vec<ChangeSelection>,
    pub fingerprints: Vec<SelectionFingerprint>,
    pub expected_revision: SnapshotRevision,
}

/// Branch operation requested against one session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchCommand {
    Create { name: RefName },
    Checkout { name: RefName },
    Rename { from: RefName, to: RefName },
    Delete { name: RefName, force: bool },
}

/// Merge operation requested against one session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeCommand {
    pub source: RefName,
    pub no_ff: bool,
}

/// Remote operation placeholder with typed intent, not an arbitrary shell command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteCommand {
    Fetch {
        remote: Option<RefName>,
    },
    Push {
        remote: Option<RefName>,
        branch: Option<RefName>,
        force_with_lease: bool,
    },
    Pull {
        remote: Option<RefName>,
        branch: Option<RefName>,
    },
}

/// A configured remote without exposing credentials or human-formatted diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub name: RefName,
    pub fetch_url: Vec<u8>,
    pub push_url: Vec<u8>,
}

/// Push safety mode selected after preview and preflight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PushMode {
    Normal,
    ForceWithLease { expected: ObjectId },
}

/// Immutable push preview bound to one source and destination ref.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushPreview {
    pub remote: RefName,
    pub source_ref: RefName,
    pub destination_ref: RefName,
    pub outgoing: Vec<crate::CommitSummary>,
    pub expected_remote_oid: Option<ObjectId>,
    pub mode: PushMode,
}

/// Sequencer operation kept separate from ordinary command state.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SequencerCommand {
    CherryPick { commit: ObjectId },
    Revert { commit: ObjectId, no_edit: bool },
    Continue,
    Skip,
    Abort,
}

/// Kind of an active Git sequencer state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SequencerKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// Read-only observation of Git's active sequencer state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequencerSnapshot {
    pub kind: SequencerKind,
    pub active: bool,
    pub current: Option<u32>,
    pub total: Option<u32>,
    pub conflicts: Vec<RepoPath>,
}

/// Every write intent accepted by the application boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitCommand {
    StageFiles {
        target: SessionTarget,
        paths: Vec<RepoPath>,
    },
    UnstageFiles {
        target: SessionTarget,
        paths: Vec<RepoPath>,
    },
    StageSelection {
        target: SessionTarget,
        selection: ChangeSelection,
        fingerprint: SelectionFingerprint,
    },
    UnstageSelection {
        target: SessionTarget,
        selection: ChangeSelection,
        fingerprint: SelectionFingerprint,
    },
    ApplyEditedIndex {
        target: SessionTarget,
        session: EditedIndexSessionId,
    },
    CommitIndex {
        target: SessionTarget,
        request: StandardCommitRequest,
    },
    CommitSelection {
        target: SessionTarget,
        request: SelectionCommitRequest,
    },
    Branch {
        target: SessionTarget,
        command: BranchCommand,
    },
    Merge {
        target: SessionTarget,
        command: MergeCommand,
    },
    Remote {
        target: SessionTarget,
        command: RemoteCommand,
    },
    Sequencer {
        target: SessionTarget,
        command: SequencerCommand,
    },
}

impl GitCommand {
    #[must_use]
    pub const fn target(&self) -> SessionTarget {
        match self {
            Self::StageFiles { target, .. }
            | Self::UnstageFiles { target, .. }
            | Self::StageSelection { target, .. }
            | Self::UnstageSelection { target, .. }
            | Self::ApplyEditedIndex { target, .. }
            | Self::CommitIndex { target, .. }
            | Self::CommitSelection { target, .. }
            | Self::Branch { target, .. }
            | Self::Merge { target, .. }
            | Self::Remote { target, .. }
            | Self::Sequencer { target, .. } => *target,
        }
    }

    /// Validates command-local invariants before a coordinator accepts the intent.
    pub fn validate(&self) -> Result<(), CommandValidationError> {
        match self {
            Self::StageFiles { paths, .. } | Self::UnstageFiles { paths, .. }
                if paths.is_empty() =>
            {
                Err(CommandValidationError::EmptyPaths)
            }
            Self::StageSelection {
                target, selection, ..
            }
            | Self::UnstageSelection {
                target, selection, ..
            } if selection.target() != *target => Err(CommandValidationError::TargetMismatch),
            Self::CommitIndex { request, .. } if request.options.message.is_empty() => {
                Err(CommandValidationError::EmptyCommitMessage)
            }
            Self::CommitSelection { target, request } => {
                if request.options.message.is_empty() {
                    return Err(CommandValidationError::EmptyCommitMessage);
                }
                if request.selections.is_empty() {
                    return Err(CommandValidationError::EmptySelection);
                }
                if request
                    .selections
                    .iter()
                    .any(|selection| selection.target() != *target)
                {
                    return Err(CommandValidationError::TargetMismatch);
                }
                if request.fingerprints.len() != request.selections.len() {
                    return Err(CommandValidationError::FingerprintCountMismatch);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Command-local validation failures; execution failures are structured [`GitError`] values.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum CommandValidationError {
    #[error("stage or unstage requires at least one path")]
    EmptyPaths,
    #[error("nested command target does not match command target")]
    TargetMismatch,
    #[error("commit message must not be empty")]
    EmptyCommitMessage,
    #[error("selection commit requires at least one selection")]
    EmptySelection,
    #[error("selection fingerprint count must match selection count")]
    FingerprintCountMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RepositoryId, WorktreeId};

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn path(value: &str) -> RepoPath {
        RepoPath::try_from(value).expect("test path should be valid")
    }

    fn options() -> CommitOptions {
        CommitOptions {
            message: b"message".to_vec(),
            amend: false,
            allow_empty: false,
            signoff: false,
            author: None,
            committer: None,
            signing: SigningMode::UseGitConfig,
        }
    }

    #[test]
    fn every_command_exposes_one_stable_target() {
        let target = target();
        let command = GitCommand::StageFiles {
            target,
            paths: vec![path("file.txt")],
        };
        assert_eq!(command.target(), target);
        assert_eq!(command.validate(), Ok(()));
    }

    #[test]
    fn selection_commands_reject_cross_session_identity() {
        let target_a = target();
        let other = target();
        let selection = ChangeSelection::WholeFile {
            file: FileIdentity {
                target: other,
                path: path("file.txt"),
                previous_path: None,
                kind: FileKind::Regular,
            },
        };
        let command = GitCommand::StageSelection {
            target: target_a,
            selection,
            fingerprint: SelectionFingerprint {
                expected_head: None,
                expected_index: None,
                diff_generation: SnapshotRevision::INITIAL,
                content_digest: vec![1],
            },
        };
        assert_eq!(
            command.validate(),
            Err(CommandValidationError::TargetMismatch)
        );
    }

    #[test]
    fn standard_and_selection_commits_reject_unsafe_empty_inputs() {
        let target = target();
        let standard = GitCommand::CommitIndex {
            target,
            request: StandardCommitRequest {
                options: CommitOptions {
                    message: Vec::new(),
                    ..options()
                },
                expected_revision: SnapshotRevision::INITIAL,
                expected_head: None,
                expected_index: None,
            },
        };
        assert_eq!(
            standard.validate(),
            Err(CommandValidationError::EmptyCommitMessage)
        );
        let selection = GitCommand::CommitSelection {
            target,
            request: SelectionCommitRequest {
                options: options(),
                selections: Vec::new(),
                fingerprints: Vec::new(),
                expected_revision: SnapshotRevision::INITIAL,
            },
        };
        assert_eq!(
            selection.validate(),
            Err(CommandValidationError::EmptySelection)
        );
    }
}
