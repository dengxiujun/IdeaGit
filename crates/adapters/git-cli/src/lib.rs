//! Git CLI process execution and machine-output adapters.

mod executor;
mod head;
mod repository;
mod runner;
mod status;

pub use executor::{GitCliCommand, GitCliExecutor, GitVersionCommand};
pub use head::GitCliHeadReader;
pub use repository::GitCliRepositoryReader;
pub use runner::TokioProcessRunner;
pub use status::{GitCliStatusReader, PorcelainV2StatusParser};
