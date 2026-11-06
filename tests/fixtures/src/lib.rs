//! Real-Git test fixtures for IdeaGit integration and contract tests.
//!
//! Every repository created here owns an isolated HOME, global Git config,
//! hook directory, credential configuration, signing setting, and local bare
//! remote. The fixture never reads or writes the user's repository, global
//! configuration, credential helper, or a network remote.

use std::{
    ffi::OsString,
    fs, io,
    path::{Component, Path, PathBuf},
    process::{Command, ExitStatus, Output},
};

use tempfile::TempDir;
use thiserror::Error;

/// Errors returned while constructing or operating a real Git fixture.
#[derive(Debug, Error)]
pub enum TestError {
    /// A fixture path would escape its temporary worktree.
    #[error("fixture path must be a non-empty relative path without parent components: {path}")]
    UnsafePath { path: PathBuf },

    /// A hook name is not a single file name.
    #[error("hook name must be a single file name: {name}")]
    InvalidHookName { name: String },

    /// An operation requires an existing commit but HEAD is unborn.
    #[error("{operation} requires a repository with at least one commit")]
    UnbornHead { operation: &'static str },

    /// Git could not be started.
    #[error("failed to start git while {operation}: {source}")]
    GitSpawn {
        operation: String,
        #[source]
        source: io::Error,
    },

    /// Git completed with an unsuccessful status.
    #[error("git failed while {operation} ({status}): {stderr}")]
    GitFailure {
        operation: String,
        status: ExitStatus,
        stderr: String,
    },

    /// A filesystem operation within the fixture failed.
    #[error("failed to {operation}: {source}")]
    Io {
        operation: String,
        #[source]
        source: io::Error,
    },

    /// Git completed a merge when the fixture expected a conflict.
    #[error("expected a merge conflict, but the merge completed successfully")]
    ExpectedConflict,

    /// Git reported a failed merge but did not leave an unmerged path.
    #[error("git did not report an unmerged path after the expected conflict")]
    MissingConflictState,
}

/// A local bare repository that belongs to one [`TestRepository`].
#[derive(Debug, Clone)]
pub struct LocalBareRemote {
    path: PathBuf,
}

impl LocalBareRemote {
    /// Returns the filesystem path of the fixture-owned bare repository.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// A temporary worktree together with fully isolated Git process settings.
///
/// The public fields intentionally match the TEST-001 baseline API. All Git
/// commands created by this type receive the fixture-only environment.
pub struct TestRepository {
    /// Owns all fixture files and removes them when dropped.
    pub temp_dir: TempDir,
    /// Root directory of the initialized non-bare worktree.
    pub worktree: PathBuf,
    home: PathBuf,
    global_config: PathBuf,
    hooks_dir: PathBuf,
}

impl TestRepository {
    /// Initializes an empty real Git repository with isolated process state.
    pub fn init() -> Result<Self, TestError> {
        let temp_dir = TempDir::new().map_err(|source| TestError::Io {
            operation: "create fixture temporary directory".to_owned(),
            source,
        })?;
        let root = temp_dir.path().to_path_buf();
        let home = root.join("home");
        let hooks_dir = root.join("hooks");
        let worktree = root.join("worktree");
        let global_config = root.join("global.gitconfig");

        for (path, operation) in [
            (&home, "create isolated fixture HOME"),
            (&hooks_dir, "create isolated fixture hooks directory"),
        ] {
            fs::create_dir_all(path).map_err(|source| TestError::Io {
                operation: operation.to_owned(),
                source,
            })?;
        }

        let repository = Self {
            temp_dir,
            worktree,
            global_config,
            home,
            hooks_dir,
        };

        repository.run_git_at(
            &root,
            "initialize fixture repository",
            vec![
                OsString::from("init"),
                repository.worktree.as_os_str().to_owned(),
            ],
        )?;
        repository.configure_isolation()?;
        Ok(repository)
    }

    /// Writes `data`, stages only `path`, and creates a fixture commit.
    pub fn commit_file(&self, path: &str, data: &[u8]) -> Result<(), TestError> {
        self.commit_file_with_message(path, data, "fixture commit")
    }

    /// Returns whether the repository currently has no commit at HEAD.
    pub fn is_unborn(&self) -> Result<bool, TestError> {
        Ok(self.head_oid()?.is_none())
    }

    /// Returns the current HEAD object ID, or `None` for an unborn HEAD.
    pub fn head_oid(&self) -> Result<Option<String>, TestError> {
        let output = self.git_output_at(
            &self.worktree,
            "inspect HEAD",
            vec![
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("HEAD"),
            ],
        )?;
        if !output.status.success() {
            return Ok(None);
        }

        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ))
    }

