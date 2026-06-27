// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::attachments::{content_block_kind, content_block_summary};
use crate::chat::interactive::InteractiveChatRuntime;
use crate::chat::workbench::{WorkbenchCell, WorkbenchCellKind, terminal_inline};
use ikaros_agent::chat::ChatRunOptions;
use ikaros_core::{IkarosConfig, STRUCTURED_TRACE_SCHEMA, redact_secrets};
use ikaros_execution::harness::ApprovalRecord;
use ikaros_state::session::{SessionContinuation, SessionContinuationStatus};

use super::super::truncate_chars;

pub(super) fn screen_session_context_cell(
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    pending_approvals: usize,
    continuations: usize,
) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "session current".into(),
        detail: format!(
            "id={} agent={} attachments={} pending_approvals={} continuations={} agent_loop={} stream={} no_context={} command=/session history=/session history timeline=/session timeline sessions=/sessions",
            terminal_inline(&runtime.chat_session_id),
            terminal_inline(&runtime.agent.name),
            runtime.pending_content_blocks.len(),
            pending_approvals,
            continuations,
            options.agent_loop,
            options.stream,
            options.no_context,
        ),
    }
}

pub(super) fn screen_notice_cells(
    runtime: &InteractiveChatRuntime,
    limit: usize,
) -> Vec<WorkbenchCell> {
    let mut notices = runtime
        .notices
        .iter()
        .rev()
        .take(limit.max(1))
        .map(|notice| notice.to_cell())
        .collect::<Vec<_>>();
    notices.reverse();
    if notices.is_empty() {
        return vec![WorkbenchCell {
            kind: WorkbenchCellKind::Session,
            title: "notices".into(),
            detail: "recent=0 source=workbench commands/progress/errors".into(),
        }];
    }
    notices
}

pub(super) fn screen_side_notice_cells(
    runtime: &InteractiveChatRuntime,
    limit: usize,
) -> Vec<WorkbenchCell> {
    let mut cells = runtime
        .notices
        .iter()
        .rev()
        .filter(|notice| {
            matches!(
                notice.kind,
                crate::chat::notice::WorkbenchNoticeKind::Approval
                    | crate::chat::notice::WorkbenchNoticeKind::Continuation
                    | crate::chat::notice::WorkbenchNoticeKind::Error
            )
        })
        .take(limit.max(1))
        .map(|notice| notice.to_cell())
        .collect::<Vec<_>>();
    cells.reverse();
    cells
}

pub(super) fn screen_attachment_status_cell(runtime: &InteractiveChatRuntime) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Context,
        title: "attachments".into(),
        detail: format!(
            "pending={} forces_single_call={} command=/attach list clear=/attach clear add=image|audio|file usage=/attach <kind> <url-or-path>",
            runtime.pending_content_blocks.len(),
            !runtime.pending_content_blocks.is_empty(),
        ),
    }
}

pub(super) fn screen_bottom_pane_status_cell(
    pending: &[ApprovalRecord],
    continuations: &[SessionContinuation],
    runtime: &InteractiveChatRuntime,
) -> WorkbenchCell {
    let next_input = runtime
        .pending_inputs
        .front()
        .map(|value| terminal_inline(&truncate_chars(&redact_secrets(value), 96)))
        .unwrap_or_else(|| "none".into());
    let active_view = if !pending.is_empty() {
        "approval"
    } else if !runtime.pending_inputs.is_empty() {
        "input_queue"
    } else if !runtime.pending_content_blocks.is_empty() {
        "attachments"
    } else if continuations.iter().any(|continuation| {
        matches!(
            continuation.status,
            SessionContinuationStatus::Queued | SessionContinuationStatus::Running
        )
    }) {
        "continuation"
    } else {
        "composer"
    };
    WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "bottom pane".into(),
        detail: format!(
            "active_view={} approvals={} pending_inputs={} next_input={} attachments={} continuations={} input=readline tab=complete ctrl-r=history ctrl-z=undo ctrl-y=redo alt-a=approve alt-d=deny alt-c=cancel enter=open-selected alt-enter=confirm-selected raw=/screen --raw rich=/screen --rich command=/screen focus=/screen tab palette=/screen --palette",
            active_view,
            pending.len(),
            runtime.pending_inputs.len(),
            next_input,
            runtime.pending_content_blocks.len(),
            continuations.len(),
        ),
    }
}

pub(super) fn screen_attachment_cells(runtime: &InteractiveChatRuntime) -> Vec<WorkbenchCell> {
    if runtime.pending_content_blocks.is_empty() {
        return vec![WorkbenchCell {
            kind: WorkbenchCellKind::Context,
            title: "multimodal attachments".into(),
            detail:
                "pending=0 next_turn_agent_loop=enabled command=/attach list add=image|audio|file usage=/attach <kind> <url-or-path>"
                    .into(),
        }];
    }

    let mut cells = vec![WorkbenchCell {
        kind: WorkbenchCellKind::Context,
        title: "multimodal attachments".into(),
        detail: format!(
            "pending={} next_turn_agent_loop=disabled reason=multimodal_content_blocks command=/attach list clear=/attach clear",
            runtime.pending_content_blocks.len()
        ),
    }];
    cells.extend(
        runtime
            .pending_content_blocks
            .iter()
            .take(6)
            .enumerate()
            .map(|(index, block)| WorkbenchCell {
                kind: WorkbenchCellKind::Context,
                title: format!("attachment {}", index + 1),
                detail: format!(
                    "kind={} summary={} command=/attach list clear=/attach remove {} clear_all=/attach clear",
                    content_block_kind(block),
                    terminal_inline(&content_block_summary(block)),
                    index + 1,
                ),
            }),
    );
    cells
}

pub(super) fn screen_sandbox_cell(config: &IkarosConfig) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Tool,
        title: "sandbox".into(),
        detail: format!(
            "backend={} read_scope={} network_enabled={} allow_provider_hosts={} allowed_hosts={} image_configured={} command=/sandbox probe=/sandbox --probe debug=/debug sandbox readiness=/debug readiness",
            terminal_inline(&config.execution.sandbox.backend),
            terminal_inline(&config.execution.sandbox.read_scope),
            config.execution.network.enabled,
            config.execution.network.allow_provider_hosts,
            config.execution.network.allowed_hosts.len(),
            !config.execution.sandbox.image.trim().is_empty(),
        ),
    }
}

pub(super) fn screen_state_db_cell(runtime: &InteractiveChatRuntime) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "state db".into(),
        detail: format!(
            "path={} command=/debug state-db dump=/debug dump logs=/debug logs",
            terminal_inline(&runtime.state_dir.join("state.db").display().to_string())
        ),
    }
}

pub(super) fn screen_observability_cell() -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Audit,
        title: "observability".into(),
        detail: format!(
            "trace_schema={} command=/debug insights logs=/debug logs trace=/debug logs --source trace dump=/debug dump readiness=/debug readiness",
            STRUCTURED_TRACE_SCHEMA
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::screen_sandbox_cell;
    use ikaros_core::IkarosConfig;

    #[test]
    fn sandbox_screen_cell_opens_sandbox_command_by_default() {
        let config = IkarosConfig::default();

        let rendered = screen_sandbox_cell(&config).render();

        assert!(rendered.contains("title=sandbox"));
        assert!(rendered.contains("command=/sandbox"));
        assert!(rendered.contains("probe=/sandbox --probe"));
        assert!(rendered.contains("debug=/debug sandbox"));
        assert!(rendered.contains("network_enabled="));
    }
}
