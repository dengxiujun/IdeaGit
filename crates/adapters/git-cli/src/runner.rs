//! Tokio process runner with bounded pipes and platform process-tree ownership.

use std::{io, process::ExitStatus, time::Duration};

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult, OperationId};
use git_ports::{
    ExecutionContext, ProcessOutput, ProcessProgress, ProcessRunner, ProcessSpec, ProcessStdin,
    ProcessStream,
};
#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, Command},
    time::Instant,
};

/// Production process runner. It never invokes a shell.
#[derive(Clone, Copy, Debug, Default)]
pub struct TokioProcessRunner;

#[async_trait]
impl ProcessRunner for TokioProcessRunner {
    async fn run(
        &self,
        mut spec: ProcessSpec,
        context: ExecutionContext,
    ) -> GitResult<ProcessOutput> {
        if context.cancellation.is_cancelled() {
            return Err(context_error(
                GitErrorKind::Cancelled,
                "process execution was cancelled before start",
                context.operation_id,
            ));
        }

        let now = Instant::now();
        let deadline = effective_deadline(now, spec.timeout(), context.deadline);
        if deadline.is_some_and(|deadline| deadline <= now) {
            return Err(context_error(
                GitErrorKind::TimedOut,
                "process deadline elapsed before start",
                context.operation_id,
            ));
        }

        let mut command = Command::new(spec.executable());
        command
            .args(spec.args())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(cwd) = spec.cwd() {
            command.current_dir(cwd);
        }
        if !spec.inherits_environment() {
            command.env_clear();
        }
        for variable in spec.environment() {
            match variable.value() {
                Some(value) => {
                    command.env(variable.name(), value);
                }
                None => {
                    command.env_remove(variable.name());
                }
            }
        }

        let mut wrapped = CommandWrap::from(command);
        wrapped.wrap(KillOnDrop);
        #[cfg(unix)]
        wrapped.wrap(ProcessGroup::leader());
        #[cfg(windows)]
        wrapped.wrap(JobObject);

        let mut child = wrapped.spawn().map_err(|error| {
            io_error(
                GitErrorKind::GitUnavailable,
                "failed to start process",
                context.operation_id,
                &error,
            )
        })?;

        let pid = child.id();
        let stdin = child.stdin().take().ok_or_else(|| {
            context_error(
                GitErrorKind::Internal,
                "spawned process has no stdin pipe",
                context.operation_id,
            )
        })?;
        let stdout = child.stdout().take().ok_or_else(|| {
            context_error(
                GitErrorKind::Internal,
                "spawned process has no stdout pipe",
                context.operation_id,
            )
        })?;
        let stderr = child.stderr().take().ok_or_else(|| {
            context_error(
                GitErrorKind::Internal,
                "spawned process has no stderr pipe",
                context.operation_id,
            )
        })?;

        let _ = context.progress.try_send(ProcessProgress::Started { pid });
        let stdin_spec = spec.take_stdin();
        let operation_id = context.operation_id;
        let progress = context.progress.clone();
        let stdout_limit = spec.stdout_limit();
        let stderr_limit = spec.stderr_limit();

        enum Outcome {
            Completed(Result<(ExitStatus, Vec<u8>, Vec<u8>), CommunicationFailure>),
            Cancelled,
            TimedOut,
        }

        let outcome = {
            let communication = communicate(
                child.as_mut(),
                stdin,
                stdin_spec,
                stdout,
                stderr,
                stdout_limit,
                stderr_limit,
                progress,
            );
            tokio::pin!(communication);

            tokio::select! {
                result = &mut communication => Outcome::Completed(result),
                () = context.cancellation.cancelled() => Outcome::Cancelled,
                () = wait_for_deadline(deadline) => Outcome::TimedOut,
            }
        };

        match outcome {
            Outcome::Completed(Ok((status, stdout, stderr))) => {
                let output = ProcessOutput {
                    stdout,
                    stderr,
                    exit_code: status.code(),
                    success: status.success(),
                };
                let _ = context.progress.try_send(ProcessProgress::Finished {
                    exit_code: output.exit_code,
                    success: output.success,
                });
                Ok(output)
            }
            Outcome::Completed(Err(failure)) => {
                terminate_and_reap(child.as_mut(), operation_id).await?;
                Err(failure.into_git_error(operation_id))
            }
            Outcome::Cancelled => {
                terminate_and_reap(child.as_mut(), operation_id).await?;
                Err(context_error(
                    GitErrorKind::Cancelled,
                    "process execution was cancelled",
                    operation_id,
                ))
            }
            Outcome::TimedOut => {
                terminate_and_reap(child.as_mut(), operation_id).await?;
                Err(context_error(
                    GitErrorKind::TimedOut,
                    "process execution exceeded its deadline",
                    operation_id,
                ))
            }
        }
    }
}