    /// Creates a fixture-owned bare `origin` and pushes the current branch to it.
    pub fn create_local_remote(&self) -> Result<LocalBareRemote, TestError> {
        if self.head_oid()?.is_none() {
            return Err(TestError::UnbornHead {
                operation: "create a local remote",
            });
        }

        let remote = self.temp_dir.path().join("origin.git");
        self.run_git_at(
            self.temp_dir.path(),
            "initialize local bare remote",
            vec![
                OsString::from("init"),
                OsString::from("--bare"),
                remote.as_os_str().to_owned(),
            ],
        )?;
        self.run_git_at(
            &self.worktree,
            "add local bare remote",
            vec![
                OsString::from("remote"),
                OsString::from("add"),
                OsString::from("origin"),
                remote.as_os_str().to_owned(),
            ],
        )?;
        self.run_git_at(
            &self.worktree,
            "push initial branch to local bare remote",
            vec![
                OsString::from("push"),
                OsString::from("-u"),
                OsString::from("origin"),
                OsString::from("HEAD"),
            ],
        )?;

        let branch = self.current_branch()?;
        self.run_git_at(
            self.temp_dir.path(),
            "set local bare remote HEAD",
            vec![
                OsString::from("--git-dir"),
                remote.as_os_str().to_owned(),
                OsString::from("symbolic-ref"),
                OsString::from("HEAD"),
                OsString::from(format!("refs/heads/{branch}")),
            ],
        )?;

        Ok(LocalBareRemote { path: remote })
    }

    /// Commits a change through a peer clone, pushes it to `remote`, then pulls it.
    ///
    /// This is a real local Push/Pull round trip; it never contacts a network remote.
    pub fn pull_change_from_peer(
        &self,
        remote: &LocalBareRemote,
        path: &str,
        data: &[u8],
    ) -> Result<(), TestError> {
        let peer = self.temp_dir.path().join("peer-worktree");
        self.run_git_at(
            self.temp_dir.path(),
            "clone local bare remote into peer worktree",
            vec![
                OsString::from("clone"),
                remote.path.as_os_str().to_owned(),
                peer.as_os_str().to_owned(),
            ],
        )?;

        self.write_file_at(&peer, path, data)?;
        self.run_git_at(
            &peer,
            "stage peer change",
            vec![
                OsString::from("add"),
                OsString::from("--"),
                OsString::from(path),
            ],
        )?;
        self.run_git_at(
            &peer,
            "commit peer change",
            vec![
                OsString::from("commit"),
                OsString::from("-m"),
                OsString::from("peer fixture commit"),
            ],
        )?;
        self.run_git_at(
            &peer,
            "push peer change to local bare remote",
            vec![
                OsString::from("push"),
                OsString::from("origin"),
                OsString::from("HEAD"),
            ],
        )?;
        self.run_git_at(
            &self.worktree,
            "pull peer change from local bare remote",
            vec![OsString::from("pull"), OsString::from("--ff-only")],
        )?;
        Ok(())
    }

    /// Produces a real unresolved merge conflict for `path`.
    pub fn create_conflict(
        &self,
        path: &str,
        base: &[u8],
        current: &[u8],
        other: &[u8],
    ) -> Result<(), TestError> {
        self.commit_file_with_message(path, base, "conflict base")?;
        let primary_branch = self.current_branch()?;

        self.run_git_at(
            &self.worktree,
            "create conflicting branch",
            vec![
                OsString::from("checkout"),
                OsString::from("-b"),
                OsString::from("fixture-conflict-other"),
            ],
        )?;
        self.commit_file_with_message(path, other, "conflict other side")?;
        self.run_git_at(
            &self.worktree,
            "return to primary conflict branch",
            vec![OsString::from("checkout"), OsString::from(primary_branch)],
        )?;
        self.commit_file_with_message(path, current, "conflict current side")?;

        let merge = self.git_output_at(
            &self.worktree,
            "merge conflicting branch",
            vec![
                OsString::from("merge"),
                OsString::from("fixture-conflict-other"),
            ],
        )?;
        if merge.status.success() {
            return Err(TestError::ExpectedConflict);
        }

        let status = self.run_git_at(
            &self.worktree,
            "inspect merge conflict status",
            vec![OsString::from("status"), OsString::from("--porcelain")],
        )?;
        if !String::from_utf8_lossy(&status.stdout).starts_with("UU ") {
            return Err(TestError::MissingConflictState);
        }
        Ok(())
    }

