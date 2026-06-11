// SPDX-License-Identifier: GPL-3.0-only

use super::{
    SelfModifyOperationKind, SelfModifyOperationRecord, SelfModifyProposal, SelfModifyRollbackPlan,
    SelfModifyRollbackReport, SelfModifyStore,
};
use ikaros_core::{IkarosError, Result, now_rfc3339};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

impl SelfModifyStore {
    pub fn rollback(&self, proposal_id: &str) -> Result<SelfModifyRollbackReport> {
        self.rollback_with_kind(proposal_id, SelfModifyOperationKind::Rollback)
    }

    pub(super) fn rollback_with_kind(
        &self,
        proposal_id: &str,
        kind: SelfModifyOperationKind,
    ) -> Result<SelfModifyRollbackReport> {
        let proposal = self
            .get(proposal_id)?
            .ok_or_else(|| IkarosError::Message(format!("proposal not found: {proposal_id}")))?;
        let target_path = self.resolve_target(&proposal.target_path)?;
        let snapshot_path = self.validated_snapshot_path(&proposal)?;
        if !snapshot_path.exists() {
            return Err(IkarosError::Message(format!(
                "rollback snapshot not found: {}",
                snapshot_path.display()
            )));
        }

        let mut restored_snapshot = false;
        let mut removed_created_target = false;
        if proposal.rollback_plan.target_existed {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
            }
            fs::copy(&snapshot_path, &target_path)
                .map_err(|source| IkarosError::io(&target_path, source))?;
            restored_snapshot = true;
        } else if target_path.exists() {
            fs::remove_file(&target_path)
                .map_err(|source| IkarosError::io(&target_path, source))?;
            removed_created_target = true;
        }

        let report = SelfModifyRollbackReport {
            at: now_rfc3339()?,
            operation_id: Uuid::new_v4().to_string(),
            proposal_id: proposal.id,
            target_path: proposal.target_path,
            snapshot_path,
            restored_snapshot,
            removed_created_target,
        };
        self.append_operation(SelfModifyOperationRecord {
            id: report.operation_id.clone(),
            at: report.at.clone(),
            kind,
            proposal_id: report.proposal_id.clone(),
            approval_id: None,
            target_path: report.target_path.clone(),
            summary: if report.restored_snapshot {
                "self-modify rollback restored the proposal snapshot".into()
            } else if report.removed_created_target {
                "self-modify rollback removed the created target".into()
            } else {
                "self-modify rollback had no target change to restore".into()
            },
            check_profile: None,
            post_checks_passed: None,
            auto_rollback_operation_id: None,
        })?;
        Ok(report)
    }

    pub(super) fn write_rollback_snapshot(
        &self,
        id: &str,
        target_path: &Path,
    ) -> Result<SelfModifyRollbackPlan> {
        let snapshot_dir = self.store_dir.join("rollback").join(id);
        fs::create_dir_all(&snapshot_dir)
            .map_err(|source| IkarosError::io(&snapshot_dir, source))?;
        let snapshot_path = snapshot_dir.join("target.snapshot");
        let target_existed = target_path.exists();
        if target_existed {
            fs::copy(target_path, &snapshot_path)
                .map_err(|source| IkarosError::io(&snapshot_path, source))?;
        } else {
            fs::write(&snapshot_path, b"IKAROS_TARGET_DID_NOT_EXIST\n")
                .map_err(|source| IkarosError::io(&snapshot_path, source))?;
        }
        Ok(SelfModifyRollbackPlan {
            snapshot_required: true,
            snapshot_path,
            target_existed,
            notes: vec![
                "snapshot captured before any apply path exists".into(),
                "rollback can restore this target through the self-modify rollback command".into(),
            ],
        })
    }

    pub(super) fn ensure_target_matches_snapshot(
        &self,
        proposal: &SelfModifyProposal,
        target_path: &Path,
    ) -> Result<()> {
        let snapshot_path = self.validated_snapshot_path(proposal)?;
        if proposal.rollback_plan.target_existed {
            let current =
                fs::read(target_path).map_err(|source| IkarosError::io(target_path, source))?;
            let snapshot = fs::read(&snapshot_path)
                .map_err(|source| IkarosError::io(&snapshot_path, source))?;
            if current != snapshot {
                return Err(IkarosError::Message(
                    "self-modify target changed since proposal snapshot".into(),
                ));
            }
        } else if target_path.exists() {
            return Err(IkarosError::Message(
                "self-modify target was created after proposal snapshot".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn validated_snapshot_path(&self, proposal: &SelfModifyProposal) -> Result<PathBuf> {
        let snapshot_path = proposal.rollback_plan.snapshot_path.clone();
        if !snapshot_path.starts_with(&self.store_dir) {
            return Err(IkarosError::Message(
                "rollback snapshot must stay under the local self-modify store".into(),
            ));
        }
        Ok(snapshot_path)
    }
}
