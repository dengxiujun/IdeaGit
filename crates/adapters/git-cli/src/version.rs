//! Git CLI three-version reader used by conflict resolution.

use std::{ffi::OsString, fs, path::PathBuf};

use async_trait::async_trait;
use git_domain::{FileKind, FileMode, GitError, GitErrorKind, GitResult, ObjectId, RepoPath};
use git_ports::{
    ExecutionContext, FileVersions, ProcessRunner, RepositoryLocation, VersionDescriptor,
    VersionReader,
};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

#[derive(Debug)]
pub struct GitCliVersionReader<R> {
    executor: GitCliExecutor<R>,
}

impl<R> GitCliVersionReader<R> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> VersionReader for GitCliVersionReader<R> {
    #[allow(clippy::result_large_err)]
    async fn read(
        &self,
        location: &RepositoryLocation,
        path: &RepoPath,
        context: ExecutionContext,
    ) -> GitResult<FileVersions> {
        let path = path_utf8(path)?;
        let cwd = location
            .worktree_root()
            .unwrap_or(location.git_dir())
            .to_path_buf();
        let index = self
            .executor
            .execute(
                IndexVersionCommand {
                    cwd: cwd.clone(),
                    path: path.clone(),
                },
                context.clone(),
            )
            .await?;
        let head = self
            .executor
            .execute(
                HeadVersionCommand {
                    cwd: cwd.clone(),
                    path: path.clone(),
                },
                context,
            )
            .await?;
        let worktree = if let Some(root) = location.worktree_root() {
            let full = root.join(&path);
            fs::symlink_metadata(&full).ok().and_then(|metadata| {
                let kind = if metadata.file_type().is_symlink() {
                    FileKind::Symlink
                } else {
                    FileKind::Regular
                };
                Some(VersionDescriptor {
                    kind,
                    oid: None,
                    mode: Some(worktree_mode(&metadata)),
                    byte_length: metadata.is_file().then(|| metadata.len()),
                })
            })
        } else {
            None
        };
        Ok(FileVersions {
            head: parse_tree(&head.stdout)?,
            index: parse_index(&index.stdout)?,
            worktree,
        })
    }
}

#[cfg(unix)]
fn worktree_mode(metadata: &fs::Metadata) -> FileMode {
    use std::os::unix::fs::PermissionsExt;
    FileMode::new(metadata.permissions().mode() & 0o177777)
}

#[cfg(not(unix))]
fn worktree_mode(_metadata: &fs::Metadata) -> FileMode {
    FileMode::new(0o100644)
}

fn path_utf8(path: &RepoPath) -> GitResult<String> {
    std::str::from_utf8(path.as_bytes())
        .map(str::to_owned)
        .map_err(|_| {
            GitError::new(
                GitErrorKind::UnsupportedEncoding,
                "conflict path is not representable by this Git CLI reader",
            )
        })
}

fn parse_index(raw: &[u8]) -> GitResult<Option<VersionDescriptor>> {
    let Some(record) = raw
        .split(|byte| *byte == 0)
        .find(|record| !record.is_empty())
    else {
        return Ok(None);
    };
    let tab = record
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or_else(|| invalid_output("index record has no path separator"))?;
    let fields: Vec<&[u8]> = record[..tab].split(|byte| *byte == b' ').collect();
    if fields.len() != 3 {
        return Err(invalid_output("index record has invalid field count"));
    }
    let mode = parse_mode(fields[0])?;
    let oid = parse_oid(fields[1])?;
    Ok(Some(VersionDescriptor {
        kind: kind_for_mode(mode),
        oid: Some(oid),
        mode: Some(mode),
        byte_length: None,
    }))
}

