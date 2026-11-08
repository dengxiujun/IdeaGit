#![allow(clippy::result_large_err)]

use std::{ffi::OsString, path::PathBuf, str::FromStr};

use async_trait::async_trait;
use git_domain::{
    BlameLine, FileHistoryEntry, FileHistoryQuery, GitError, GitErrorKind, GitResult, ObjectId,
    RepoPath,
};
use git_ports::{
    BlameReader, ExecutionContext, FileHistoryReader, ProcessRunner, RepositoryLocation,
};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

#[derive(Debug)]
pub struct GitCliFileHistoryReader<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliFileHistoryReader<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> FileHistoryReader for GitCliFileHistoryReader<R> {
    #[allow(clippy::result_large_err)]
    async fn history(
        &self,
        query: FileHistoryQuery,
        context: ExecutionContext,
    ) -> GitResult<Vec<FileHistoryEntry>> {
        let cwd = self.cwd()?;
        let path = query.path.clone();
        let output = self
            .executor
            .execute(
                FileHistoryCommand {
                    cwd,
                    path: query.path,
                    limit: query.limit + 1,
                    skip: query.skip,
                },
                context,
            )
            .await?;
        if !output.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git file history query failed",
            ));
        }
        parse_history(&output.stdout, path)
    }
}

#[async_trait]
impl<R: ProcessRunner> BlameReader for GitCliFileHistoryReader<R> {
    #[allow(clippy::result_large_err)]
    async fn blame(
        &self,
        _target: git_domain::SessionTarget,
        path: RepoPath,
        context: ExecutionContext,
    ) -> GitResult<Vec<BlameLine>> {
        let output = self
            .executor
            .execute(
                BlameCommand {
                    cwd: self.cwd()?,
                    path,
                },
                context,
            )
            .await?;
        if !output.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git blame query failed",
            ));
        }
        parse_blame(&output.stdout)
    }
}

impl<R> GitCliFileHistoryReader<R> {
    fn cwd(&self) -> GitResult<PathBuf> {
        self.location
            .worktree_root()
            .or(Some(self.location.git_dir()))
            .map(PathBuf::from)
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::InvalidRepository,
                    "repository location is unavailable",
                )
            })
    }
}

#[allow(clippy::manual_is_multiple_of, clippy::chunks_exact_to_as_chunks)]
fn parse_history(bytes: &[u8], path: RepoPath) -> GitResult<Vec<FileHistoryEntry>> {
    let mut fields: Vec<&[u8]> = bytes.split(|b| *b == 0).map(trim_record_field).collect();
    if fields.last().is_some_and(|field| field.is_empty()) {
        fields.pop();
    }
    if fields.len() % 5 != 0 {
        return Err(GitError::new(
            GitErrorKind::Internal,
            "Git file history returned truncated records",
        ));
    }
    fields
        .chunks_exact(5)
        .map(|record| {
            let commit = parse_oid(record[0], "file history commit")?;
            let parent = if record[1].is_empty() {
                None
            } else {
                Some(parse_oid(record[1], "file history parent")?)
            };
            let timestamp = parse_i64(record[3], "file history timestamp")?;
            Ok(FileHistoryEntry {
                commit,
                parent,
                author: record[2].to_vec(),
                timestamp,
                subject: record[4].to_vec(),
                path: path.clone(),
            })
        })
        .collect()
}

fn trim_record_field(field: &[u8]) -> &[u8] {
    field.strip_prefix(b"\n").unwrap_or(field)
}

fn parse_blame(bytes: &[u8]) -> GitResult<Vec<BlameLine>> {
    let mut lines = Vec::new();
    let mut record: Option<(ObjectId, u32, u32, Vec<u8>, i64)> = None;
    for line in bytes.split(|b| *b == b'\n') {
        if line.starts_with(b"author ") {
            if let Some(r) = record.as_mut() {
                r.3 = line[7..].to_vec();
            }
        } else if line.starts_with(b"author-time ") {
            if let Some(r) = record.as_mut() {
                r.4 = parse_i64(&line[12..], "blame timestamp")?;
            }
        } else if line.first() == Some(&b'\t') {
            if let Some((commit, original, final_line, author, timestamp)) = record.take() {
                lines.push(BlameLine {
                    commit,
                    author,
                    timestamp,
                    original_line: original,
                    final_line,
                    content: line[1..].to_vec(),
                });
            }
        } else if !line.is_empty() {
            let fields: Vec<&[u8]> = line.split(|b| *b == b' ').collect();
            if fields.len() >= 3 && fields[0].len() >= 40 {
                record = Some((
                    parse_oid(fields[0], "blame commit")?,
                    parse_u32(fields[1], "blame original line")?,
                    parse_u32(fields[2], "blame final line")?,
                    Vec::new(),
                    0,
                ));
            }
        }
    }
    Ok(lines)
}

fn parse_oid(value: &[u8], context: &str) -> GitResult<ObjectId> {
    ObjectId::from_str(
        std::str::from_utf8(value)
            .map_err(|_| GitError::new(GitErrorKind::UnsupportedEncoding, context))?,
    )
    .map_err(|e| GitError::new(GitErrorKind::Internal, context).with_diagnostic(e.to_string()))
}
fn parse_i64(value: &[u8], context: &str) -> GitResult<i64> {
    std::str::from_utf8(value)
        .ok()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| GitError::new(GitErrorKind::Internal, context))
}
fn parse_u32(value: &[u8], context: &str) -> GitResult<u32> {
    std::str::from_utf8(value)
        .ok()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| GitError::new(GitErrorKind::Internal, context))
}

struct FileHistoryCommand {
    cwd: PathBuf,
    path: RepoPath,
    limit: usize,
    skip: usize,
}
impl GitCliCommand for FileHistoryCommand {}
impl sealed::Sealed for FileHistoryCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("log"),
                OsString::from("--follow"),
                OsString::from("--format=%H%x00%P%x00%an%x00%at%x00%s%x00"),
                OsString::from(format!("--max-count={}", self.limit)),
                OsString::from(format!("--skip={}", self.skip)),
                OsString::from("--"),
                OsString::from(self.path.display_lossy().as_ref()),
            ],
        )
        .with_cwd(self.cwd)
    }
}
struct BlameCommand {
    cwd: PathBuf,
    path: RepoPath,
}
impl GitCliCommand for BlameCommand {}
impl sealed::Sealed for BlameCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("blame"),
                OsString::from("--line-porcelain"),
                OsString::from("--"),
                OsString::from(self.path.display_lossy().as_ref()),
            ],
        )
        .with_cwd(self.cwd)
    }
}
