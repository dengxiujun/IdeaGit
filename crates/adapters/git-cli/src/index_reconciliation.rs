//! Conservative, lock-aware publication of a verified real Git Index candidate.

use std::{fs, io::ErrorKind};

use async_trait::async_trait;
use git_domain::{GitError, GitErrorKind, GitResult};
use git_ports::{
    ExecutionContext, IndexReconciliationOutcome, IndexReconciliationPublisher,
    IndexReconciliationRequest, RepositoryLocation,
};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct GitCliIndexReconciliationPublisher {
    location: RepositoryLocation,
}

impl GitCliIndexReconciliationPublisher {
    #[must_use]
    pub fn new(location: RepositoryLocation) -> Self {
        Self { location }
    }
}

#[async_trait]
impl IndexReconciliationPublisher for GitCliIndexReconciliationPublisher {
    #[allow(clippy::result_large_err)]
    async fn publish(
        &self,
        request: IndexReconciliationRequest,
        _context: ExecutionContext,
    ) -> GitResult<IndexReconciliationOutcome> {
        let index = self.location.git_dir().join("index");
        let lock = index.with_extension("lock");
        let file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                return Err(GitError::new(
                    GitErrorKind::IndexLocked,
                    "real Git Index lock is already held",
                ));
            }
            Err(error) => return Err(io_error("create real Git Index lock", error)),
        };
        let current = match fs::read(&index) {
            Ok(current) => current,
            Err(error) => {
                let _ = fs::remove_file(&lock);
                return Err(io_error("read real Git Index", error));
            }
        };
        if Sha256::digest(&current)[..] != request.expected_index_digest[..] {
            let _ = fs::remove_file(&lock);
            return Ok(IndexReconciliationOutcome::Conflict);
        }
        if let Err(error) = write_locked(file, &request.candidate_index) {
            let _ = fs::remove_file(&lock);
            return Err(error);
        }
        if let Err(error) = fs::rename(&lock, &index) {
            let _ = fs::remove_file(&lock);
            return Err(io_error("publish real Git Index", error));
        }
        match fs::read(&index) {
            Ok(published) if published == request.candidate_index => {}
            Ok(_) => return Ok(IndexReconciliationOutcome::Unknown),
            Err(_) => return Ok(IndexReconciliationOutcome::Unknown),
        }
        Ok(IndexReconciliationOutcome::Published)
    }
}

#[allow(clippy::result_large_err)]
fn write_locked(mut file: fs::File, bytes: &[u8]) -> GitResult<()> {
    use std::io::Write;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error("write real Git Index lock", error))
}

#[allow(clippy::result_large_err)]
fn io_error(operation: &'static str, error: std::io::Error) -> GitError {
    GitError::new(GitErrorKind::PermissionDenied, operation).with_diagnostic(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{OperationId, RepositoryId, SessionTarget, WorktreeId};
    use git_fixture::TestRepository;
    use git_ports::ProcessProgress;
    use sha2::{Digest, Sha256};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(4);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn location(fixture: &TestRepository) -> RepositoryLocation {
        RepositoryLocation::worktree(
            &fixture.worktree,
            fixture.worktree.join(".git"),
            fixture.worktree.join(".git"),
        )
    }

    #[tokio::test]
    async fn publishes_candidate_only_after_matching_index_digest() {
        let fixture = TestRepository::init().expect("fixture");
        fixture.commit_file("file.txt", b"base\n").expect("commit");
        let location = location(&fixture);
        let index = fixture.worktree.join(".git/index");
        let original = fs::read(&index).expect("index");
        let candidate = original
            .iter()
            .copied()
            .chain([0, 1, 2])
            .collect::<Vec<_>>();
        let publisher = GitCliIndexReconciliationPublisher::new(location);
        let result = publisher
            .publish(
                IndexReconciliationRequest {
                    target: target(),
                    expected_index_digest: Sha256::digest(&original).into(),
                    candidate_index: candidate.clone(),
                },
                context(),
            )
            .await
            .expect("publish");
        assert_eq!(result, IndexReconciliationOutcome::Published);
        assert_eq!(fs::read(index).expect("published index"), candidate);
    }

    #[tokio::test]
    async fn rejects_stale_index_without_replacing_it() {
        let fixture = TestRepository::init().expect("fixture");
        fixture.commit_file("file.txt", b"base\n").expect("commit");
        let location = location(&fixture);
        let index = fixture.worktree.join(".git/index");
        let original = fs::read(&index).expect("index");
        let external = original
            .iter()
            .copied()
            .chain([9, 8, 7])
            .collect::<Vec<_>>();
        fs::write(&index, &external).expect("external index mutation");
        let publisher = GitCliIndexReconciliationPublisher::new(location);
        let result = publisher
            .publish(
                IndexReconciliationRequest {
                    target: target(),
                    expected_index_digest: Sha256::digest(&original).into(),
                    candidate_index: b"must-not-publish".to_vec(),
                },
                context(),
            )
            .await
            .expect("stale index result");
        assert_eq!(result, IndexReconciliationOutcome::Conflict);
        assert_eq!(fs::read(index).expect("index"), external);
    }

    #[tokio::test]
    async fn refuses_an_existing_git_index_lock() {
        let fixture = TestRepository::init().expect("fixture");
        fixture.commit_file("file.txt", b"base\n").expect("commit");
        let location = location(&fixture);
        let index = fixture.worktree.join(".git/index");
        let lock = index.with_extension("lock");
        fs::write(&lock, b"external lock").expect("lock");
        let current = fs::read(&index).expect("index");
        let error = GitCliIndexReconciliationPublisher::new(location)
            .publish(
                IndexReconciliationRequest {
                    target: target(),
                    expected_index_digest: Sha256::digest(&current).into(),
                    candidate_index: current,
                },
                context(),
            )
            .await
            .expect_err("lock must be refused");
        assert_eq!(error.kind, GitErrorKind::IndexLocked);
        fs::remove_file(lock).expect("cleanup lock");
    }
}
