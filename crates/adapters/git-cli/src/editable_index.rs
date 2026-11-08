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
        if !is_editable_mode(request.mode) {
            return Err(GitError::new(
                GitErrorKind::UnsupportedFileKind,
                "editable Index candidate mode is not a supported file mode",
            ));
        }
        let path = repo_path_arg(&request.path)?;
        for key in ["core.splitIndex", "core.sparseCheckout", "index.sparse"] {
            let features = self
                .executor
                .execute(
                    IndexFeatureCheckCommand {
                        cwd: cwd.to_path_buf(),
                        key: OsString::from(key),
                    },
                    context.clone(),
                )
                .await?;
            if String::from_utf8_lossy(&features.stdout)
                .trim()
                .eq_ignore_ascii_case("true")
            {
                return Err(GitError::new(
                    GitErrorKind::UnsupportedFeature,
                    "editable Index Apply does not support sparse or split Index configuration",
                ));
            }
        }
        let index_bytes = fs::read(&request.temporary_index).map_err(|error| {
            GitError::new(GitErrorKind::PermissionDenied, "read candidate Index")
                .with_diagnostic(error.to_string())
        })?;
        if !index_extensions_are_supported(&index_bytes) {
            return Err(GitError::new(
                GitErrorKind::UnsupportedFeature,
                "editable Index Apply does not support this Index version or extension",
            ));
        }
        let conflict = self
            .executor
            .execute(
                ConflictStageCheckCommand {
                    cwd: cwd.to_path_buf(),
                    index: request.temporary_index.clone(),
                    path: path.clone(),
                },
                context.clone(),
            )
            .await?;
        if !conflict.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git could not inspect candidate Index conflict stages",
            ));
        }
        if !conflict.stdout.is_empty() {
            return Err(GitError::new(
                GitErrorKind::MergeConflict,
                "editable Index Apply does not support unresolved conflict stages",
            ));
        }
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

fn is_editable_mode(mode: u32) -> bool {
    matches!(mode, 0o100644 | 0o100755 | 0o120000)
}

fn index_extensions_are_supported(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[..4] != b"DIRC" {
        return false;
    }
    let version = u32::from_be_bytes(bytes[4..8].try_into().unwrap_or_default());
    if !matches!(version, 2 | 3) {
        return false;
    }
    let entries = u32::from_be_bytes(bytes[8..12].try_into().unwrap_or_default()) as usize;
    [20usize, 32usize]
        .iter()
        .any(|hash_len| parse_index_extensions(bytes, version, entries, *hash_len))
}

fn parse_index_extensions(bytes: &[u8], _version: u32, entries: usize, hash_len: usize) -> bool {
    let Some(extension_end) = bytes.len().checked_sub(hash_len) else {
        return false;
    };
    let fixed = 40usize.saturating_add(hash_len).saturating_add(2);
    let mut cursor = 12usize;
    for _ in 0..entries {
        let Some(path_start) = cursor.checked_add(fixed) else {
            return false;
        };
        if path_start > extension_end {
            return false;
        }
        let Some(nul_offset) = bytes[path_start..extension_end]
            .iter()
            .position(|byte| *byte == 0)
        else {
            return false;
        };
        let entry_end = path_start + nul_offset + 1;
        let padding = (8 - ((entry_end - cursor) % 8)) % 8;
        cursor = entry_end.saturating_add(padding);
        if cursor > extension_end {
            return false;
        }
    }
    while cursor < extension_end {
        if extension_end - cursor < 8 {
            return false;
        }
        let signature = &bytes[cursor..cursor + 4];
        let size = u32::from_be_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap_or_default())
            as usize;
        cursor += 8;
        let Some(next) = cursor.checked_add(size) else {
            return false;
        };
        if next > extension_end || signature != b"TREE" {
            return false;
        }
        cursor = next;
    }
    cursor == extension_end
}

struct IndexFeatureCheckCommand {
    cwd: PathBuf,
    key: OsString,
}
impl GitCliCommand for IndexFeatureCheckCommand {}
impl sealed::Sealed for IndexFeatureCheckCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("config"),
                OsString::from("--bool"),
                OsString::from("--get"),
                self.key,
            ],
        )
        .with_cwd(self.cwd)
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

