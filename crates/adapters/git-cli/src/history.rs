//! Machine-readable Git history reader.

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};
use async_trait::async_trait;
use git_domain::{
    CommitPage, CommitSummary, GitError, GitErrorKind, GitResult, LogQuery, ObjectId, SessionTarget,
};
use git_ports::{ExecutionContext, HistoryReader, ProcessRunner, RepositoryLocation};
use std::{ffi::OsString, path::PathBuf, str::FromStr};

#[derive(Debug)]
pub struct GitCliHistoryReader<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliHistoryReader<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> HistoryReader for GitCliHistoryReader<R> {
    #[allow(clippy::result_large_err)]
    async fn log(
        &self,
        target: SessionTarget,
        query: LogQuery,
        context: ExecutionContext,
    ) -> GitResult<CommitPage> {
        let cwd = self
            .location
            .worktree_root()
            .or(Some(self.location.git_dir()))
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::InvalidRepository,
                    "repository location is unavailable",
                )
            })?
            .to_path_buf();
        let output = self
            .executor
            .execute(
                LogCommand {
                    cwd,
                    query: query.clone(),
                },
                context,
            )
            .await?;
        if !output.success {
            return Err(
                GitError::new(GitErrorKind::InvalidRepository, "Git log query failed")
                    .with_diagnostic(format!("exit_code={:?}", output.exit_code)),
            );
        }
        let commits = parse_log(&output.stdout)?;
        Ok(CommitPage {
            target,
            has_more: commits.len() > query.limit,
            commits: commits.into_iter().take(query.limit).collect(),
        })
    }
}

#[allow(clippy::result_large_err, clippy::chunks_exact_to_as_chunks)]
fn parse_log(bytes: &[u8]) -> GitResult<Vec<CommitSummary>> {
    let fields: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut result = Vec::new();
    for chunk in fields.chunks_exact(5) {
        if chunk.iter().all(|field| field.is_empty()) {
            continue;
        }
        let id = ObjectId::from_str(std::str::from_utf8(trim_newline(chunk[0])).map_err(|_| {
            GitError::new(
                GitErrorKind::UnsupportedEncoding,
                "Git log OID is not UTF-8",
            )
        })?)
        .map_err(|error| {
            GitError::new(GitErrorKind::Internal, "Git log returned invalid OID")
                .with_diagnostic(error.to_string())
        })?;
        let parents = if trim_newline(chunk[1]).is_empty() {
            Vec::new()
        } else {
            trim_newline(chunk[1])
                .split(|byte| *byte == b' ')
                .map(|value| {
                    ObjectId::from_str(std::str::from_utf8(value).unwrap_or_default()).map_err(
                        |error| {
                            GitError::new(
                                GitErrorKind::Internal,
                                "Git log returned invalid parent OID",
                            )
                            .with_diagnostic(error.to_string())
                        },
                    )
                })
                .collect::<GitResult<Vec<_>>>()?
        };
        let timestamp = std::str::from_utf8(trim_newline(chunk[3]))
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| {
                GitError::new(GitErrorKind::Internal, "Git log returned invalid timestamp")
            })?;
        result.push(CommitSummary {
            id,
            parents,
            subject: trim_newline(chunk[4]).to_vec(),
            author: trim_newline(chunk[2]).to_vec(),
            timestamp,
        });
    }
    Ok(result)
}

fn trim_newline(value: &[u8]) -> &[u8] {
    value
        .strip_prefix(b"\n")
        .unwrap_or(value)
        .strip_suffix(b"\n")
        .unwrap_or_else(|| value.strip_prefix(b"\n").unwrap_or(value))
}

struct LogCommand {
    cwd: PathBuf,
    query: LogQuery,
}
impl GitCliCommand for LogCommand {}
impl sealed::Sealed for LogCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("log"),
                OsString::from("--no-decorate"),
                OsString::from("--format=%H%x00%P%x00%an%x00%at%x00%s%x00"),
                OsString::from(format!("--max-count={}", self.query.limit + 1)),
                OsString::from(format!("--skip={}", self.query.skip)),
            ],
        )
        .with_cwd(self.cwd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{OperationId, RepositoryId, SessionTarget, WorktreeId};
    use git_fixture::TestRepository;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn reads_real_history_with_parent_and_pagination() {
        let fixture = TestRepository::init().expect("fixture");
        fixture.commit_file("one.txt", b"one").expect("first");
        fixture.commit_file("two.txt", b"two").expect("second");
        let reader = GitCliHistoryReader::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        );
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let (progress, _receiver) = mpsc::channel(4);
        let page = reader
            .log(
                target,
                LogQuery { limit: 1, skip: 0 },
                ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress),
            )
            .await
            .expect("log");
        assert_eq!(page.commits.len(), 1);
        assert!(page.has_more);
        assert_eq!(page.commits[0].parents.len(), 1);
    }
}
