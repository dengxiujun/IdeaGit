//! Git CLI process execution and machine-output adapters.

mod branch;
mod commit;
mod conflict;
mod executor;
mod head;
mod history;
mod index;
mod merge;
mod remote;
mod repository;
mod runner;
mod sequencer;
mod status;

pub use branch::GitCliBranchWriter;
pub use commit::{GitCliCommitState, GitCliCommitStateProbe, GitCliCommitWriter};
pub use conflict::GitCliConflictStageReader;
pub use executor::{GitCliCommand, GitCliExecutor, GitVersionCommand};
pub use head::GitCliHeadReader;
pub use history::GitCliHistoryReader;
pub use index::GitCliIndexWriter;
pub use merge::GitCliMergeWriter;
pub use remote::GitCliRemoteAdapter;
pub use repository::GitCliRepositoryReader;
pub use runner::TokioProcessRunner;
pub use sequencer::{GitCliSequencerReader, GitCliSequencerWriter};
pub use status::{GitCliStatusReader, PorcelainV2StatusParser};
