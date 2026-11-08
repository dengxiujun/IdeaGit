use sha2::{Digest, Sha256};

use git_domain::{GitResult, ObjectId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconciliationPlan {
    Publish { index_bytes: Vec<u8> },
    RecoveryRequired { reason: String },
}

/// Conservative post-commit Index reconciliation. It plans a publication only when every
/// observed identity still matches the pre-commit snapshot; the actual atomic Index replacement
/// remains the responsibility of the OperationCoordinator/adapter lock.
#[derive(Debug, Default)]
pub struct RealIndexReconciler;

impl RealIndexReconciler {
    #[allow(clippy::result_large_err)]
    pub fn plan(
        expected_index_digest: [u8; 32],
        observed_index: &[u8],
        expected_head: Option<ObjectId>,
        observed_head: Option<ObjectId>,
        new_index: Option<Vec<u8>>,
    ) -> GitResult<ReconciliationPlan> {
        let Some(new_index) = new_index else {
            return Ok(ReconciliationPlan::RecoveryRequired {
                reason: "no verified reconciled Index candidate was provided".to_owned(),
            });
        };
        if digest(observed_index) != expected_index_digest {
            return Ok(ReconciliationPlan::RecoveryRequired {
                reason: "real Index changed during partial commit".to_owned(),
            });
        }
        if expected_head == observed_head {
            Ok(ReconciliationPlan::RecoveryRequired {
                reason: "HEAD did not advance after partial commit".to_owned(),
            })
        } else {
            Ok(ReconciliationPlan::Publish {
                index_bytes: new_index,
            })
        }
    }

    #[must_use]
    pub fn digest(index_bytes: &[u8]) -> [u8; 32] {
        digest(index_bytes)
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(value: char) -> ObjectId {
        value.to_string().repeat(40).parse().expect("oid")
    }

    #[test]
    fn publishes_only_when_index_is_unchanged_and_head_advanced() {
        let original = b"original-index";
        let plan = RealIndexReconciler::plan(
            RealIndexReconciler::digest(original),
            original,
            Some(oid('a')),
            Some(oid('b')),
            Some(b"reconciled-index".to_vec()),
        )
        .expect("plan");
        assert_eq!(
            plan,
            ReconciliationPlan::Publish {
                index_bytes: b"reconciled-index".to_vec()
            }
        );
    }

    #[test]
    fn external_index_change_never_produces_publish_plan() {
        let plan = RealIndexReconciler::plan(
            RealIndexReconciler::digest(b"original-index"),
            b"external-index",
            Some(oid('a')),
            Some(oid('b')),
            Some(b"candidate".to_vec()),
        )
        .expect("plan");
        assert!(matches!(plan, ReconciliationPlan::RecoveryRequired { .. }));
    }
}