    /// Adds a linked worktree on a fresh fixture branch and returns its path.
    pub fn add_linked_worktree(&self) -> Result<PathBuf, TestError> {
        if self.head_oid()?.is_none() {
            return Err(TestError::UnbornHead {
                operation: "add a linked worktree",
            });
        }

        let linked = self.temp_dir.path().join("linked-worktree");
        self.run_git_at(
            &self.worktree,
            "add linked worktree",
            vec![
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("-b"),
                OsString::from("fixture-linked"),
                linked.as_os_str().to_owned(),
            ],
        )?;
        Ok(linked)
    }

    /// Installs a hook in the fixture-only hook directory.
    pub fn install_hook(&self, name: &str, contents: &[u8]) -> Result<PathBuf, TestError> {
        if !is_single_file_name(name) {
            return Err(TestError::InvalidHookName {
                name: name.to_owned(),
            });
        }

        let path = self.hooks_dir.join(name);
        fs::write(&path, contents).map_err(|source| TestError::Io {
            operation: format!("write fixture hook {}", path.display()),
            source,
        })?;
        set_hook_executable(&path)?;
        Ok(path)
    }

    fn configure_isolation(&self) -> Result<(), TestError> {
        for (key, value) in [
            ("user.name", "IdeaGit Fixture"),
            ("user.email", "fixture@ideagit.invalid"),
            ("core.hooksPath", self.hooks_dir.to_string_lossy().as_ref()),
            ("credential.helper", ""),
            ("commit.gpgSign", "false"),
        ] {
            self.run_git_at(
                &self.worktree,
                "configure isolated global Git setting",
                vec![
                    OsString::from("config"),
                    OsString::from("--global"),
                    OsString::from(key),
                    OsString::from(value),
                ],
            )?;
        }
        Ok(())
    }

    fn commit_file_with_message(
        &self,
        path: &str,
        data: &[u8],
        message: &str,
    ) -> Result<(), TestError> {
        self.write_file_at(&self.worktree, path, data)?;
        self.run_git_at(
            &self.worktree,
            "stage fixture file",
            vec![
                OsString::from("add"),
                OsString::from("--"),
                OsString::from(path),
            ],
        )?;
        self.run_git_at(
            &self.worktree,
            "commit fixture file",
            vec![
                OsString::from("commit"),
                OsString::from("-m"),
                OsString::from(message),
            ],
        )?;
        Ok(())
    }

    fn write_file_at(&self, worktree: &Path, path: &str, data: &[u8]) -> Result<(), TestError> {
        let relative = validate_relative_path(Path::new(path))?;
        let target = worktree.join(relative);
        let parent = target.parent().ok_or_else(|| TestError::UnsafePath {
            path: PathBuf::from(path),
        })?;
        fs::create_dir_all(parent).map_err(|source| TestError::Io {
            operation: format!("create fixture parent directory {}", parent.display()),
            source,
        })?;
        fs::write(&target, data).map_err(|source| TestError::Io {
            operation: format!("write fixture file {}", target.display()),
            source,
        })
    }

    fn current_branch(&self) -> Result<String, TestError> {
        let output = self.run_git_at(
            &self.worktree,
            "inspect current branch",
            vec![
                OsString::from("symbolic-ref"),
                OsString::from("--quiet"),
                OsString::from("--short"),
                OsString::from("HEAD"),
            ],
        )?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn run_git_at(
        &self,
        working_dir: &Path,
        operation: &str,
        arguments: Vec<OsString>,
    ) -> Result<Output, TestError> {
        let output = self.git_output_at(working_dir, operation, arguments)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(TestError::GitFailure {
                operation: operation.to_owned(),
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    fn git_output_at(
        &self,
        working_dir: &Path,
        operation: &str,
        arguments: Vec<OsString>,
    ) -> Result<Output, TestError> {
        let mut command = Command::new("git");
        command
            .args(arguments)
            .current_dir(working_dir)
            .env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home);
        command.output().map_err(|source| TestError::GitSpawn {
            operation: operation.to_owned(),
            source,
        })
    }
}

fn validate_relative_path(path: &Path) -> Result<&Path, TestError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(TestError::UnsafePath {
            path: path.to_path_buf(),
        });
    }
    Ok(path)
}

fn is_single_file_name(name: &str) -> bool {
    matches!(
        Path::new(name).components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(_)]
    )
}

#[cfg(unix)]
fn set_hook_executable(path: &Path) -> Result<(), TestError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|source| TestError::Io {
            operation: format!("read fixture hook permissions {}", path.display()),
            source,
        })?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|source| TestError::Io {
        operation: format!("mark fixture hook executable {}", path.display()),
        source,
    })
}

