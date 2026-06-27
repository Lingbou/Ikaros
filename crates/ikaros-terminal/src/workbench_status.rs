// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    SlashCommandPaletteItem, WorkbenchCell, WorkbenchCellKind, WorkbenchProgressSnapshot,
    slash_command_palette_items, slash_command_palette_summary, terminal_inline,
};

pub fn screen_command_palette_cells(query: Option<&str>, limit: usize) -> Vec<WorkbenchCell> {
    let summary = slash_command_palette_summary(query);
    let mut cells = vec![WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "command palette".into(),
        detail: format!(
            "query={} commands={} total={} effects={} permissions={} surfaces={} command=/screen --palette palette=/screen --palette inspect=/commands --palette search=/commands",
            terminal_inline(&summary.query),
            summary.command_count,
            summary.total_commands,
            terminal_inline(&summary.effects),
            terminal_inline(&summary.permissions),
            terminal_inline(&summary.surfaces),
        ),
    }];
    cells.extend(
        slash_command_palette_items(query, limit)
            .into_iter()
            .map(|item| {
                let primary = command_palette_primary_action(&item);
                WorkbenchCell {
                    kind: command_palette_cell_kind(item.effect),
                    title: format!("palette {}", item.name),
                    detail: format!(
                        "command={} inspect=/commands {} usage={} args={} effect={} permissions={} surfaces={} tags={} summary={}",
                        terminal_inline(&primary),
                        terminal_inline(item.name),
                        terminal_inline(item.usage),
                        item.argument_model,
                        item.effect,
                        terminal_inline(&item.permissions),
                        terminal_inline(&item.surfaces),
                        terminal_inline(&item.tags),
                        terminal_inline(item.summary),
                    ),
                }
            }),
    );
    cells
}

pub fn screen_active_work_cells(
    progress: Option<&WorkbenchProgressSnapshot>,
    pending_approvals: usize,
    continuations: usize,
    budget_recovery_command: Option<&str>,
) -> Vec<WorkbenchCell> {
    let Some(progress) = progress else {
        return Vec::new();
    };
    let active = matches!(
        progress.status.as_str(),
        "running" | "approval_pending" | "queued" | "failed"
    );
    if !active && pending_approvals == 0 && continuations == 0 {
        return Vec::new();
    }
    let title = match progress.status.as_str() {
        "approval_pending" => "active approval",
        "failed" => "active failure",
        "running" => "active turn",
        "queued" => "active queue",
        _ => "last turn",
    };
    let mut detail = format!(
        "kind={} status={} phase={} spinner={} progress_bar={} approvals={} continuations={} elapsed_ms={} detail={} cancel=/cancel all trace=/trace timeline=/timeline",
        terminal_inline(&progress.kind),
        terminal_inline(&progress.status),
        progress.phase(),
        progress.spinner(),
        progress.progress_bar(),
        pending_approvals,
        continuations,
        progress
            .elapsed_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".into()),
        terminal_inline(&progress.detail),
    );
    if progress.status == "approval_pending" || pending_approvals > 0 {
        detail.push_str(
            " approve=/screen approve-selected deny=/screen deny-selected approval=/approval",
        );
    }
    detail.push_str(&progress_recovery_commands(
        &progress.status,
        progress.error_kind.as_deref(),
        budget_recovery_command,
    ));
    vec![WorkbenchCell {
        kind: progress_cell_kind(&progress.status),
        title: title.into(),
        detail,
    }]
}

pub fn screen_progress_status_cell(
    progress: Option<&WorkbenchProgressSnapshot>,
    budget_recovery_command: Option<&str>,
) -> WorkbenchCell {
    let Some(progress) = progress else {
        return WorkbenchCell {
            kind: WorkbenchCellKind::Session,
            title: "progress".into(),
            detail: "kind=idle status=idle phase=idle spinner=- progress_bar=[----------] elapsed_ms=none error_kind=none detail=none".into(),
        };
    };
    WorkbenchCell {
        kind: progress_cell_kind(&progress.status),
        title: "progress".into(),
        detail: format!(
            "kind={} status={} phase={} spinner={} progress_bar={} elapsed_ms={} error_kind={} detail={}{}",
            terminal_inline(&progress.kind),
            terminal_inline(&progress.status),
            progress.phase(),
            progress.spinner(),
            progress.progress_bar(),
            progress
                .elapsed_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            progress
                .error_kind
                .as_deref()
                .map(terminal_inline)
                .unwrap_or_else(|| "none".into()),
            terminal_inline(&progress.detail),
            progress_recovery_commands(
                &progress.status,
                progress.error_kind.as_deref(),
                budget_recovery_command,
            )
        ),
    }
}

pub fn progress_footer_summary(progress: Option<&WorkbenchProgressSnapshot>) -> String {
    progress
        .map(|progress| {
            format!(
                "{}:{}:{}",
                terminal_inline(&progress.kind),
                terminal_inline(&progress.status),
                progress.phase()
            )
        })
        .unwrap_or_else(|| "idle".into())
}

pub fn workbench_screen_dimensions_from_values(
    columns: Option<&str>,
    lines: Option<&str>,
) -> (usize, usize) {
    let Some(width) = columns.and_then(|value| value.parse::<usize>().ok()) else {
        return (100, 24);
    };
    let Some(height) = lines.and_then(|value| value.parse::<usize>().ok()) else {
        return (100, 24);
    };
    (width.max(80), height.max(20))
}

