//! Git CLI adapter for constructing a linear interactive rebase plan.

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};
use async_trait::async_trait;
use git_domain::{
    GitError, GitErrorKind, GitResult, InteractiveRebasePlan, ObjectId, RebaseAction,
    RebaseTodoEntry, RecoveryAction, SessionTarget,
};
use git_ports::{
    EnvironmentVariable, ExecutionContext, InteractiveRebasePlanReader, InteractiveRebaseWriter,
    ProcessRunner, RepositoryLocation,
};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Debug)]
pub struct GitCliInteractiveRebasePlanReader<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliInteractiveRebasePlanReader<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> InteractiveRebasePlanReader for GitCliInteractiveRebasePlanReader<R> {
    #[allow(clippy::result_large_err)]
    async fn plan(
        &self,
        target: SessionTarget,
        upstream: ObjectId,
        context: ExecutionContext,
    ) -> GitResult<InteractiveRebasePlan> {
        let cwd = self
            .location
            .worktree_root()
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "interactive rebase plan requires a worktree",
                )
                .with_target(target)
            })?
            .to_path_buf();
        let output = self
            .executor
            .execute(RebasePlanCommand { cwd, upstream }, context)
            .await?;
        if !output.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git rebase plan query failed",
            )
            .with_target(target)
            .with_diagnostic(format!("exit_code={:?}", output.exit_code)));
        }
        let entries = parse_plan(&output.stdout)?;
        let plan = InteractiveRebasePlan {
            target,
            upstream,
            entries,
        };
        plan.validate().map_err(|error| {
            GitError::new(
                GitErrorKind::Internal,
                "Git returned an invalid interactive rebase plan",
            )
            .with_target(target)
            .with_diagnostic(error.to_string())
        })?;
        Ok(plan)
    }
}

#[derive(Debug)]
pub struct GitCliInteractiveRebaseWriter<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliInteractiveRebaseWriter<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> InteractiveRebaseWriter for GitCliInteractiveRebaseWriter<R> {
    #[allow(clippy::result_large_err)]
    async fn apply(
        &self,
        target: SessionTarget,
        plan: InteractiveRebasePlan,
        context: ExecutionContext,
    ) -> GitResult<()> {
        plan.validate().map_err(|error| {
            GitError::new(GitErrorKind::Internal, "interactive rebase plan is invalid")
                .with_target(target)
                .with_operation(context.operation_id)
                .with_diagnostic(error.to_string())
        })?;
        let cwd = self
            .location
            .worktree_root()
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "interactive rebase requires a worktree",
                )
                .with_target(target)
            })?
            .to_path_buf();
        let workspace = EditorWorkspace::create(context.operation_id, &plan)?;
        let output = self
            .executor
            .execute(
                RebaseStartCommand {
                    cwd,
                    upstream: plan.upstream,
                    editor: shell_quote(workspace.editor.as_os_str()),
                },
                context.clone(),
            )
            .await?;
        workspace.remove();
        if output.success {
            Ok(())
        } else {
            Err(GitError::new(
                GitErrorKind::MergeConflict,
                "interactive rebase did not complete",
            )
            .with_target(target)
            .with_operation(context.operation_id)
            .with_recovery_actions([RecoveryAction::ContinueRebase, RecoveryAction::AbortRebase])
            .with_diagnostic(format!("exit_code={:?}", output.exit_code)))
        }
    }
}

struct EditorWorkspace {
    directory: PathBuf,
    editor: PathBuf,
}

