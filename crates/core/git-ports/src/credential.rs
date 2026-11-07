//! Credential and host-confirmation boundary for remote Git operations.

use async_trait::async_trait;
use git_domain::OperationId;
use secrecy::SecretString;
use thiserror::Error;

/// The kind of secret or confirmation requested by Git.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialKind {
    Username,
    Password,
    Passphrase,
    HostConfirmation,
}

/// A redacted prompt scoped to one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialRequest {
    pub operation_id: OperationId,
    pub prompt: String,
    pub kind: CredentialKind,
}

/// Response returned by a credential UI or AskPass adapter.
#[derive(Debug)]
pub enum CredentialResponse {
    Text(SecretString),
    Confirmed,
    Rejected,
    Cancelled,
}

/// Failure to obtain or validate a credential response.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CredentialError {
    #[error("credential request was rejected")]
    Rejected,
    #[error("credential request was cancelled")]
    Cancelled,
    #[error("credential provider is unavailable")]
    Unavailable,
    #[error("credential prompt is invalid")]
    InvalidPrompt,
    #[error("credential provider failed")]
    ProviderFailed,
}

/// Async credential boundary used by remote adapters without exposing secrets to Git commands.
#[async_trait]
pub trait CredentialBroker: Send + Sync {
    async fn request(
        &self,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialError>;
}
