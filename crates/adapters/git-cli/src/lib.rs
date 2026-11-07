//! Git CLI process execution and machine-output adapters.

mod commit;
mod executor;
mod head;
mod index;
mod repository;
mod runner;
mod status;

pub use commit::GitCliCommitWriter;
pub use executor::{GitCliCommand, GitCliExecutor, GitVersionCommand};
pub use head::GitCliHeadReader;
pub use index::GitCliIndexWriter;
pub use repository::GitCliRepositoryReader;
pub use runner::TokioProcessRunner;
pub use status::{GitCliStatusReader, PorcelainV2StatusParser};
