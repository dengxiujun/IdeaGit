//! Git CLI construction of an editable candidate Index.

use std::{ffi::OsString, fs, path::PathBuf, str::FromStr};

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult, ObjectId};
use git_ports::{
    EditableIndexCandidate, EditableIndexCandidateBuilder, EditableIndexCandidateRequest,
    EnvironmentVariable, ExecutionContext, ProcessRunner, RepositoryLocation,
};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

#[derive(Debug)]
pub struct GitCliEditableIndexCandidateBuilder<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliEditableIndexCandidateBuilder<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> EditableIndexCandidateBuilder for GitCliEditableIndexCandidateBuilder<R> {
    #[allow(clippy::result_large_err)]
    async fn build(
        &self,
        request: EditableIndexCandidateRequest,
        context: ExecutionContext,
    ) -> GitResult<EditableIndexCandidate> {
        let cwd = self.location.worktree_root().ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "editable Index candidates require a worktree",
            )
        })?;
        if request.mode == 0 {
            return Err(GitError::new(
                GitErrorKind::UnsupportedFileKind,
                "editable Index candidate mode must be non-zero",
            ));
        }
        let path = repo_path_arg(&request.path)?;
        let blob = self
            .executor
            .execute(
                HashObjectCommand {
                    cwd: cwd.to_path_buf(),
                    path: path.clone(),
                    content: request.content.clone(),
                },
                context.clone(),
            )
            .await?;
        if !blob.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git could not create editable Blob",
            ));
        }
        let oid =
            ObjectId::from_str(String::from_utf8_lossy(&blob.stdout).trim()).map_err(|_| {
                GitError::new(GitErrorKind::Internal, "Git returned an invalid Blob OID")
            })?;
        let update = self
            .executor
            .execute(
                UpdateIndexCommand {
                    cwd: cwd.to_path_buf(),
                    index: request.temporary_index.clone(),
                    entry: cacheinfo_entry(request.mode, &oid, &path),
                },
                context.clone(),
            )
            .await?;
        if !update.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git could not update editable candidate Index",
            ));
        }
        let observed = self
            .executor
            .execute(
                VerifyIndexEntryCommand {
                    cwd: cwd.to_path_buf(),
                    index: request.temporary_index.clone(),
                    path: path.clone(),
                },
                context,
            )
            .await?;
        if !observed.success || !entry_matches(&observed.stdout, request.mode, &oid) {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "candidate Index entry does not match the edited Blob",
            ));
        }
        let index_bytes = fs::read(request.temporary_index).map_err(|error| {
            GitError::new(GitErrorKind::PermissionDenied, "read candidate Index")
                .with_diagnostic(error.to_string())
        })?;
        Ok(EditableIndexCandidate {
            index_bytes,
            content: request.content,
        })
    }
}

struct HashObjectCommand {
    cwd: PathBuf,
    path: OsString,
    content: Vec<u8>,
}
impl GitCliCommand for HashObjectCommand {}
impl sealed::Sealed for HashObjectCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("hash-object"),
                OsString::from("-w"),
                OsString::from("--filters"),
                OsString::from("--stdin"),
                {
                    let mut path = OsString::from("--path=");
                    path.push(self.path);
                    path
                },
            ],
        )
        .with_cwd(self.cwd)
        .with_stdin(git_ports::ProcessStdin::Bytes(self.content))
    }
}

struct UpdateIndexCommand {
    cwd: PathBuf,
    index: PathBuf,
    entry: OsString,
}
impl GitCliCommand for UpdateIndexCommand {}
impl sealed::Sealed for UpdateIndexCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("update-index"),
                OsString::from("--add"),
                OsString::from("--cacheinfo"),
                self.entry,
            ],
        )
        .with_cwd(self.cwd)
        .with_environment([EnvironmentVariable::public("GIT_INDEX_FILE", self.index)])
    }
}

struct VerifyIndexEntryCommand {
    cwd: PathBuf,
    index: PathBuf,
    path: OsString,
}
impl GitCliCommand for VerifyIndexEntryCommand {}
impl sealed::Sealed for VerifyIndexEntryCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("ls-files"),
                OsString::from("--stage"),
                OsString::from("--"),
                self.path,
            ],
        )
        .with_cwd(self.cwd)
        .with_environment([EnvironmentVariable::public("GIT_INDEX_FILE", self.index)])
    }
}