struct ConflictStageCheckCommand {
    cwd: PathBuf,
    index: PathBuf,
    path: OsString,
}
impl GitCliCommand for ConflictStageCheckCommand {}
impl sealed::Sealed for ConflictStageCheckCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("ls-files"),
                OsString::from("--unmerged"),
                OsString::from("--"),
                self.path,
            ],
        )
        .with_cwd(self.cwd)
        .with_environment([EnvironmentVariable::public("GIT_INDEX_FILE", self.index)])
    }
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

    #[tokio::test]
    async fn rejects_editing_a_path_with_unresolved_conflict_stages() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .create_conflict("conflict.txt", b"base\n", b"ours\n", b"theirs\n")
            .expect("conflict fixture");
        let real_index = fixture.worktree.join(".git/index");
        let temporary_index = fixture.temp_dir.path().join("editable-conflict-index");
        fs::copy(&real_index, &temporary_index).expect("copy conflict Index");
        let builder = GitCliEditableIndexCandidateBuilder::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        );
        let error = builder
            .build(
                EditableIndexCandidateRequest {
                    target: SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                    path: RepoPath::try_from("conflict.txt").expect("path"),
                    mode: 0o100644,
                    content: b"resolved\n".to_vec(),
                    temporary_index,
                },
                context(),
            )
            .await
            .expect_err("unresolved stages must be rejected");
        assert_eq!(error.kind, GitErrorKind::MergeConflict);
    }

    #[tokio::test]
    async fn rejects_gitlink_and_unknown_editable_modes() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("file.txt", b"base\n")
            .expect("fixture commit");
        let temporary_index = fixture.temp_dir.path().join("editable-mode-index");
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
        for mode in [0o160000, 0o100666, 0o777] {
            let error = builder
                .build(
                    EditableIndexCandidateRequest {
                        target: SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                        path: RepoPath::try_from("file.txt").expect("path"),
                        mode,
                        content: b"edited\n".to_vec(),
                        temporary_index: temporary_index.clone(),
                    },
                    context(),
                )
                .await
                .expect_err("unsupported mode must be rejected");
            assert_eq!(error.kind, GitErrorKind::UnsupportedFileKind);
        }
    }

    #[tokio::test]
    async fn rejects_split_index_configuration_before_candidate_write() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("file.txt", b"base\n")
            .expect("fixture commit");
        let temporary_index = fixture.temp_dir.path().join("editable-split-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        assert!(read_tree.status().expect("git starts").success());
        let mut config = Command::new("git");
        config
            .args(["config", "core.splitIndex", "true"])
            .current_dir(&fixture.worktree);
        for (name, value) in fixture.isolated_environment() {
            config.env(name, value);
        }
        assert!(config.status().expect("git config starts").success());

        let builder = GitCliEditableIndexCandidateBuilder::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        );
        let error = builder
            .build(
                EditableIndexCandidateRequest {
                    target: SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                    path: RepoPath::try_from("file.txt").expect("path"),
                    mode: 0o100644,
                    content: b"edited\n".to_vec(),
                    temporary_index,
                },
                context(),
            )
            .await
            .expect_err("split Index configuration must be rejected");
        assert_eq!(error.kind, GitErrorKind::UnsupportedFeature);
    }

    #[tokio::test]
    async fn rejects_an_unknown_index_extension_before_candidate_write() {
        let fixture = TestRepository::init().expect("fixture");
        fixture
            .commit_file("file.txt", b"base\n")
            .expect("fixture commit");
        let temporary_index = fixture.temp_dir.path().join("editable-unknown-index");
        let mut read_tree = Command::new("git");
        read_tree
            .args(["read-tree", "HEAD"])
            .current_dir(&fixture.worktree)
            .env("GIT_INDEX_FILE", &temporary_index);
        for (name, value) in fixture.isolated_environment() {
            read_tree.env(name, value);
        }
        assert!(read_tree.status().expect("git starts").success());
        let mut bytes = fs::read(&temporary_index).expect("candidate Index");
        let extension = bytes
            .windows(4)
            .position(|value| value == b"TREE")
            .expect("fixture Index should contain TREE extension");
        bytes[extension..extension + 4].copy_from_slice(b"UNKN");
        fs::write(&temporary_index, bytes).expect("corrupt extension signature");

        let builder = GitCliEditableIndexCandidateBuilder::new(
            crate::TokioProcessRunner,
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            ),
        );
        let error = builder
            .build(
                EditableIndexCandidateRequest {
                    target: SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                    path: RepoPath::try_from("file.txt").expect("path"),
                    mode: 0o100644,
                    content: b"edited\n".to_vec(),
                    temporary_index,
                },
                context(),
            )
            .await
            .expect_err("unknown Index extension must be rejected");
        assert_eq!(error.kind, GitErrorKind::UnsupportedFeature);
    }
}
