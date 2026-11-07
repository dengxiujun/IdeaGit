//! Git executable composition kept behind typed adapter commands.

use std::{ffi::OsString, path::PathBuf, time::Duration};

use git_domain::{GitResult, SessionTarget};
use git_ports::{
    EnvironmentVariable, ExecutionContext, ProcessOutput, ProcessRunner, ProcessSpec, ProcessStdin,
};

/// Shared Git CLI executor used by typed commands implemented in this adapter crate.
#[derive(Debug)]
pub struct GitCliExecutor<R> {
    runner: R,
    executable: OsString,
}

impl<R> GitCliExecutor<R> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            executable: OsString::from("git"),
        }
    }

    #[must_use]
    pub fn with_executable(runner: R, executable: impl Into<OsString>) -> Self {
        Self {
            runner,
            executable: executable.into(),
        }
    }
}

impl<R: ProcessRunner> GitCliExecutor<R> {
    /// Executes a sealed, typed Git command without invoking a shell.
    #[allow(clippy::result_large_err)]
    pub async fn execute<C: GitCliCommand>(
        &self,
        command: C,
        context: ExecutionContext,
    ) -> GitResult<ProcessOutput> {
        let invocation = sealed::Sealed::into_invocation(command);
        let target = invocation.target;
        let mut spec = ProcessSpec::new(self.executable.clone())
            .with_args(invocation.args)
            .with_environment(invocation.environment)
            .with_stdin(invocation.stdin)
            .with_output_limits(invocation.stdout_limit, invocation.stderr_limit);

        if let Some(cwd) = invocation.cwd {
            spec = spec.with_cwd(cwd);
        }
        if let Some(timeout) = invocation.timeout {
            spec = spec.with_timeout(timeout);
        }

        self.runner
            .run(spec, context)
            .await
            .map_err(|error| match (error.target, target) {
                (None, Some(target)) => error.with_target(target),
                _ => error,
            })
    }
}

/// Marker for Git commands defined by this adapter.
///
/// The trait is sealed: callers may execute supported typed commands but cannot inject arbitrary
/// argument vectors.
pub trait GitCliCommand: sealed::Sealed + Send {}

/// Typed command used to probe the selected Git executable.
#[derive(Clone, Copy, Debug, Default)]
pub struct GitVersionCommand;

impl GitCliCommand for GitVersionCommand {}

impl sealed::Sealed for GitVersionCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(None, [OsString::from("--version")])
    }
}

/// Opaque low-level invocation assembled only by sealed Git CLI adapter commands.
#[doc(hidden)]
pub struct GitCliInvocation {
    target: Option<SessionTarget>,
    cwd: Option<PathBuf>,
    args: Vec<OsString>,
    environment: Vec<EnvironmentVariable>,
    stdin: ProcessStdin,
    timeout: Option<Duration>,
    stdout_limit: usize,
    stderr_limit: usize,
}

impl GitCliInvocation {
    pub(crate) fn new(
        target: Option<SessionTarget>,
        args: impl IntoIterator<Item = OsString>,
    ) -> Self {
        Self {
            target,
            cwd: None,
            args: args.into_iter().collect(),
            environment: Vec::new(),
            stdin: ProcessStdin::Empty,
            timeout: None,
            stdout_limit: git_ports::DEFAULT_OUTPUT_LIMIT,
            stderr_limit: git_ports::DEFAULT_OUTPUT_LIMIT,
        }
    }

    pub(crate) fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub(crate) fn with_environment(
        mut self,
        environment: impl IntoIterator<Item = EnvironmentVariable>,
    ) -> Self {
        self.environment = environment.into_iter().collect();
        self
    }

    pub(crate) fn with_stdin(mut self, stdin: ProcessStdin) -> Self {
        self.stdin = stdin;
        self
    }
}

pub(crate) mod sealed {
    use super::GitCliInvocation;

    pub trait Sealed {
        fn into_invocation(self) -> GitCliInvocation;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use git_domain::{
        GitError, GitErrorKind, OperationId, RepositoryId, SessionTarget, WorktreeId,
    };
    use git_ports::{ProcessProgress, ProcessRunner};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[derive(Default)]
    struct RecordingRunner {
        executable: Mutex<Option<OsString>>,
    }

    #[async_trait]
    impl ProcessRunner for RecordingRunner {
        async fn run(
            &self,
            spec: ProcessSpec,
            _context: ExecutionContext,
        ) -> GitResult<ProcessOutput> {
            *self.executable.lock().expect("recording mutex poisoned") =
                Some(spec.executable().to_owned());
            Err(GitError::new(GitErrorKind::Internal, "synthetic failure"))
        }
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(4);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    #[tokio::test]
    async fn executor_uses_configured_binary_and_attaches_target_to_errors() {
        let runner = RecordingRunner::default();
        let executor = GitCliExecutor::with_executable(runner, "custom-git");
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        struct TargetCommand(SessionTarget);

        impl GitCliCommand for TargetCommand {}

        impl sealed::Sealed for TargetCommand {
            fn into_invocation(self) -> GitCliInvocation {
                GitCliInvocation::new(Some(self.0), [OsString::from("--version")])
            }
        }

        let error = executor
            .execute(TargetCommand(target), context())
            .await
            .expect_err("synthetic failure should be returned");

        assert_eq!(error.target, Some(target));
        assert_eq!(
            executor
                .runner
                .executable
                .lock()
                .expect("recording mutex poisoned")
                .as_deref(),
            Some(std::ffi::OsStr::new("custom-git"))
        );
    }

    #[tokio::test]
    async fn typed_version_command_runs_real_git_binary() {
        let executor = GitCliExecutor::new(crate::TokioProcessRunner);
        let output = executor
            .execute(GitVersionCommand, context())
            .await
            .expect("installed Git should execute");

        assert!(output.success);
        assert!(output.stdout.starts_with(b"git version "));
    }
}
