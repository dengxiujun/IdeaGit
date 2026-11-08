//! Git CLI adapter for constructing a linear interactive rebase plan.

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};
use async_trait::async_trait;
use git_domain::{
    GitError, GitErrorKind, GitResult, InteractiveRebasePlan, ObjectId, RebaseAction,
    RebaseTodoEntry, SessionTarget,
};
use git_ports::{ExecutionContext, InteractiveRebasePlanReader, ProcessRunner, RepositoryLocation};
use std::{ffi::OsString, path::PathBuf, str::FromStr};

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