#[cfg(not(unix))]
fn set_hook_executable(_path: &Path) -> Result<(), TestError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::TestRepository;

    #[test]
    fn initializes_an_unborn_repository_with_isolated_configuration() {
        let repository = TestRepository::init().expect("fixture initialization should succeed");

        assert!(
            repository
                .is_unborn()
                .expect("HEAD inspection should succeed")
        );
        assert!(
            repository
                .global_config
                .starts_with(repository.temp_dir.path())
        );
        assert!(repository.home.starts_with(repository.temp_dir.path()));
        assert!(repository.hooks_dir.starts_with(repository.temp_dir.path()));

        let name = repository
            .run_git_at(
                &repository.worktree,
                "read isolated user name",
                vec![
                    "config".into(),
                    "--global".into(),
                    "--get".into(),
                    "user.name".into(),
                ],
            )
            .expect("isolated global config should be readable");
        assert_eq!(
            String::from_utf8_lossy(&name.stdout).trim(),
            "IdeaGit Fixture"
        );

        let signing = repository
            .run_git_at(
                &repository.worktree,
                "read isolated signing setting",
                vec![
                    "config".into(),
                    "--global".into(),
                    "--get".into(),
                    "commit.gpgSign".into(),
                ],
            )
            .expect("isolated signing setting should be readable");
        assert_eq!(String::from_utf8_lossy(&signing.stdout).trim(), "false");

        let credential_helper = repository
            .run_git_at(
                &repository.worktree,
                "read isolated credential helper",
                vec![
                    "config".into(),
                    "--global".into(),
                    "--get".into(),
                    "credential.helper".into(),
                ],
            )
            .expect("isolated credential helper should be readable");
        assert!(
            String::from_utf8_lossy(&credential_helper.stdout)
                .trim()
                .is_empty()
        );
    }

    #[test]
    fn commits_special_paths_without_leaving_the_fixture() {
        let repository = TestRepository::init().expect("fixture initialization should succeed");
        #[cfg(windows)]
        let path = "directory with spaces/brackets[and]#hash.txt";
        #[cfg(not(windows))]
        let path = "directory with spaces/line\nfeed.txt";

        repository
            .commit_file(path, b"fixture content\n")
            .expect("fixture commit should succeed");

        assert!(
            !repository
                .is_unborn()
                .expect("HEAD inspection should succeed")
        );
        assert_eq!(
            fs::read(repository.worktree.join(path)).expect("fixture file should exist"),
            b"fixture content\n"
        );
    }

    #[test]
    fn simulates_push_and_pull_against_a_local_bare_remote() {
        let repository = TestRepository::init().expect("fixture initialization should succeed");
        repository
            .commit_file("README.md", b"initial\n")
            .expect("initial fixture commit should succeed");
        let remote = repository
            .create_local_remote()
            .expect("local bare remote should be created");

        repository
            .pull_change_from_peer(&remote, "from-peer.txt", b"peer change\n")
            .expect("local push/pull round trip should succeed");

        assert!(remote.path().is_dir());
        assert_eq!(
            fs::read(repository.worktree.join("from-peer.txt"))
                .expect("pulled peer file should exist"),
            b"peer change\n"
        );
    }

    #[test]
    fn creates_a_real_merge_conflict() {
        let repository = TestRepository::init().expect("fixture initialization should succeed");

        repository
            .create_conflict(
                "conflict.txt",
                b"base\n",
                b"current branch\n",
                b"other branch\n",
            )
            .expect("fixture should leave an unresolved merge conflict");

        let status = repository
            .run_git_at(
                &repository.worktree,
                "inspect test merge conflict",
                vec!["status".into(), "--porcelain".into()],
            )
            .expect("conflict status should be available");
        assert!(String::from_utf8_lossy(&status.stdout).starts_with("UU "));
    }

    #[test]
    fn supports_linked_worktrees_and_hook_rejection() {
        let repository = TestRepository::init().expect("fixture initialization should succeed");
        repository
            .commit_file("tracked.txt", b"base\n")
            .expect("initial fixture commit should succeed");
        let linked = repository
            .add_linked_worktree()
            .expect("linked worktree should be created");
        assert!(linked.join(".git").is_file());

        repository
            .install_hook("pre-commit", b"#!/bin/sh\nexit 1\n")
            .expect("fixture hook should be installed");
        let head_before_rejection = repository
            .head_oid()
            .expect("HEAD inspection should succeed before rejection");
        let rejected = repository.commit_file("blocked.txt", b"must not commit\n");
        assert!(rejected.is_err());
        assert_eq!(
            repository
                .head_oid()
                .expect("HEAD inspection should succeed after rejection"),
            head_before_rejection
        );
    }
}
