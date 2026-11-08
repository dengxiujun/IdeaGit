//! Git CLI process execution and machine-output adapters.

mod branch;
mod candidate_tree;
mod clone;
mod commit;
mod conflict;
mod diff;
mod editable_index;
mod executor;
mod head;
mod history;
mod hunk_stage;
mod index;
mod index_reconciliation;
mod merge;
mod rebase;
mod remote;
mod repository;
mod resolution;
mod runner;
mod sequencer;
mod shelf;
mod status;
mod temporary_index;
mod version;

pub use branch::GitCliBranchWriter;
pub use candidate_tree::GitCliCandidateTreeBuilder;
pub use clone::GitCliCloneWriter;
pub use commit::{
    GitCliCommitSideEffectObserver, GitCliCommitState, GitCliCommitStateProbe, GitCliCommitWriter,
    GitCliTemporaryIndexCommitWriter,
};
pub use conflict::GitCliConflictStageReader;
pub use diff::GitCliDiffReader;
pub use editable_index::GitCliEditableIndexCandidateBuilder;
pub use executor::{GitCliCommand, GitCliExecutor, GitVersionCommand};
pub use head::GitCliHeadReader;
pub use history::GitCliHistoryReader;
pub use hunk_stage::GitCliHunkStageWriter;
pub use index::GitCliIndexWriter;
pub use index_reconciliation::GitCliIndexReconciliationPublisher;
pub use merge::GitCliMergeWriter;
pub use rebase::{GitCliInteractiveRebasePlanReader, GitCliInteractiveRebaseWriter};
pub use remote::GitCliRemoteAdapter;
pub use repository::GitCliRepositoryReader;
pub use resolution::GitCliConflictResolutionWriter;
pub use runner::TokioProcessRunner;
pub use sequencer::{GitCliSequencerReader, GitCliSequencerWriter};
pub use shelf::GitCliShelfWriter;
pub use status::{GitCliStatusReader, PorcelainV2StatusParser};
pub use temporary_index::GitCliTemporaryIndexManager;
pub use version::GitCliVersionReader;
