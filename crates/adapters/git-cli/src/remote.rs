//! Remote configuration and fetch commands implemented with typed Git invocations.

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};
use async_trait::async_trait;
use git_domain::{
    GitError, GitErrorKind, GitResult, ObjectId, PushMode, PushPreview, RefName, RemoteCommand,
    RemoteConfig, SessionTarget,
};
use git_ports::{
    ExecutionContext, ProcessRunner, PushPreviewReader, RemoteReader, RemoteWriter,
    RepositoryLocation,
};
use std::{ffi::OsString, path::PathBuf, str::FromStr};

#[derive(Debug)]
pub struct GitCliRemoteAdapter<R> {
    executor: GitCliExecutor<R>,
    location: RepositoryLocation,
}

impl<R> GitCliRemoteAdapter<R> {
    #[must_use]
    pub fn new(runner: R, location: RepositoryLocation) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            location,
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> RemoteReader for GitCliRemoteAdapter<R> {
    async fn list(
        &self,
        location: &RepositoryLocation,
        context: ExecutionContext,
    ) -> GitResult<Vec<RemoteConfig>> {
        let cwd = location
            .worktree_root()
            .unwrap_or(location.git_dir())
            .to_path_buf();
        let output = self
            .executor
            .execute(RemoteConfigCommand { cwd }, context)
            .await?;
        if !output.success {
            return Err(GitError::new(
                GitErrorKind::InvalidRepository,
                "Git remote configuration could not be read",
            ));
        }
        parse_remote_config(&output.stdout)
    }
}

#[async_trait]
impl<R: ProcessRunner> RemoteWriter for GitCliRemoteAdapter<R> {
    async fn apply(
        &self,
        _target: SessionTarget,
        command: RemoteCommand,
        context: ExecutionContext,
    ) -> GitResult<()> {
        let (remote, branch, force_with_lease, push) = match command {
            RemoteCommand::Fetch { remote } => (remote, None, false, false),
            RemoteCommand::Push {
                remote,
                branch,
                force_with_lease,
            } => (remote, branch, force_with_lease, true),
        };
        let cwd = self
            .location
            .worktree_root()
            .unwrap_or(self.location.git_dir())
            .to_path_buf();
        let output = self
            .executor
            .execute(
                RemoteMutation {
                    cwd,
                    remote,
                    branch,
                    force_with_lease,
                    push,
                },
                context,
            )
            .await?;
        if output.success {
            Ok(())
        } else {
            Err(GitError::new(
                GitErrorKind::NetworkFailure,
                "Git remote operation failed",
            ))
        }
    }
}

#[async_trait]
impl<R: ProcessRunner> PushPreviewReader for GitCliRemoteAdapter<R> {
    async fn preview_push(
        &self,
        _target: SessionTarget,
        remote: RefName,
        branch: RefName,
        mode: PushMode,
        context: ExecutionContext,
    ) -> GitResult<PushPreview> {
        let cwd = self
            .location
            .worktree_root()
            .unwrap_or(self.location.git_dir())
            .to_path_buf();
        let output = self
            .executor
            .execute(
                PushPreviewCommand {
                    cwd,
                    remote: remote.clone(),
                    branch: branch.clone(),
                    mode: mode.clone(),
                },
                context,
            )
            .await?;
        if !output.success {
            return Err(GitError::new(
                GitErrorKind::NetworkFailure,
                "Git push preview failed",
            ));
        }
        let expected_remote_oid = parse_push_preview_oid(&output.stdout)?;
        Ok(PushPreview {
            remote,
            source_ref: branch.clone(),
            destination_ref: branch,
            outgoing: Vec::new(),
            expected_remote_oid,
            mode,
        })
    }
}

#[allow(clippy::result_large_err)]
fn parse_remote_config(raw: &[u8]) -> GitResult<Vec<RemoteConfig>> {
    let records: Vec<&[u8]> = raw
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect();
    let mut remotes = Vec::new();
    for record in records {
        let Some(separator) = record.iter().position(|byte| *byte == b'\n') else {
            return Err(GitError::new(
                GitErrorKind::Internal,
                "Git remote configuration output was truncated",
            ));
        };
        let key = &record[..separator];
        let value = &record[separator + 1..];
        let Some(key) = key.strip_prefix(b"remote.") else {
            continue;
        };
        let (name, push_url) = if let Some(name) = key.strip_suffix(b".pushurl") {
            (name, true)
        } else if let Some(name) = key.strip_suffix(b".url") {
            (name, false)
        } else {
            continue;
        };
        let name = RefName::new(name.to_vec()).map_err(|_| {
            GitError::new(
                GitErrorKind::InvalidRepository,
                "Git reported an invalid remote name",
            )
        })?;
        let url = value.to_vec();
        if let Some(remote) = remotes
            .iter_mut()
            .find(|remote: &&mut RemoteConfig| remote.name == name)
        {
            if push_url {
                remote.push_url = url;
            } else if remote.fetch_url == remote.push_url {
                remote.fetch_url = url;
            }
        } else {
            remotes.push(RemoteConfig {
                name,
                fetch_url: if push_url { Vec::new() } else { url.clone() },
                push_url: if push_url { url } else { url.clone() },
            });
        }
    }
    Ok(remotes)
}

struct RemoteConfigCommand {
    cwd: PathBuf,
}
impl GitCliCommand for RemoteConfigCommand {}
impl sealed::Sealed for RemoteConfigCommand {
    fn into_invocation(self) -> GitCliInvocation {
        GitCliInvocation::new(
            None,
            [
                OsString::from("config"),
                OsString::from("--null"),
                OsString::from("--get-regexp"),
                OsString::from(r"^remote\..*\.url$"),
            ],
        )
        .with_cwd(self.cwd)
    }
}

struct RemoteMutation {
    cwd: PathBuf,
    remote: Option<RefName>,
    branch: Option<RefName>,
    force_with_lease: bool,
    push: bool,
}
impl GitCliCommand for RemoteMutation {}
impl sealed::Sealed for RemoteMutation {
    fn into_invocation(self) -> GitCliInvocation {
        let is_push = self.push;
        let mut args = if is_push {
            vec![OsString::from("push")]
        } else {
            vec![OsString::from("fetch"), OsString::from("--no-prune")]
        };
        if self.force_with_lease {
            args.push(OsString::from("--force-with-lease"));
        }
        match self.remote {
            Some(remote) => args.push(OsString::from(remote.display_lossy().into_owned())),
            None if is_push => {}
            None => args.push(OsString::from("--all")),
        }
        if let Some(branch) = self.branch {
            args.push(OsString::from(branch.display_lossy().into_owned()));
        }
        GitCliInvocation::new(None, args).with_cwd(self.cwd)
    }
}

struct PushPreviewCommand {
    cwd: PathBuf,
    remote: RefName,
    branch: RefName,
    mode: PushMode,
}
impl GitCliCommand for PushPreviewCommand {}
impl sealed::Sealed for PushPreviewCommand {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = vec![
            OsString::from("push"),
            OsString::from("--dry-run"),
            OsString::from("--porcelain"),
        ];
        if let PushMode::ForceWithLease { expected } = self.mode {
            args.push(OsString::from(format!(
                "--force-with-lease=refs/heads/{}:{}",
                self.branch.display_lossy(),
                expected
            )));
        }
        args.push(OsString::from(self.remote.display_lossy().into_owned()));
        args.push(OsString::from(self.branch.display_lossy().into_owned()));
        GitCliInvocation::new(None, args).with_cwd(self.cwd)
    }
}

