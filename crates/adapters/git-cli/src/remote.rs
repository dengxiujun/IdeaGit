//! Remote configuration and fetch commands implemented with typed Git invocations.

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};
use async_trait::async_trait;
use git_domain::{
    GitError, GitErrorKind, GitResult, RefName, RemoteCommand, RemoteConfig, SessionTarget,
};
use git_ports::{ExecutionContext, ProcessRunner, RemoteReader, RemoteWriter, RepositoryLocation};
use std::{ffi::OsString, path::PathBuf};

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
        let RemoteCommand::Fetch { remote } = command else {
            return Err(GitError::new(
                GitErrorKind::UnsupportedFeature,
                "Git remote writer does not support Push yet",
            ));
        };
        let cwd = self
            .location
            .worktree_root()
            .unwrap_or(self.location.git_dir())
            .to_path_buf();
        let output = self
            .executor
            .execute(RemoteMutation { cwd, remote }, context)
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
}
impl GitCliCommand for RemoteMutation {}
impl sealed::Sealed for RemoteMutation {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = vec![OsString::from("fetch"), OsString::from("--no-prune")];
        match self.remote {
            Some(remote) => args.push(OsString::from(remote.display_lossy().into_owned())),
            None => args.push(OsString::from("--all")),
        }
        GitCliInvocation::new(None, args).with_cwd(self.cwd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GitCliRepositoryReader, TokioProcessRunner};
    use git_domain::{OperationId, RepositoryId, WorktreeId};
    use git_fixture::TestRepository;
    use git_ports::{RemoteReader, RemoteWriter, RepositoryReader};
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
}
