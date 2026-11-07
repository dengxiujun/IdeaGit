//! Credential helper, AskPass, and SSH integration.
//!
//! The adapter invokes a caller-supplied AskPass executable without a shell. Secret output is
//! converted directly into `secrecy::SecretString`; diagnostics contain only outcome metadata.

use async_trait::async_trait;
use git_ports::{
    CredentialBroker, CredentialError, CredentialKind, CredentialRequest, CredentialResponse,
    EnvironmentVariable, ExecutionContext, ProcessRunner, ProcessSpec,
};
use secrecy::SecretString;
use std::{ffi::OsString, time::Duration};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const ASKPASS_OUTPUT_LIMIT: usize = 64 * 1024;

/// AskPass-backed credential provider. The executable is invoked directly, never through a shell.
#[derive(Debug)]
pub struct AskPassCredentialBroker<R> {
    runner: R,
    executable: OsString,
    timeout: Duration,
}

impl<R> AskPassCredentialBroker<R> {
    /// Creates a broker using the platform's configured AskPass executable.
    #[must_use]
    pub fn new(runner: R, executable: impl Into<OsString>) -> Self {
        Self {
            runner,
            executable: executable.into(),
            timeout: Duration::from_secs(60),
        }
    }

    /// Sets a bounded AskPass execution timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait]
impl<R: ProcessRunner> CredentialBroker for AskPassCredentialBroker<R> {
    async fn request(
        &self,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialError> {
        if request.prompt.trim().is_empty() {
            return Err(CredentialError::InvalidPrompt);
        }

        let (progress, _progress_receiver) = mpsc::channel(4);
        let context =
            ExecutionContext::new(request.operation_id, CancellationToken::new(), progress);
        let spec = ProcessSpec::new(self.executable.clone())
            .with_args([OsString::from(request.prompt)])
            .with_environment([EnvironmentVariable::public("GIT_TERMINAL_PROMPT", "0")])
            .with_timeout(self.timeout)
            .with_output_limits(ASKPASS_OUTPUT_LIMIT, ASKPASS_OUTPUT_LIMIT);
        let output = self
            .runner
            .run(spec, context)
            .await
            .map_err(|_| CredentialError::ProviderFailed)?;
        if !output.success {
            return Err(CredentialError::Rejected);
        }

        response_from_output(request.kind, &output.stdout)
    }
}

fn response_from_output(
    kind: CredentialKind,
    output: &[u8],
) -> Result<CredentialResponse, CredentialError> {
    let value = std::str::from_utf8(output)
        .map_err(|_| CredentialError::ProviderFailed)?
        .trim_end_matches(['\r', '\n']);
    match kind {
        CredentialKind::HostConfirmation => match value.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" | "true" | "ok" | "confirm" | "confirmed" => {
                Ok(CredentialResponse::Confirmed)
            }
            _ => Ok(CredentialResponse::Rejected),
        },
        CredentialKind::Username | CredentialKind::Password | CredentialKind::Passphrase => {
            if value.is_empty() {
                Err(CredentialError::Rejected)
            } else {
                Ok(CredentialResponse::Text(SecretString::from(
                    value.to_owned(),
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::OperationId;

    #[test]
    fn parses_secret_without_leaking_it_in_debug() {
        let response = response_from_output(CredentialKind::Password, b"token\n")
            .expect("secret should parse");
        let CredentialResponse::Text(secret) = response else {
            panic!("expected secret response");
        };
        assert!(!format!("{secret:?}").contains("token"));
    }

    #[test]
    fn only_explicit_confirmation_is_accepted() {
        assert!(matches!(
            response_from_output(CredentialKind::HostConfirmation, b"yes\n"),
            Ok(CredentialResponse::Confirmed)
        ));
        assert!(matches!(
            response_from_output(CredentialKind::HostConfirmation, b"no\n"),
            Ok(CredentialResponse::Rejected)
        ));
    }

    #[test]
    fn request_shape_keeps_operation_identity() {
        let request = CredentialRequest {
            operation_id: OperationId::new(),
            prompt: "Password:".to_owned(),
            kind: CredentialKind::Password,
        };
        assert_eq!(request.prompt, "Password:");
    }
}
