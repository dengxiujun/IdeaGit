//! Status parsing and repository-reading contracts.

use async_trait::async_trait;
use git_domain::{GitResult, StatusData};
use thiserror::Error;

use crate::{ExecutionContext, RepositoryLocation};

/// A byte-level porcelain v2 parse failure.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StatusParseError {
    #[error("truncated porcelain v2 output at byte {offset}: {context}")]
    Truncated {
        offset: usize,
        context: &'static str,
    },
    #[error("unknown porcelain v2 record 0x{marker:02x} at byte {offset}")]
    UnknownRecord { offset: usize, marker: u8 },
    #[error("invalid {field} at byte {offset}: {reason}")]
    InvalidField {
        offset: usize,
        field: &'static str,
        reason: String,
    },
    #[error("duplicate {header} header at byte {offset}")]
    DuplicateHeader { offset: usize, header: &'static str },
}

/// Parses exactly one complete `git status --porcelain=v2 -z --branch` payload.
pub trait StatusParser: Send + Sync {
    fn parse(&self, raw: &[u8]) -> Result<StatusData, StatusParseError>;
}

/// Read options that affect which non-tracked entries Git emits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusOptions {
    pub include_untracked: bool,
    pub include_ignored: bool,
}

impl Default for StatusOptions {
    fn default() -> Self {
        Self {
            include_untracked: true,
            include_ignored: false,
        }
    }
}

/// Reads and parses repository status through a replaceable adapter.
#[async_trait]
pub trait StatusReader: Send + Sync {
    async fn status(
        &self,
        location: &RepositoryLocation,
        options: StatusOptions,
        context: ExecutionContext,
    ) -> GitResult<StatusData>;
}