#[allow(clippy::result_large_err)]
fn repo_path_arg(path: &git_domain::RepoPath) -> GitResult<OsString> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(OsString::from_vec(path.as_bytes().to_vec()))
    }
    #[cfg(windows)]
    {
        String::from_utf8(path.as_bytes().to_vec())
            .map(OsString::from)
            .map_err(|_| {
                GitError::new(
                    GitErrorKind::UnsupportedEncoding,
                    "path is not representable on Windows",
                )
            })
    }
}

fn cacheinfo_entry(mode: u32, oid: &ObjectId, path: &OsString) -> OsString {
    let mut entry = OsString::from(format!("{:o},{},", mode, oid));
    entry.push(path);
    entry
}

fn entry_matches(output: &[u8], expected_mode: u32, expected_oid: &ObjectId) -> bool {
    let mut fields = output.splitn(3, |byte| *byte == b' ' || *byte == b'\t');
    let Some(mode) = fields
        .next()
        .and_then(|value| std::str::from_utf8(value).ok())
    else {
        return false;
    };
    let Some(oid) = fields
        .next()
        .and_then(|value| std::str::from_utf8(value).ok())
    else {
        return false;
    };
    mode == format!("{:o}", expected_mode) && oid == expected_oid.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{OperationId, RepoPath, RepositoryId, SessionTarget, WorktreeId};
    use git_fixture::TestRepository;
    use git_ports::{EditableIndexCandidateBuilder, ProcessProgress};
    use std::process::Command;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(4);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    #[tokio::test]
    async fn builds_candidate_index_without_touching_real_index() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("glob[1].txt", b"base\n")
            .expect("commit");
        let real_index = fixture.worktree.join(".git/index");
        let original = fs::read(&real_index).expect("real index");
        let temporary_index = fixture.temp_dir.path().join("editable-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        assert!(read_tree.status().expect("git starts").success());

        let builder = GitCliEditableIndexCandidateBuilder::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        );
        let candidate = builder
            .build(
                EditableIndexCandidateRequest {
                    target: SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                    path: RepoPath::try_from("glob[1].txt").expect("path"),
                    mode: 0o100755,
                    content: b"edited\n".to_vec(),
                    temporary_index,
                },
                context(),
            )
            .await
            .expect("candidate");
        assert_eq!(candidate.content, b"edited\n");
        assert_ne!(candidate.index_bytes, original);
        assert_eq!(fs::read(real_index).expect("real index"), original);
    }

    #[tokio::test]
    async fn applies_clean_filter_before_writing_candidate_blob() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file(".gitattributes", b"*.txt text eol=lf\n")
            .expect("attributes commit");
        fixture
            .commit_file("filtered.txt", b"base\n")
            .expect("file commit");
        let temporary_index = fixture.temp_dir.path().join("editable-filter-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        assert!(read_tree.status().expect("git starts").success());

        let builder = GitCliEditableIndexCandidateBuilder::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        );
        builder
            .build(
                EditableIndexCandidateRequest {
                    target: SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                    path: RepoPath::try_from("filtered.txt").expect("path"),
                    mode: 0o100644,
                    content: b"edited\r\n".to_vec(),
                    temporary_index: temporary_index.clone(),
                },
                context(),
            )
            .await
            .expect("filtered candidate");

        let mut read_entry = Command::new("git");
        read_entry
            .args(["ls-files", "--stage", "--", "filtered.txt"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_entry.env(name, value);
        }
        let entry = read_entry.output().expect("read candidate entry");
        assert!(entry.status.success());
        let oid = String::from_utf8_lossy(&entry.stdout)
            .split_whitespace()
            .nth(1)
            .expect("candidate blob oid")
            .to_owned();
        let output = Command::new("git")
            .args(["cat-file", "blob", &oid])
            .current_dir(&fixture.worktree)
            .output()
            .expect("read candidate blob");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"edited\n");
    }
}
