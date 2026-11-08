use git_diff::{SelectionPatchError, build_selection_patch};
use git_domain::{
    ChangeSelection, GitError, GitErrorKind, GitResult, SessionTarget, SnapshotRevision,
};
use git_ports::{DiffReadRequest, DiffReader, ExecutionContext, RepositoryLocation};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionPatchPreview {
    pub target: SessionTarget,
    pub generation: SnapshotRevision,
    pub selection: ChangeSelection,
    pub patch: Vec<u8>,
    pub selection_digest: [u8; 32],
}

#[derive(Debug)]
pub struct SelectionPatchPreviewService<R> {
    reader: R,
}

impl<R> SelectionPatchPreviewService<R> {
    #[must_use]
    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R: DiffReader> SelectionPatchPreviewService<R> {
    #[allow(clippy::result_large_err)]
    pub async fn prepare(
        &self,
        location: &RepositoryLocation,
        target: SessionTarget,
        request: DiffReadRequest,
        selection: ChangeSelection,
        context: ExecutionContext,
    ) -> GitResult<SelectionPatchPreview> {
        if request.target != target || selection.target() != target {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "selection preview target does not match the active session",
            )
            .with_target(target));
        }
        let document = self.reader.read(location, request, context).await?;
        let patch = build_selection_patch(&document.patch, &selection).map_err(|error| {
            let kind = match error {
                SelectionPatchError::BinaryFile => GitErrorKind::UnsupportedFeature,
                SelectionPatchError::FileNotFound
                | SelectionPatchError::EmptySelection
                | SelectionPatchError::HunkOutOfBounds
                | SelectionPatchError::LinePatch(_) => GitErrorKind::StaleSelection,
            };
            GitError::new(kind, "selection cannot be prepared from the current diff")
                .with_target(target)
        })?;
        let mut digest = Sha256::new();
        digest.update(&patch);
        Ok(SelectionPatchPreview {
            target,
            generation: document.generation,
            selection,
            patch,
            selection_digest: digest.finalize().into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use git_diff::parse;
    use git_domain::{FileIdentity, FileKind, OperationId, RepoPath, RepositoryId, WorktreeId};
    use git_ports::{DiffDocument, ProcessProgress};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[derive(Clone)]
    struct Reader {
        document: DiffDocument,
    }

    #[async_trait]
    impl DiffReader for Reader {
        async fn read(
            &self,
            _location: &RepositoryLocation,
            _request: DiffReadRequest,
            _context: ExecutionContext,
        ) -> GitResult<DiffDocument> {
            Ok(self.document.clone())
        }
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(2);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn session_target() -> SessionTarget {
        SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1)
    }

    fn selection(target: SessionTarget) -> ChangeSelection {
        ChangeSelection::WholeFile {
            file: FileIdentity {
                target,
                path: RepoPath::try_from("file.txt").expect("valid path"),
                previous_path: None,
                kind: FileKind::Regular,
            },
        }
    }

    #[tokio::test]
    async fn prepares_a_selection_patch_without_mutating_git_state() {
        let target = session_target();
        let reader = Reader {
            document: DiffDocument {
                generation: SnapshotRevision::new(4),
                patch: parse(b"diff --git a/file.txt b/file.txt\n@@ -1 +1 @@\n-old\n+new\n")
                    .expect("valid patch"),
            },
        };
        let service = SelectionPatchPreviewService::new(reader);
        let request = DiffReadRequest {
            target,
            before: git_diff::ContentSource::Head,
            after: git_diff::ContentSource::Worktree,
            paths: vec![RepoPath::try_from("file.txt").expect("valid path")],
            context_lines: 3,
        };
        let location = RepositoryLocation::worktree("/repo", "/repo/.git", "/repo/.git");
        let preview = service
            .prepare(&location, target, request, selection(target), context())
            .await
            .expect("preview");
        assert_eq!(preview.generation, SnapshotRevision::new(4));
        assert!(
            preview
                .patch
                .windows(b"+new\n".len())
                .any(|w| w == b"+new\n")
        );
        assert_ne!(preview.selection_digest, [0; 32]);
    }

    #[tokio::test]
    async fn rejects_a_selection_from_another_session() {
        let target = session_target();
        let reader = Reader {
            document: DiffDocument {
                generation: SnapshotRevision::new(1),
                patch: parse(b"diff --git a/file.txt b/file.txt\n@@ -1 +1 @@\n-a\n+b\n")
                    .expect("valid patch"),
            },
        };
        let service = SelectionPatchPreviewService::new(reader);
        let request = DiffReadRequest {
            target,
            before: git_diff::ContentSource::Head,
            after: git_diff::ContentSource::Worktree,
            paths: Vec::new(),
            context_lines: 3,
        };
        let error = service
            .prepare(
                &RepositoryLocation::worktree("/repo", "/repo/.git", "/repo/.git"),
                target,
                request,
                selection(session_target()),
                context(),
            )
            .await
            .expect_err("foreign session must be rejected");
        assert_eq!(error.kind, GitErrorKind::StaleSelection);
    }
}
