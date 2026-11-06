//! Process execution contracts shared by adapters and application services.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use git_domain::{GitResult, OperationId};
use tokio::{sync::mpsc, time::Instant};
use tokio_util::sync::CancellationToken;

/// Default maximum retained bytes for each output stream.
pub const DEFAULT_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

/// Sensitivity marker for an environment value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensitivity {
    Public,
    Sensitive,
}

/// An explicit environment override or removal.
#[derive(Clone, PartialEq, Eq)]
pub struct EnvironmentVariable {
    name: OsString,
    value: Option<OsString>,
    sensitivity: Sensitivity,
}

impl EnvironmentVariable {
    /// Creates a non-sensitive environment override.
    #[must_use]
    pub fn public(name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        Self {
            name: name.into(),
            value: Some(value.into()),
            sensitivity: Sensitivity::Public,
        }
    }

    /// Creates an environment override whose value must never appear in diagnostics.
    #[must_use]
    pub fn sensitive(name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        Self {
            name: name.into(),
            value: Some(value.into()),
            sensitivity: Sensitivity::Sensitive,
        }
    }

    /// Creates an explicit environment removal.
    #[must_use]
    pub fn remove(name: impl Into<OsString>) -> Self {
        Self {
            name: name.into(),
            value: None,
            sensitivity: Sensitivity::Public,
        }
    }

    #[must_use]
    pub fn name(&self) -> &OsStr {
        &self.name
    }

    #[must_use]
    pub fn value(&self) -> Option<&OsStr> {
        self.value.as_deref()
    }

    #[must_use]
    pub const fn sensitivity(&self) -> Sensitivity {
        self.sensitivity
    }
}

impl fmt::Debug for EnvironmentVariable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentVariable")
            .field("name", &self.name)
            .field("value", &self.value.as_ref().map(|_| "[redacted]"))
            .field("sensitivity", &self.sensitivity)
            .finish()
    }
}

/// Input supplied to the child without invoking a shell.
pub enum ProcessStdin {
    Empty,
    Bytes(Vec<u8>),
    Stream(mpsc::Receiver<Vec<u8>>),
}

impl fmt::Debug for ProcessStdin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Empty"),
            Self::Bytes(bytes) => formatter
                .debug_struct("Bytes")
                .field("length", &bytes.len())
                .finish(),
            Self::Stream(_) => formatter.write_str("Stream([redacted])"),
        }
    }
}

/// Identifies a child output stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessStream {
    Stdout,
    Stderr,
}

/// Bounded, content-free execution progress suitable for UI projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessProgress {
    Started {
        pid: Option<u32>,
    },
    Output {
        stream: ProcessStream,
        bytes: usize,
    },
    Finished {
        exit_code: Option<i32>,
        success: bool,
    },
}

/// Cancellation, deadline, and progress state for one operation.
#[derive(Clone, Debug)]
pub struct ExecutionContext {
    pub operation_id: OperationId,
    pub cancellation: CancellationToken,
    pub progress: mpsc::Sender<ProcessProgress>,
    pub deadline: Option<Instant>,
}

impl ExecutionContext {
    #[must_use]
    pub fn new(
        operation_id: OperationId,
        cancellation: CancellationToken,
        progress: mpsc::Sender<ProcessProgress>,
    ) -> Self {
        Self {
            operation_id,
            cancellation,
            progress,
            deadline: None,
        }
    }

    #[must_use]
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }
}

/// Complete low-level process specification. Arguments remain OS-native and are never shell text.
pub struct ProcessSpec {
    executable: OsString,
    cwd: Option<PathBuf>,
    args: Vec<OsString>,
    environment: Vec<EnvironmentVariable>,
    inherit_environment: bool,
    stdin: ProcessStdin,
    timeout: Option<Duration>,
    stdout_limit: usize,
    stderr_limit: usize,
}

impl ProcessSpec {
    #[must_use]
    pub fn new(executable: impl Into<OsString>) -> Self {
        Self {
            executable: executable.into(),
            cwd: None,
            args: Vec::new(),
            environment: Vec::new(),
            inherit_environment: true,
            stdin: ProcessStdin::Empty,
            timeout: None,
            stdout_limit: DEFAULT_OUTPUT_LIMIT,
            stderr_limit: DEFAULT_OUTPUT_LIMIT,
        }
    }