#[allow(clippy::result_large_err)]
fn parse_push_preview_oid(raw: &[u8]) -> GitResult<Option<ObjectId>> {
    for line in raw.split(|byte| *byte == b'\n') {
        let Some(ref_start) = line
            .windows(b"refs/".len())
            .position(|window| window == b"refs/")
        else {
            continue;
        };
        let fields = &line[ref_start..];
        let Some(range) = fields
            .split(|byte| byte.is_ascii_whitespace())
            .find(|field| field.windows(2).any(|pair| pair == b".."))
        else {
            continue;
        };
        let Some(separator) = range.windows(2).position(|pair| pair == b"..") else {
            continue;
        };
        let old = &range[..separator];
        if old.is_empty() || old.iter().all(|byte| *byte == b'0') {
            return Ok(None);
        }
        let old = std::str::from_utf8(old).map_err(|_| {
            GitError::new(
                GitErrorKind::UnsupportedEncoding,
                "Git preview OID is not UTF-8",
            )
        })?;
        if old.len() != 40 && old.len() != 64 {
            return Ok(None);
        }
        return ObjectId::from_str(old).map(Some).map_err(|error| {
            GitError::new(GitErrorKind::Internal, "Git preview OID is invalid")
                .with_diagnostic(error.to_string())
        });
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GitCliRepositoryReader, TokioProcessRunner};
    use git_domain::{OperationId, RepositoryId, WorktreeId};
    use git_fixture::TestRepository;
    use git_ports::{PushPreviewReader, RemoteReader, RemoteWriter, RepositoryReader};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn parses_nul_delimited_remote_config_without_human_output() {
        let parsed = parse_remote_config(
            b"remote.origin.url\nhttps://example.invalid/a.git\0remote.origin.pushurl\nssh://example/a.git\0",
        )
        .expect("remote config should parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name.display_lossy(), "origin");
        assert_eq!(parsed[0].fetch_url, b"https://example.invalid/a.git");
        assert_eq!(parsed[0].push_url, b"ssh://example/a.git");
    }

    #[test]
    fn rejects_truncated_remote_config() {
        assert!(parse_remote_config(b"remote.origin.url\0").is_err());
    }

    #[tokio::test]
    async fn reads_and_fetches_a_real_local_remote() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture.commit_file("base.txt", b"base\n").expect("commit");
        let remote = fixture
            .create_local_remote()
            .expect("local remote should initialize");
        let (progress, _) = mpsc::channel(4);
        let context = ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress);
        let location = GitCliRepositoryReader::new(TokioProcessRunner)
            .discover(fixture.worktree.clone(), context.clone())
            .await
            .expect("repository should be discovered");
        let adapter = GitCliRemoteAdapter::new(TokioProcessRunner, location);
        let remotes = adapter
            .list(
                &RepositoryLocation::worktree(
                    &fixture.worktree,
                    fixture.worktree.join(".git"),
                    fixture.worktree.join(".git"),
                ),
                context.clone(),
            )
            .await
            .expect("remote config should be readable");
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].name.display_lossy(), "origin");
        assert!(
            remotes[0]
                .fetch_url
                .ends_with(remote.path().to_string_lossy().as_bytes())
        );
        adapter
            .apply(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                RemoteCommand::Fetch {
                    remote: Some(RefName::new(b"origin".to_vec()).expect("remote name")),
                },
                context,
            )
            .await
            .expect("fetch should succeed");
    }

    #[tokio::test]
    async fn pushes_a_real_local_commit_without_force() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture.commit_file("base.txt", b"base\n").expect("commit");
        let remote = fixture
            .create_local_remote()
            .expect("local remote should initialize");
        fixture
            .commit_file("next.txt", b"next\n")
            .expect("local commit should succeed");
        let (progress, _) = mpsc::channel(4);
        let context = ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress);
        let location = GitCliRepositoryReader::new(TokioProcessRunner)
            .discover(fixture.worktree.clone(), context.clone())
            .await
            .expect("repository should be discovered");
        let adapter = GitCliRemoteAdapter::new(TokioProcessRunner, location);
        adapter
            .apply(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                RemoteCommand::Push {
                    remote: Some(RefName::new(b"origin".to_vec()).expect("remote name")),
                    branch: Some(RefName::new(b"main".to_vec()).expect("branch name")),
                    force_with_lease: false,
                },
                context,
            )
            .await
            .expect("push should succeed");
        assert!(remote.path().is_dir());
    }

    #[tokio::test]
    async fn previews_a_real_push_before_execution() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture.commit_file("base.txt", b"base\n").expect("commit");
        fixture
            .create_local_remote()
            .expect("local remote should initialize");
        fixture
            .commit_file("next.txt", b"next\n")
            .expect("local commit should succeed");
        let (progress, _) = mpsc::channel(4);
        let context = ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress);
        let location = GitCliRepositoryReader::new(TokioProcessRunner)
            .discover(fixture.worktree.clone(), context.clone())
            .await
            .expect("repository should be discovered");
        let adapter = GitCliRemoteAdapter::new(TokioProcessRunner, location);
        let preview = adapter
            .preview_push(
                SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1),
                RefName::new(b"origin".to_vec()).expect("remote name"),
                RefName::new(b"main".to_vec()).expect("branch name"),
                PushMode::Normal,
                context,
            )
            .await
            .expect("preview should succeed");
        assert_eq!(preview.remote.display_lossy(), "origin");
        assert_eq!(preview.source_ref.display_lossy(), "main");
    }
}