fn command_palette_primary_action(item: &SlashCommandPaletteItem) -> String {
    match item.argument_model {
        "none" | "optional" => item.name.to_owned(),
        _ => format!("/commands {}", item.name),
    }
}

fn command_palette_cell_kind(effect: &str) -> WorkbenchCellKind {
    match effect {
        "approval-decision" => WorkbenchCellKind::Approval,
        "context-inspection" => WorkbenchCellKind::Context,
        "workspace-inspection" | "workspace-mutation" => WorkbenchCellKind::Coding,
        "provider-probe" => WorkbenchCellKind::Model,
        "queue-mutation" | "interrupt" => WorkbenchCellKind::Continuation,
        "config-mutation" | "agent-mutation" | "session-mutation" => WorkbenchCellKind::Session,
        _ => WorkbenchCellKind::Session,
    }
}

fn progress_cell_kind(status: &str) -> WorkbenchCellKind {
    match status {
        "failed" => WorkbenchCellKind::Error,
        "approval_pending" => WorkbenchCellKind::Approval,
        "running" => WorkbenchCellKind::Continuation,
        _ => WorkbenchCellKind::Session,
    }
}

fn progress_recovery_commands(
    status: &str,
    error_kind: Option<&str>,
    budget_recovery_command: Option<&str>,
) -> String {
    if status == "approval_pending" {
        return " command=/approval approve=/screen approve-selected deny=/screen deny-selected trace=/trace --approval".into();
    }
    match error_kind {
        Some("budget_exceeded") => format!(
            " command=/status budget=/budget raise={} disable=/budget disable trace=/trace --failed",
            terminal_inline(budget_recovery_command.unwrap_or("/budget set <tokens>"))
        ),
        Some("provider_error") => {
            " command=/provider debug health=/provider health --live trace=/trace --failed".into()
        }
        Some("unsupported_content") => {
            " command=/attach list clear=/attach clear matrix=/provider matrix trace=/trace --failed"
                .into()
        }
        Some("cancelled") => " command=/trace --failed".into(),
        Some(_) => " command=/trace --failed".into(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_progress_status_cell_renders_latest_progress_without_secret_leakage() {
        let progress = WorkbenchProgressSnapshot {
            kind: "chat_turn".into(),
            status: "failed".into(),
            elapsed_ms: Some(42),
            detail: "provider key [REDACTED_SECRET] failed".into(),
            error_kind: Some("provider_error".into()),
        };

        let rendered = screen_progress_status_cell(Some(&progress), None).render();

        assert!(rendered.contains("cell kind=error title=progress"));
        assert!(rendered.contains("kind=chat_turn"));
        assert!(rendered.contains("status=failed"));
        assert!(rendered.contains("elapsed_ms=42"));
        assert!(rendered.contains("error_kind=provider_error"));
        assert!(rendered.contains("command=/provider debug"));
        assert!(rendered.contains("health=/provider health --live"));
        assert!(rendered.contains("trace=/trace --failed"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn screen_progress_status_cell_surfaces_approval_pending_actions() {
        let progress = WorkbenchProgressSnapshot {
            kind: "chat_turn".into(),
            status: "approval_pending".into(),
            elapsed_ms: Some(17),
            detail: "pending_approvals=1 new_approvals=1".into(),
            error_kind: None,
        };

        let rendered = screen_progress_status_cell(Some(&progress), None).render();

        assert!(rendered.contains("cell kind=approval title=progress"));
        assert!(rendered.contains("status=approval_pending"));
        assert!(rendered.contains("pending_approvals=1"));
        assert!(rendered.contains("command=/approval"));
        assert!(rendered.contains("approve=/screen approve-selected"));
        assert!(rendered.contains("deny=/screen deny-selected"));
        assert!(rendered.contains("trace=/trace --approval"));
    }

    #[test]
    fn active_work_cell_uses_caller_supplied_budget_recovery_command() {
        let progress = WorkbenchProgressSnapshot {
            kind: "chat_turn".into(),
            status: "failed".into(),
            elapsed_ms: Some(8),
            detail: "used 100 estimated request 200 budget 150".into(),
            error_kind: Some("budget_exceeded".into()),
        };

        let rendered = screen_active_work_cells(Some(&progress), 0, 0, Some("/budget set 100000"))
            .into_iter()
            .map(|cell| cell.render())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("title=active failure"));
        assert!(rendered.contains("raise=/budget set 100000"));
        assert!(rendered.contains("trace=/trace --failed"));
    }

    #[test]
    fn command_palette_cells_show_summary_and_items() {
        let cells = screen_command_palette_cells(Some("provider"), 2);
        let rendered = cells
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("title=command palette"));
        assert!(rendered.contains("query=provider"));
        assert!(rendered.contains("title=palette "));
        assert!(rendered.contains("command="));
        assert!(rendered.contains("inspect=/commands"));
    }

    #[test]
    fn workbench_screen_dimensions_use_terminal_env_with_minimum_bounds() {
        assert_eq!(
            workbench_screen_dimensions_from_values(Some("132"), Some("43")),
            (132, 43)
        );
        assert_eq!(
            workbench_screen_dimensions_from_values(Some("12"), Some("3")),
            (80, 20)
        );
        assert_eq!(
            workbench_screen_dimensions_from_values(Some("bad"), None),
            (100, 24)
        );
    }
}
