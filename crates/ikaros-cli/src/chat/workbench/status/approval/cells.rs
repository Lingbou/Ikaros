// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::RiskLevel;
use ikaros_execution::harness::ApprovalRecord;

use super::super::super::{WorkbenchCell, WorkbenchCellKind, terminal_inline};
use super::value::{
    approval_bool, approval_input_preview, approval_operations, approval_provider,
    approval_risk_is_high, approval_risk_level, approval_scope, approval_session_id,
    approval_shell_command_summary, approval_turn_id, approval_write_targets,
};

pub(in crate::chat::workbench::status) fn screen_approval_cells(
    pending: &[ApprovalRecord],
) -> Vec<WorkbenchCell> {
    if pending.is_empty() {
        return vec![WorkbenchCell {
            kind: WorkbenchCellKind::Approval,
            title: "approvals".into(),
            detail: "none pending".into(),
        }];
    }
    let mut cells = vec![screen_approval_summary_cell(pending)];
    cells.extend(pending.iter().take(4).map(|record| WorkbenchCell {
        kind: WorkbenchCellKind::Approval,
        title: format!("pending {}", terminal_inline(&record.request.id)),
        detail: format!(
            "approval_id={} call_id={} tool={} risk={:?} risk_level={} scope={} operations={} provider={} session={} turn={} write_targets={} shell_commands={} reason={} input_preview={} approve=/approval approve {} deny=/approval deny {} open=/screen open-selected",
            terminal_inline(&record.request.id),
            terminal_inline(&record.request.call.id),
            terminal_inline(&record.request.call.name),
            record.request.call.risk,
            approval_risk_level(&record.request.call.risk),
            approval_scope(record),
            approval_operations(record).join(","),
            approval_provider(record),
            approval_session_id(record),
            approval_turn_id(record),
            approval_write_targets(record),
            approval_shell_command_summary(record),
            terminal_inline(&record.request.reason),
            approval_input_preview(record),
            terminal_inline(&record.request.id),
            terminal_inline(&record.request.id),
        ),
    }));
    cells
}

fn screen_approval_summary_cell(pending: &[ApprovalRecord]) -> WorkbenchCell {
    let summary = approval_queue_summary(pending);
    let first = pending
        .first()
        .map(|record| terminal_inline(&record.request.id))
        .unwrap_or_else(|| "none".into());
    WorkbenchCell {
        kind: WorkbenchCellKind::Approval,
        title: "approval controls".into(),
        detail: format!(
            "pending={} high_risk={} provider={} write={} shell={} network={} plugin={} self_modify={} first={} approve=/screen approve-selected deny=/screen deny-selected continue=/queue run list=/approval trace=/trace --approval timeline=/timeline --approval inspect=/screen --focus side --select 2 open=/screen open-selected",
            pending.len(),
            summary.high_risk,
            summary.provider_calls,
            summary.workspace_writes,
            summary.shell_calls,
            summary.network_calls,
            summary.plugin_calls,
            summary.self_modify_calls,
            first,
        ),
    }
}

#[derive(Debug, Default)]
pub(super) struct ApprovalQueueSummary {
    pub(super) high_risk: usize,
    pub(super) provider_calls: usize,
    pub(super) workspace_writes: usize,
    pub(super) shell_calls: usize,
    pub(super) network_calls: usize,
    pub(super) plugin_calls: usize,
    pub(super) self_modify_calls: usize,
}

pub(super) fn approval_queue_summary(pending: &[ApprovalRecord]) -> ApprovalQueueSummary {
    let mut summary = ApprovalQueueSummary::default();
    for record in pending {
        let context = record.request.context.as_ref();
        if approval_risk_is_high(&record.request.call.risk) {
            summary.high_risk += 1;
        }
        if approval_bool(context, &["operations", "provider_call"]) {
            summary.provider_calls += 1;
        }
        if approval_bool(context, &["operations", "workspace_write"])
            || matches!(
                record.request.call.risk,
                RiskLevel::LocalWrite
                    | RiskLevel::ShellWrite
                    | RiskLevel::DatabaseWrite
                    | RiskLevel::SelfModify
            )
        {
            summary.workspace_writes += 1;
        }
        if approval_bool(context, &["operations", "shell"])
            || matches!(
                record.request.call.risk,
                RiskLevel::ShellRead | RiskLevel::ShellWrite | RiskLevel::Destructive
            )
        {
            summary.shell_calls += 1;
        }
        if approval_bool(context, &["operations", "network"])
            || matches!(
                record.request.call.risk,
                RiskLevel::Network | RiskLevel::RemoteServer
            )
        {
            summary.network_calls += 1;
        }
        if approval_bool(context, &["operations", "plugin"])
            || record.request.call.name.starts_with("plugin_")
        {
            summary.plugin_calls += 1;
        }
        if matches!(record.request.call.risk, RiskLevel::SelfModify)
            || record.request.call.name.contains("self_modify")
        {
            summary.self_modify_calls += 1;
        }
    }
    summary
}