fn effective_deadline(
    now: Instant,
    timeout: Option<Duration>,
    context_deadline: Option<Instant>,
) -> Option<Instant> {
    match (timeout.map(|timeout| now + timeout), context_deadline) {
        (Some(timeout), Some(deadline)) => Some(timeout.min(deadline)),
        (Some(timeout), None) => Some(timeout),
        (None, deadline) => deadline,
    }
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn communicate<Stdout, Stderr>(
    child: &mut dyn ChildWrapper,
    stdin: ChildStdin,
    stdin_spec: ProcessStdin,
    stdout: Stdout,
    stderr: Stderr,
    stdout_limit: usize,
    stderr_limit: usize,
    progress: tokio::sync::mpsc::Sender<ProcessProgress>,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), CommunicationFailure>
where
    Stdout: AsyncRead + Unpin,
    Stderr: AsyncRead + Unpin,
{
    let wait = async { child.wait().await.map_err(CommunicationFailure::Wait) };
    let write = write_stdin(stdin, stdin_spec);
    let reads = async {
        tokio::try_join!(
            read_limited(stdout, stdout_limit, ProcessStream::Stdout, &progress),
            read_limited(stderr, stderr_limit, ProcessStream::Stderr, &progress)
        )
    };
    tokio::pin!(wait, write, reads);

    let mut status = None;
    let mut outputs = None;
    let mut input_finished = false;
    while status.is_none() || outputs.is_none() {
        tokio::select! {
            result = &mut wait, if status.is_none() => status = Some(result?),
            result = &mut reads, if outputs.is_none() => outputs = Some(result?),
            result = &mut write, if !input_finished => {
                result?;
                input_finished = true;
            }
        }
    }

    let (stdout, stderr) = outputs.expect("loop exits only after output is available");
    Ok((
        status.expect("loop exits only after status is available"),
        stdout,
        stderr,
    ))
}

async fn write_stdin(
    mut stdin: ChildStdin,
    stdin_spec: ProcessStdin,
) -> Result<(), CommunicationFailure> {
    match stdin_spec {
        ProcessStdin::Empty => {}
        ProcessStdin::Bytes(bytes) => stdin
            .write_all(&bytes)
            .await
            .map_err(CommunicationFailure::Stdin)?,
        ProcessStdin::Stream(mut chunks) => {
            while let Some(chunk) = chunks.recv().await {
                stdin
                    .write_all(&chunk)
                    .await
                    .map_err(CommunicationFailure::Stdin)?;
            }
        }
    }
    stdin.shutdown().await.map_err(CommunicationFailure::Stdin)
}

async fn read_limited<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
    stream: ProcessStream,
    progress: &tokio::sync::mpsc::Sender<ProcessProgress>,
) -> Result<Vec<u8>, CommunicationFailure> {
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|source| CommunicationFailure::OutputIo { stream, source })?;
        if read == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(read) > limit {
            return Err(CommunicationFailure::OutputLimit { stream, limit });
        }
        output.extend_from_slice(&buffer[..read]);
        let _ = progress.try_send(ProcessProgress::Output {
            stream,
            bytes: read,
        });
    }
}

#[allow(clippy::result_large_err)]
async fn terminate_and_reap(
    child: &mut dyn ChildWrapper,
    operation_id: OperationId,
) -> GitResult<()> {
    let kill = Box::into_pin(child.kill());
    kill.await.map_err(|error| {
        io_error(
            GitErrorKind::Internal,
            "failed to terminate and reap process tree",
            operation_id,
            &error,
        )
    })
}

#[derive(Debug)]
enum CommunicationFailure {
    Wait(io::Error),
    Stdin(io::Error),
    OutputIo {
        stream: ProcessStream,
        source: io::Error,
    },
    OutputLimit {
        stream: ProcessStream,
        limit: usize,
    },
}

impl CommunicationFailure {
    fn into_git_error(self, operation_id: OperationId) -> GitError {
        match self {
            Self::Wait(error) => io_error(
                GitErrorKind::Internal,
                "failed while waiting for process",
                operation_id,
                &error,
            ),
            Self::Stdin(error) => io_error(
                GitErrorKind::Internal,
                "failed while writing process input",
                operation_id,
                &error,
            ),
            Self::OutputIo { stream, source } => io_error(
                GitErrorKind::Internal,
                "failed while reading process output",
                operation_id,
                &source,
            )
            .with_diagnostic(format!(
                "stream={stream:?}; io_kind={:?}; os_code={:?}",
                source.kind(),
                source.raw_os_error()
            )),
            Self::OutputLimit { stream, limit } => context_error(
                GitErrorKind::OutputTooLarge,
                "process output exceeded its configured limit",
                operation_id,
            )
            .with_diagnostic(format!("stream={stream:?}; limit_bytes={limit}")),
        }
    }
}

