// SPDX-License-Identifier: GPL-3.0-only

use super::{
    SelfModifyDryRunReport, SelfModifyStore,
    diff::{diff_file_paths, diff_matches_single_target},
};
use crate::{CodeReviewAssistant, DiffSummarizer};
use ikaros_core::{IkarosError, Result, contains_secret_like, redact_secrets};
use std::path::Path;

impl SelfModifyStore {
    pub(super) fn dry_run_report(
        &self,
        target_path: &Path,
        unified_diff: &str,
    ) -> Result<SelfModifyDryRunReport> {
        let redacted_diff = redact_secrets(unified_diff);
        let diff_summary = DiffSummarizer::summarize(&redacted_diff);
        let review = CodeReviewAssistant::review(&redacted_diff, None);
        let changed_files = diff_file_paths(&redacted_diff);
        let mut reasons = vec![
            "self-modify is disabled by default".into(),
            "autonomous apply is unavailable".into(),
            "manual apply requires an explicit approval and heartbeat check".into(),
        ];
        let mut ok_to_request_approval =
            !redacted_diff.trim().is_empty() && diff_summary.files_changed > 0;
        if contains_secret_like(unified_diff) {
            ok_to_request_approval = false;
            reasons.push("diff contains secret-like content".into());
        }
        if !diff_matches_single_target(&changed_files, target_path, &self.workspace_root) {
            ok_to_request_approval = false;
            reasons.push("diff must touch only the declared target path".into());
        }
        if review
            .findings
            .iter()
            .any(|finding| matches!(finding.severity, crate::ReviewSeverity::High))
        {
            ok_to_request_approval = false;
            reasons.push("high-severity review findings must be resolved first".into());
        }
        Ok(SelfModifyDryRunReport {
            enabled: false,
            apply_available: false,
            manual_apply_available: true,
            ok_to_request_approval,
            target_path: target_path
                .strip_prefix(&self.workspace_root)
                .unwrap_or(target_path)
                .to_path_buf(),
            diff_summary,
            changed_files,
            findings: review.findings,
            reasons,
        })
    }
}

pub(super) fn reject_dry_run_drift(
    original: &SelfModifyDryRunReport,
    current: &SelfModifyDryRunReport,
) -> Result<()> {
    if original.diff_summary != current.diff_summary
        || original.changed_files != current.changed_files
        || original.ok_to_request_approval != current.ok_to_request_approval
    {
        return Err(IkarosError::Message(
            "self-modify dry-run report changed since proposal".into(),
        ));
    }
    Ok(())
}
