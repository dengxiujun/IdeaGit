//! Git CLI process execution and machine-output adapters.

mod executor;
mod runner;

pub use executor::{GitCliCommand, GitCliExecutor, GitVersionCommand};
pub use runner::TokioProcessRunner;