fn context_error(kind: GitErrorKind, message: &'static str, operation_id: OperationId) -> GitError {
    GitError::new(kind, message).with_operation(operation_id)
}

fn io_error(
    kind: GitErrorKind,
    message: &'static str,
    operation_id: OperationId,
    error: &io::Error,
) -> GitError {
    context_error(kind, message, operation_id).with_diagnostic(format!(
        "io_kind={:?}; os_code={:?}",
        error.kind(),
        error.raw_os_error()
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::OsString,
        fs,
        io::{Read, Write},
        process, thread,
    };

    use git_domain::OperationId;
    use git_ports::{EnvironmentVariable, ProcessProgress};
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;

    const HELPER_MODE: &str = "IDEAGIT_PROCESS_HELPER_MODE";
    const HELPER_MARKER: &str = "IDEAGIT_PROCESS_HELPER_MARKER";
    const HELPER_TEST: &str = "runner::tests::process_helper";

    fn context() -> (ExecutionContext, mpsc::Receiver<ProcessProgress>) {
        let (progress, receiver) = mpsc::channel(64);
        (
            ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            receiver,
        )
    }

    fn helper_spec(mode: &str) -> ProcessSpec {
        ProcessSpec::new(env::current_exe().expect("test executable should be available"))
            .with_args([
                OsString::from("--exact"),
                OsString::from(HELPER_TEST),
                OsString::from("--nocapture"),
            ])
            .with_environment([EnvironmentVariable::public(HELPER_MODE, mode)])
    }

    #[allow(clippy::result_large_err)]
    async fn run_with_guard(
        spec: ProcessSpec,
        context: ExecutionContext,
    ) -> GitResult<ProcessOutput> {
        tokio::time::timeout(
            Duration::from_secs(10),
            TokioProcessRunner.run(spec, context),
        )
        .await
        .expect("process runner test must not hang")
    }

    #[test]
    #[allow(clippy::zombie_processes)] // The parent is intentionally killed before it can wait.
    fn process_helper() {
        let Ok(mode) = env::var(HELPER_MODE) else {
            return;
        };

        match mode.as_str() {
            "echo" => {
                let mut input = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut input)
                    .expect("helper should read stdin");
                std::io::stdout()
                    .write_all(&input)
                    .expect("helper should write stdout");
            }
            "non-utf8" => {
                std::io::stdout()
                    .write_all(&[0x66, 0x80, 0xff, 0x0a])
                    .expect("helper should write raw bytes");
            }
            "large-both" => {
                std::io::stdout()
                    .write_all(&vec![b'o'; 256 * 1024])
                    .expect("helper should write large stdout");
                std::io::stderr()
                    .write_all(&vec![b'e'; 256 * 1024])
                    .expect("helper should write large stderr");
            }
            "exit-23" => process::exit(23),
            "sleep" => thread::sleep(Duration::from_secs(30)),
            "descendant-parent" => {
                let marker = env::var_os(HELPER_MARKER).expect("marker path should be set");
                process::Command::new(env::current_exe().expect("test executable should exist"))
                    .args(["--exact", HELPER_TEST, "--nocapture"])
                    .env(HELPER_MODE, "descendant")
                    .env(HELPER_MARKER, marker)
                    .spawn()
                    .expect("helper should spawn descendant");
                thread::sleep(Duration::from_secs(30));
            }
            "descendant" => {
                thread::sleep(Duration::from_millis(1200));
                let marker = env::var_os(HELPER_MARKER).expect("marker path should be set");
                fs::write(marker, b"descendant survived")
                    .expect("descendant should write marker when not terminated");
            }
            other => panic!("unknown helper mode: {other}"),
        }
    }

    #[tokio::test]
    async fn streams_stdin_and_reports_content_free_progress() {
        let (sender, receiver) = mpsc::channel(2);
        sender
            .send(b"first ".to_vec())
            .await
            .expect("stream receiver should be live");
        sender
            .send(b"second".to_vec())
            .await
            .expect("stream receiver should be live");
        drop(sender);
        let (context, mut progress) = context();

        let output = run_with_guard(
            helper_spec("echo").with_stdin(ProcessStdin::Stream(receiver)),
            context,
        )
        .await
        .expect("echo helper should succeed");

        assert!(output.success);
        assert!(
            output
                .stdout
                .windows(12)
                .any(|bytes| bytes == b"first second")
        );
        let events: Vec<_> = std::iter::from_fn(|| progress.try_recv().ok()).collect();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ProcessProgress::Started { .. }))
        );
        assert!(events.iter().any(|event| matches!(
            event,
            ProcessProgress::Output {
                stream: ProcessStream::Stdout,
                ..
            }
        )));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ProcessProgress::Finished { success: true, .. }))
        );
    }

    #[tokio::test]
    async fn process_exit_does_not_wait_for_open_input_stream() {
        let (_sender, receiver) = mpsc::channel(1);
        let (context, _progress) = context();

        let output = run_with_guard(
            helper_spec("exit-23").with_stdin(ProcessStdin::Stream(receiver)),
            context,
        )
        .await
        .expect("child exit should end execution while the producer remains open");

        assert_eq!(output.exit_code, Some(23));
    }

    #[tokio::test]
    async fn preserves_non_utf8_output() {
        let (context, _progress) = context();
        let output = run_with_guard(helper_spec("non-utf8"), context)
            .await
            .expect("non-UTF-8 output should be retained as bytes");

        assert!(
            output
                .stdout
                .windows(4)
                .any(|bytes| bytes == [0x66, 0x80, 0xff, 0x0a])
        );
    }

    #[tokio::test]
    async fn drains_large_stdout_and_stderr_concurrently() {
        let (context, _progress) = context();
        let output = run_with_guard(
            helper_spec("large-both").with_output_limits(1024 * 1024, 1024 * 1024),
            context,
        )
        .await
        .expect("both large streams should drain without deadlock");

        assert!(output.stdout.iter().filter(|byte| **byte == b'o').count() >= 256 * 1024);
        assert!(output.stderr.iter().filter(|byte| **byte == b'e').count() >= 256 * 1024);
    }

    #[tokio::test]
    async fn returns_nonzero_exit_as_process_outcome() {
        let (context, _progress) = context();
        let output = run_with_guard(helper_spec("exit-23"), context)
            .await
            .expect("nonzero exit is not a spawn failure");

        assert!(!output.success);
        assert_eq!(output.exit_code, Some(23));
    }

    #[tokio::test]
    async fn classifies_spawn_failure_without_exposing_command() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let secret = "secret-executable-name";
        let (context, _progress) = context();
        let error = run_with_guard(ProcessSpec::new(temp.path().join(secret)), context)
            .await
            .expect_err("missing executable should fail to spawn");

        assert_eq!(error.kind, GitErrorKind::GitUnavailable);
        assert!(!format!("{error:?}").contains(secret));
    }

    #[tokio::test]
    async fn terminates_process_when_output_limit_is_exceeded() {
        let (context, _progress) = context();
        let error = run_with_guard(
            helper_spec("large-both").with_output_limits(4096, 4096),
            context,
        )
        .await
        .expect_err("oversized output should fail");

        assert_eq!(error.kind, GitErrorKind::OutputTooLarge);
    }

    #[tokio::test]
    async fn terminates_process_on_timeout() {
        let (context, _progress) = context();
        let error = run_with_guard(
            helper_spec("sleep").with_timeout(Duration::from_millis(100)),
            context,
        )
        .await
        .expect_err("sleeping helper should time out");

        assert_eq!(error.kind, GitErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn terminates_process_on_cancellation() {
        let (context, _progress) = context();
        let cancellation = context.cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancellation.cancel();
        });

        let error = run_with_guard(helper_spec("sleep"), context)
            .await
            .expect_err("cancelled helper should stop");

        assert_eq!(error.kind, GitErrorKind::Cancelled);
    }

    #[tokio::test]
    async fn timeout_terminates_nested_process_tree() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let marker = temp.path().join("descendant-marker");
        let (context, _progress) = context();
        let spec = helper_spec("descendant-parent")
            .with_environment([
                EnvironmentVariable::public(HELPER_MODE, "descendant-parent"),
                EnvironmentVariable::public(HELPER_MARKER, marker.as_os_str()),
            ])
            .with_timeout(Duration::from_millis(200));

        let error = run_with_guard(spec, context)
            .await
            .expect_err("parent helper should time out");
        assert_eq!(error.kind, GitErrorKind::TimedOut);

        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            !marker.exists(),
            "descendant must not survive process-tree termination"
        );
    }
}