    #[must_use]
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    #[must_use]
    pub fn with_args(mut self, args: impl IntoIterator<Item = OsString>) -> Self {
        self.args = args.into_iter().collect();
        self
    }

    #[must_use]
    pub fn with_environment(
        mut self,
        environment: impl IntoIterator<Item = EnvironmentVariable>,
    ) -> Self {
        self.environment = environment.into_iter().collect();
        self
    }

    #[must_use]
    pub fn inherit_environment(mut self, inherit: bool) -> Self {
        self.inherit_environment = inherit;
        self
    }

    #[must_use]
    pub fn with_stdin(mut self, stdin: ProcessStdin) -> Self {
        self.stdin = stdin;
        self
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[must_use]
    pub fn with_output_limits(mut self, stdout_limit: usize, stderr_limit: usize) -> Self {
        self.stdout_limit = stdout_limit;
        self.stderr_limit = stderr_limit;
        self
    }

    #[must_use]
    pub fn executable(&self) -> &OsStr {
        &self.executable
    }

    #[must_use]
    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    #[must_use]
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    #[must_use]
    pub fn environment(&self) -> &[EnvironmentVariable] {
        &self.environment
    }

    #[must_use]
    pub const fn inherits_environment(&self) -> bool {
        self.inherit_environment
    }

    pub fn take_stdin(&mut self) -> ProcessStdin {
        std::mem::replace(&mut self.stdin, ProcessStdin::Empty)
    }

    #[must_use]
    pub const fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    #[must_use]
    pub const fn stdout_limit(&self) -> usize {
        self.stdout_limit
    }

    #[must_use]
    pub const fn stderr_limit(&self) -> usize {
        self.stderr_limit
    }
}

impl fmt::Debug for ProcessSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessSpec")
            .field("executable", &"[redacted]")
            .field("cwd", &self.cwd.as_ref().map(|_| "[redacted]"))
            .field("arg_count", &self.args.len())
            .field("environment_count", &self.environment.len())
            .field("inherit_environment", &self.inherit_environment)
            .field("stdin", &"[redacted]")
            .field("timeout", &self.timeout)
            .field("stdout_limit", &self.stdout_limit)
            .field("stderr_limit", &self.stderr_limit)
            .finish()
    }
}

/// Raw process outcome. Semantic Git exit-code interpretation belongs to higher adapters.
#[derive(Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub success: bool,
}

impl fmt::Debug for ProcessOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessOutput")
            .field("stdout_bytes", &self.stdout.len())
            .field("stderr_bytes", &self.stderr.len())
            .field("exit_code", &self.exit_code)
            .field("success", &self.success)
            .finish()
    }
}

/// Replaceable process execution boundary.
#[async_trait]
pub trait ProcessRunner: Send + Sync {
    async fn run(&self, spec: ProcessSpec, context: ExecutionContext) -> GitResult<ProcessOutput>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_redacts_command_environment_and_payloads() {
        let secret = "not-for-diagnostics";
        let spec = ProcessSpec::new(secret)
            .with_cwd(secret)
            .with_args([OsString::from(secret)])
            .with_environment([EnvironmentVariable::sensitive("TOKEN", secret)])
            .with_stdin(ProcessStdin::Bytes(secret.as_bytes().to_vec()));

        let debug = format!("{spec:?}");
        assert!(!debug.contains(secret));

        let env_debug = format!("{:?}", spec.environment()[0]);
        assert!(!env_debug.contains(secret));

        let stdin_debug = format!("{:?}", ProcessStdin::Bytes(secret.as_bytes().to_vec()));
        assert!(!stdin_debug.contains(secret));

        let output = ProcessOutput {
            stdout: secret.as_bytes().to_vec(),
            stderr: secret.as_bytes().to_vec(),
            exit_code: Some(1),
            success: false,
        };
        assert!(!format!("{output:?}").contains(secret));
    }
}