fn parse_tree(raw: &[u8]) -> GitResult<Option<VersionDescriptor>> {
    let Some(record) = raw
        .split(|byte| *byte == 0)
        .find(|record| !record.is_empty())
    else {
        return Ok(None);
    };
    let tab = record
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or_else(|| invalid_output("tree record has no path separator"))?;
    let fields: Vec<&[u8]> = record[..tab].split(|byte| *byte == b' ').collect();
    if fields.len() != 3 {
        return Err(invalid_output("tree record has invalid field count"));
    }
    let mode = parse_mode(fields[0])?;
    let oid = parse_oid(fields[2])?;
    Ok(Some(VersionDescriptor {
        kind: kind_for_mode(mode),
        oid: Some(oid),
        mode: Some(mode),
        byte_length: None,
    }))
}

fn parse_mode(raw: &[u8]) -> GitResult<FileMode> {
    let text = std::str::from_utf8(raw).map_err(|_| invalid_output("file mode is not UTF-8"))?;
    u32::from_str_radix(text, 8)
        .map(FileMode::new)
        .map_err(|_| invalid_output("file mode is invalid"))
}

fn parse_oid(raw: &[u8]) -> GitResult<ObjectId> {
    let text = std::str::from_utf8(raw).map_err(|_| invalid_output("object id is not UTF-8"))?;
    text.parse()
        .map_err(|_| invalid_output("object id is invalid"))
}

fn kind_for_mode(mode: FileMode) -> FileKind {
    match mode.get() {
        0o120000 => FileKind::Symlink,
        0o160000 => FileKind::Submodule,
        _ => FileKind::Regular,
    }
}

fn invalid_output(message: &'static str) -> GitError {
    GitError::new(GitErrorKind::Internal, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GitCliRepositoryReader, TokioProcessRunner};
    use git_domain::OperationId;
    use git_fixture::TestRepository;
    use git_ports::{EnvironmentVariable, ProcessProgress, RepositoryReader};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(32);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn environment(fixture: &TestRepository) -> Vec<EnvironmentVariable> {
        fixture
            .isolated_environment()
            .into_iter()
            .map(|(name, value)| EnvironmentVariable::public(name, value))
            .collect()
    }

    #[tokio::test]
    async fn reads_real_head_index_and_worktree_versions() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("conflict.txt", b"base\n")
            .expect("fixture commit should succeed");
        fixture
            .write_file("conflict.txt", b"edited\n")
            .expect("worktree edit should succeed");
        let location = GitCliRepositoryReader::new(TokioProcessRunner)
            .with_environment(environment(&fixture))
            .discover(fixture.worktree.clone(), context())
            .await
            .expect("repository should be discovered");
        let reader = GitCliVersionReader::new(TokioProcessRunner);
        let versions = reader
            .read(
                &location,
                &RepoPath::try_from("conflict.txt").expect("valid path"),
                context(),
            )
            .await
            .expect("versions should be readable");

        assert!(
            versions
                .head
                .as_ref()
                .and_then(|version| version.oid)
                .is_some()
        );
        assert!(
            versions
                .index
                .as_ref()
                .and_then(|version| version.oid)
                .is_some()
        );
        assert_eq!(
            versions
                .worktree
                .as_ref()
                .and_then(|version| version.byte_length),
            Some(7)
        );
    }
}

struct IndexVersionCommand {
    cwd: PathBuf,
    path: String,
}
struct HeadVersionCommand {
    cwd: PathBuf,
    path: String,
}

impl GitCliCommand for IndexVersionCommand {}
impl GitCliCommand for HeadVersionCommand {}

impl sealed::Sealed for IndexVersionCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("ls-files"),
                OsString::from("--stage"),
                OsString::from("-z"),
                OsString::from("--"),
                OsString::from(self.path),
            ],
        )
        .with_cwd(self.cwd)
    }
}
impl sealed::Sealed for HeadVersionCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("ls-tree"),
                OsString::from("-z"),
                OsString::from("HEAD"),
                OsString::from("--"),
                OsString::from(self.path),
            ],
        )
        .with_cwd(self.cwd)
    }
}
