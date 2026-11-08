use git_domain::{
    EditedIndexSessionId, GitError, GitErrorKind, GitResult, ObjectId, RepoPath, SessionTarget,
};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditableIndexSession {
    pub id: EditedIndexSessionId,
    pub target: SessionTarget,
    pub path: RepoPath,
    pub original_index_oid: Option<ObjectId>,
    pub original_head_oid: Option<ObjectId>,
    pub desired_content_digest: [u8; 32],
    pub dirty: bool,
    desired_content: Vec<u8>,
}

impl EditableIndexSession {
    pub fn new(
        target: SessionTarget,
        path: RepoPath,
        original_index_oid: Option<ObjectId>,
        original_head_oid: Option<ObjectId>,
        content: Vec<u8>,
    ) -> Self {
        Self {
            id: EditedIndexSessionId::new(),
            target,
            path,
            original_index_oid,
            original_head_oid,
            desired_content_digest: digest(&content),
            dirty: false,
            desired_content: content,
        }
    }

    pub fn edit(&mut self, content: Vec<u8>) {
        self.desired_content_digest = digest(&content);
        self.desired_content = content;
        self.dirty = true;
    }

    #[allow(clippy::result_large_err)]
    pub fn apply_preflight(
        &self,
        target: SessionTarget,
        current_index_oid: Option<ObjectId>,
        current_head_oid: Option<ObjectId>,
    ) -> GitResult<Vec<u8>> {
        if !self.dirty {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "editable Index session has no confirmed edit",
            ));
        }
        if self.target != target
            || self.original_index_oid != current_index_oid
            || self.original_head_oid != current_head_oid
        {
            return Err(GitError::new(
                GitErrorKind::StaleSelection,
                "editable Index session is based on stale repository identities",
            ));
        }
        Ok(self.desired_content.clone())
    }
}

fn digest(content: &[u8]) -> [u8; 32] {
    Sha256::digest(content).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_domain::{RepositoryId, WorktreeId};

    #[test]
    fn apply_requires_dirty_session_and_matching_identities() {
        let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
        let mut session = EditableIndexSession::new(
            target,
            RepoPath::try_from("file.txt").expect("path"),
            None,
            None,
            b"base".to_vec(),
        );
        assert!(session.apply_preflight(target, None, None).is_err());
        session.edit(b"edited".to_vec());
        assert_eq!(
            session
                .apply_preflight(target, None, None)
                .expect("apply preflight"),
            b"edited"
        );
        assert!(
            session
                .apply_preflight(
                    target,
                    Some(
                        "1111111111111111111111111111111111111111"
                            .parse()
                            .expect("oid")
                    ),
                    None
                )
                .is_err()
        );
    }
}
