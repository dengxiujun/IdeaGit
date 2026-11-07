//! Git CLI process execution and machine-output adapters.

mod branch;
mod commit;
mod executor;
mod head;
mod history;
mod index;
mod remote;
mod repository;
mod runner;
mod status;

pub use branch::GitCliBranchWriter;
pub use commit::{GitCliCommitState, GitCliCommitStateProbe, GitCliCommitWriter};
pub use executor::{GitCliCommand, GitCliExecutor, GitVersionCommand};
pub use head::GitCliHeadReader;
pub use history::GitCliHistoryReader;
pub use index::GitCliIndexWriter;
pub use remote::GitCliRemoteAdapter;
pub use repository::GitCliRepositoryReader;
pub use runner::TokioProcessRunner;
pub use status::{GitCliStatusReader, PorcelainV2StatusParser};