impl EditorWorkspace {
    #[allow(clippy::result_large_err)]
    fn create(operation: git_domain::OperationId, plan: &InteractiveRebasePlan) -> GitResult<Self> {
        let directory = std::env::temp_dir().join(format!("ideagit-rebase-{operation}"));
        fs::create_dir(&directory).map_err(|error| {
            GitError::new(
                GitErrorKind::Internal,
                "could not create rebase editor workspace",
            )
            .with_diagnostic(error.to_string())
        })?;
        let todo = directory.join("todo");
        let write_result = File::create(&todo).and_then(|mut file| {
            file.write_all(&plan.render_todo().map_err(|error| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, error.to_string())
            })?)
        });
        if let Err(error) = write_result {
            let _ = fs::remove_dir_all(&directory);
            return Err(
                GitError::new(GitErrorKind::Internal, "could not write rebase todo")
                    .with_diagnostic(error.to_string()),
            );
        }
        let editor = directory.join(if cfg!(windows) {
            "editor.cmd"
        } else {
            "editor"
        });
        write_editor_script(&editor, &todo).map_err(|error| {
            let _ = fs::remove_dir_all(&directory);
            GitError::new(GitErrorKind::Internal, "could not create rebase editor")
                .with_diagnostic(error.to_string())
        })?;
        Ok(Self { directory, editor })
    }

    fn remove(self) {
        let _ = fs::remove_dir_all(self.directory);
    }
}

fn write_editor_script(editor: &Path, todo: &Path) -> std::io::Result<()> {
    let mut file = File::create(editor)?;
    #[cfg(windows)]
    {
        writeln!(file, "@echo off")?;
        writeln!(file, "copy /Y \"{}\" \"%~1\" >NUL", todo.display())?;
    }
    #[cfg(not(windows))]
    {
        writeln!(file, "#!/bin/sh")?;
        writeln!(file, "cp -- \"{}\" \"$1\"", todo.display())?;
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(editor, permissions)?;
    }
    Ok(())
}

fn shell_quote(path: &std::ffi::OsStr) -> OsString {
    #[cfg(windows)]
    {
        OsString::from(format!(
            "\"{}\"",
            path.to_string_lossy().replace('"', "\\\"")
        ))
    }
    #[cfg(not(windows))]
    {
        OsString::from(format!(
            "'{}'",
            path.to_string_lossy().replace('\'', "'\\''")
        ))
    }
}

struct RebasePlanCommand {
    cwd: PathBuf,
    upstream: ObjectId,
}

impl GitCliCommand for RebasePlanCommand {}

impl sealed::Sealed for RebasePlanCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("log"),
                OsString::from("--reverse"),
                OsString::from("--format=%H%x00%s"),
                OsString::from(format!("{}..HEAD", self.upstream)),
            ],
        )
        .with_cwd(self.cwd)
    }
}

struct RebaseStartCommand {
    cwd: PathBuf,
    upstream: ObjectId,
    editor: OsString,
}

impl GitCliCommand for RebaseStartCommand {}

impl sealed::Sealed for RebaseStartCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("rebase"),
                OsString::from("--interactive"),
                OsString::from(self.upstream.to_string()),
            ],
        )
        .with_cwd(self.cwd)
        .with_environment([EnvironmentVariable::public(
            "GIT_SEQUENCE_EDITOR",
            self.editor,
        )])
    }
}

#[allow(clippy::result_large_err)]
fn parse_plan(bytes: &[u8]) -> GitResult<Vec<RebaseTodoEntry>> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.splitn(2, |byte| *byte == 0);
            let id = fields
                .next()
                .ok_or_else(|| invalid_plan("missing commit"))?;
            let subject = fields
                .next()
                .ok_or_else(|| invalid_plan("missing subject"))?;
            let id = ObjectId::from_str(
                std::str::from_utf8(id).map_err(|_| invalid_plan("commit is not UTF-8"))?,
            )
            .map_err(|error| invalid_plan(&format!("invalid commit: {error}")))?;
            let subject = String::from_utf8(subject.to_vec())
                .map_err(|_| invalid_plan("subject is not UTF-8"))?;
            Ok(RebaseTodoEntry {
                commit: id,
                action: RebaseAction::Pick,
                message_override: Some(subject),
            })
        })
        .collect()
}

fn invalid_plan(message: &str) -> GitError {
    GitError::new(
        GitErrorKind::Internal,
        "Git returned an invalid interactive rebase plan",
    )
    .with_diagnostic(message)
}
