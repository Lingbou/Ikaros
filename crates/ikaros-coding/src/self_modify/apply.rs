// SPDX-License-Identifier: GPL-3.0-only

use super::{
    SelfModifyApplyReport, SelfModifyCheckProfile, SelfModifyOperationKind,
    SelfModifyOperationRecord, SelfModifyProposal, SelfModifyStore, dry_run::reject_dry_run_drift,
};
use crate::GuardedPatchApplier;
use ikaros_core::{IkarosError, Result, SelfModifyConfig, now_rfc3339, redact_secrets};
use uuid::Uuid;

impl SelfModifyStore {
    pub fn apply_approved(
        &self,
        proposal_id: &str,
        approval_id: &str,
    ) -> Result<SelfModifyApplyReport> {
        let proposal = self
            .get(proposal_id)?
            .ok_or_else(|| IkarosError::Message(format!("proposal not found: {proposal_id}")))?;
        let profile = self.default_check_profile(&proposal.change_kind);
        self.apply_proposal_with_profile(proposal, approval_id, profile)
    }

    pub fn apply_approved_with_config(
        &self,
        proposal_id: &str,
        approval_id: &str,
        config: &SelfModifyConfig,
    ) -> Result<SelfModifyApplyReport> {
        let proposal = self
            .get(proposal_id)?
            .ok_or_else(|| IkarosError::Message(format!("proposal not found: {proposal_id}")))?;
        let profile = self
            .configured_check_profile(&proposal.change_kind, config)?
            .unwrap_or_else(|| self.default_check_profile(&proposal.change_kind));
        self.apply_proposal_with_profile(proposal, approval_id, profile)
    }

    pub fn apply_approved_with_checks(
        &self,
        proposal_id: &str,
        approval_id: &str,
        check_commands: &[String],
    ) -> Result<SelfModifyApplyReport> {
        let proposal = self
            .get(proposal_id)?
            .ok_or_else(|| IkarosError::Message(format!("proposal not found: {proposal_id}")))?;
        let profile = SelfModifyCheckProfile {
            change_kind: proposal.change_kind.clone(),
            source: "override".into(),
            commands: check_commands
                .iter()
                .map(|command| redact_secrets(command))
                .collect(),
            reason: "caller supplied explicit self-modify check commands".into(),
        };
        self.apply_proposal_with_profile(proposal, approval_id, profile)
    }

    fn apply_proposal_with_profile(
        &self,
        proposal: SelfModifyProposal,
        approval_id: &str,
        check_profile: SelfModifyCheckProfile,
    ) -> Result<SelfModifyApplyReport> {
        let proposal_id_owned = proposal.id.clone();
        let target_path = self.resolve_target(&proposal.target_path)?;
        let dry_run_report = self.dry_run_report(&target_path, &proposal.unified_diff)?;
        if !dry_run_report.ok_to_request_approval {
            return Err(IkarosError::Message(
                "self-modify proposal is not ready for approval-gated apply".into(),
            ));
        }
        reject_dry_run_drift(&proposal.dry_run_report, &dry_run_report)?;
        self.ensure_target_matches_snapshot(&proposal, &target_path)?;

        let pre_heartbeat = self.heartbeat()?;
        let pre_checks = self.run_checks(&check_profile.commands)?;
        if let Some(failed) = pre_checks.iter().find(|check| !check.passed) {
            return Err(IkarosError::Message(format!(
                "pre-apply self-check failed for `{}`: {}",
                failed.command, failed.analysis.summary
            )));
        }
        let patch_report =
            GuardedPatchApplier::apply_unified_diff(&self.workspace_root, &proposal.unified_diff)?;
        let post_heartbeat = self.heartbeat()?;
        let post_checks = self.run_checks(&check_profile.commands)?;
        let post_checks_passed = post_checks.iter().all(|check| check.passed);
        let auto_rollback = if post_checks_passed {
            None
        } else {
            Some(
                self.rollback_with_kind(&proposal_id_owned, SelfModifyOperationKind::AutoRollback)?,
            )
        };

        let report = SelfModifyApplyReport {
            at: now_rfc3339()?,
            operation_id: Uuid::new_v4().to_string(),
            proposal_id: proposal_id_owned,
            approval_id: approval_id.into(),
            target_path: proposal.target_path,
            dry_run_report,
            check_profile,
            pre_heartbeat,
            pre_checks,
            patch_report,
            post_heartbeat,
            post_checks,
            post_checks_passed,
            auto_rollback,
            rollback_snapshot: proposal.rollback_plan.snapshot_path,
        };
        self.append_operation(SelfModifyOperationRecord {
            id: report.operation_id.clone(),
            at: report.at.clone(),
            kind: SelfModifyOperationKind::Apply,
            proposal_id: report.proposal_id.clone(),
            approval_id: Some(report.approval_id.clone()),
            target_path: report.target_path.clone(),
            summary: if report.post_checks_passed {
                "self-modify approved apply completed".into()
            } else {
                "self-modify approved apply rolled back after failed self-check".into()
            },
            check_profile: Some(report.check_profile.clone()),
            post_checks_passed: Some(report.post_checks_passed),
            auto_rollback_operation_id: report
                .auto_rollback
                .as_ref()
                .map(|rollback| rollback.operation_id.clone()),
        })?;
        Ok(report)
    }
}
