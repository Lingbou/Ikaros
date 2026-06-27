// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use crate::chat::progress::WorkbenchProgressSnapshot;
use anyhow::Result;
use ikaros_agent::chat::ChatRunOptions;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_providers::model::ModelUsageLedger;
use ikaros_state::gateway::LocalGatewayStore;
use ikaros_terminal::{
    progress_footer_summary, screen_active_work_cells, screen_command_palette_cells,
    screen_progress_status_cell, workbench_screen_dimensions_from_values,
};
use std::path::Path;

use super::super::{
    WorkbenchCell, WorkbenchCellKind, WorkbenchScreen, WorkbenchScreenState,
    render_fullscreen_terminal_frame, render_fullscreen_workbench_with_state, screen_json_line,
    screen_selected_actions_json_line, screen_selected_actions_line, screen_selected_cell_line,
    screen_selected_primary_action, slash_command_registry_summary, terminal_inline,
};
use super::{
    api::screen_api_cell,
    approval::print_approval_overlay,
    context::screen_context_cells,
    gateway::screen_gateway_status_cell,
    memory::screen_memory_cell,
    print_workbench_status,
    provider::{active_provider_status_report, screen_provider_cells, screen_provider_health_cell},
    queue::{screen_continuations, screen_queue_status_cell, screen_side_cells},
    timeline::{
        TimelineRequest, TimelineVerbosity, print_replay_status, print_screen_trace_snapshot,
        screen_coding_cells, screen_failure_cells, screen_timeline_cells,
    },
    tools::{
        screen_browser_cell, screen_image_cell, screen_mcp_cell, screen_rag_cell,
        screen_vision_cell, screen_web_cell,
    },
};

#[cfg(test)]
mod cache;
mod cells;
mod conversation;

#[cfg(test)]
pub(in crate::chat) use cache::{
    apply_live_model_stream_to_cached_screen, apply_pending_user_input_to_cached_screen,
    apply_progress_to_cached_screen,
};
use cells::{
    screen_attachment_cells, screen_attachment_status_cell, screen_bottom_pane_status_cell,
    screen_notice_cells, screen_observability_cell, screen_sandbox_cell,
    screen_session_context_cell, screen_side_notice_cells, screen_state_db_cell,
};
use conversation::screen_conversation_cells;

pub(in crate::chat) fn print_screen_status_with_state(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
    screen_state: &WorkbenchScreenState,
) -> Result<()> {
    print_screen_status_with_terminal_mode(
        config,
        paths,
        workspace,
        runtime,
        options,
        usage_ledger,
        screen_state,
    )
}

fn print_screen_status_with_terminal_mode(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
    screen_state: &WorkbenchScreenState,
) -> Result<()> {
    let screen = build_workbench_screen(config, paths, workspace, runtime, options, usage_ledger)?;
    let (width, height) = workbench_screen_dimensions();
    let rendered = if screen_state.fullscreen() {
        render_fullscreen_terminal_frame(&screen, screen_state, width, height)
    } else {
        render_fullscreen_workbench_with_state(&screen, screen_state, width, height)
    };
    print!("{rendered}");
    if screen_state.raw_mode() && !screen_state.fullscreen() {
        print_screen_status_diagnostics(
            config,
            paths,
            workspace,
            runtime,
            options,
            usage_ledger,
            &screen,
            screen_state,
        )?;
    }
    Ok(())
}

fn print_screen_status_diagnostics(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
    screen: &WorkbenchScreen,
    screen_state: &WorkbenchScreenState,
) -> Result<()> {
    let pending = runtime.session.pending_approvals()?;
    println!(
        "screen_mode: {}",
        if screen_state.fullscreen() {
            "fullscreen"
        } else {
            "refreshed"
        }
    );
    println!("screen_header: Ikaros Workbench");
    println!("screen_sections: status approval timeline trace footer");
    println!("{}", screen_selected_cell_line(screen, screen_state));
    println!("{}", screen_selected_actions_line(screen, screen_state));
    println!(
        "{}",
        screen_selected_actions_json_line(screen, screen_state)
    );
    println!("{}", screen_json_line(screen, screen_state));
    print_workbench_status(config, paths, workspace, runtime, options, usage_ledger)?;
    print_approval_overlay(runtime, &pending);
    print_screen_provider_health_snapshot(paths, runtime)?;
    print_screen_input_queue_snapshot(runtime);
    println!("screen_timeline_command: /timeline --page 2");
    print_replay_status(
        "timeline",
        config,
        paths,
        workspace,
        runtime,
        TimelineVerbosity::Timeline,
        TimelineRequest::default(),
    )?;
    print_screen_trace_snapshot(config, paths, workspace, runtime)?;
    println!(
        "screen_footer: session={} pending_approvals={} attachments={} provider={} model={} stream={} progress={} page_hint=/timeline --page 2 approval_hint=/approval approve <id>",
        terminal_inline(&runtime.chat_session_id),
        pending.len(),
        runtime.pending_content_blocks.len(),
        terminal_inline(&runtime.model_config.provider),
        terminal_inline(&runtime.model_config.model),
        options.stream,
        progress_footer_summary(runtime.last_progress.as_ref())
    );
    Ok(())
}

pub(in crate::chat) fn selected_screen_primary_action(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
    screen_state: &WorkbenchScreenState,
) -> Result<Option<String>> {
    let screen = build_workbench_screen(config, paths, workspace, runtime, options, usage_ledger)?;
    Ok(screen_selected_primary_action(&screen, screen_state))
}

fn build_workbench_screen(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    _usage_ledger: &ModelUsageLedger,
) -> Result<WorkbenchScreen> {
    let pending = runtime.session.pending_approvals()?;
    let timeline_cells = screen_timeline_cells(config, paths, workspace, runtime)?;
    let continuations = screen_continuations(config, paths, workspace, runtime)?;
    let provider = active_provider_status_report(paths, runtime)?;
    let mut main_cells = screen_conversation_cells(config, paths, workspace, runtime)?;
    let progress_budget_command = progress_budget_recovery_command(runtime.last_progress.as_ref());
    main_cells.extend(screen_active_work_cells(
        runtime.last_progress.as_ref(),
        pending.len(),
        continuations.len(),
        progress_budget_command.as_deref(),
    ));
    main_cells.extend(screen_notice_cells(runtime, 6));
    main_cells.extend(screen_attachment_cells(runtime));
    main_cells.extend(screen_provider_cells(&provider));
    main_cells.extend(screen_failure_cells(config, paths, workspace, runtime)?);
    main_cells.extend(screen_coding_cells(config, paths, workspace, runtime)?);
    main_cells.push(WorkbenchCell {
        kind: WorkbenchCellKind::Tool,
        title: "tools".into(),
        detail: "command=/tools direct/deferred/disabled tool visibility for active agent".into(),
    });
    main_cells.push(WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "commands".into(),
        detail: slash_command_registry_summary(),
    });
    main_cells.extend(screen_command_palette_cells(None, 8));
    main_cells.push(screen_session_context_cell(
        runtime,
        options,
        pending.len(),
        continuations.len(),
    ));
    main_cells.push(WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "renderer".into(),
        detail: "markdown=terminal code_fence=true diff=true table=true selected_detail=rendered command=/screen"
            .into(),
    });
    main_cells.extend(screen_context_cells(
        config, paths, workspace, runtime, options,
    )?);
    main_cells.push(screen_memory_cell(config, paths, runtime)?);
    main_cells.push(screen_rag_cell(config, paths, options));
    main_cells.push(screen_mcp_cell(config));
    main_cells.push(screen_api_cell(config));
    main_cells.push(screen_browser_cell());
    main_cells.push(screen_web_cell());
    main_cells.push(screen_vision_cell());
    main_cells.push(screen_image_cell());
    main_cells.push(screen_sandbox_cell(config));
    main_cells.push(screen_state_db_cell(runtime));
    main_cells.push(screen_observability_cell());
    main_cells.push(WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "readiness".into(),
        detail: "command=/debug readiness mvp_status=first-slice-report output=readiness_json"
            .into(),
    });
    let gateway_store = LocalGatewayStore::new(&paths.gateway_dir);
    let gateway_status_cell = screen_gateway_status_cell(&gateway_store)?;
    Ok(WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            WorkbenchCell {
                kind: WorkbenchCellKind::Model,
                title: "model".into(),
                detail: format!(
                    "provider={} model={} stream={}",
                    terminal_inline(&runtime.model_config.provider),
                    terminal_inline(&runtime.model_config.model),
                    options.stream
                ),
            },
            WorkbenchCell {
                kind: WorkbenchCellKind::Session,
                title: "workspace".into(),
                detail: format!("path={}", terminal_inline(&workspace.display().to_string())),
            },
            WorkbenchCell {
                kind: WorkbenchCellKind::Session,
                title: "session".into(),
                detail: format!(
                    "id={} agent={} attachments={} command=/session",
                    terminal_inline(&runtime.chat_session_id),
                    terminal_inline(&runtime.agent.name),
                    runtime.pending_content_blocks.len()
                ),
            },
            screen_attachment_status_cell(runtime),
            screen_bottom_pane_status_cell(&pending, &continuations, runtime),
            screen_queue_status_cell(&continuations),
            gateway_status_cell,
            screen_progress_status_cell(
                runtime.last_progress.as_ref(),
                progress_budget_command.as_deref(),
            ),
        ],
        timeline: timeline_cells,
        main: main_cells,
        side: screen_side_cells(
            &pending,
            &continuations,
            &runtime.pending_inputs,
            &runtime.pending_content_blocks,
        )
        .into_iter()
        .chain(screen_side_notice_cells(runtime, 4))
        .collect(),
        footer: format!(
            "session={} pending_approvals={} attachments={} provider={} model={} stream={} progress={} page_hint=/timeline --page 2 approval_hint=/approval approve <id>",
            terminal_inline(&runtime.chat_session_id),
            pending.len(),
            runtime.pending_content_blocks.len(),
            terminal_inline(&runtime.model_config.provider),
            terminal_inline(&runtime.model_config.model),
            options.stream,
            progress_footer_summary(runtime.last_progress.as_ref())
        ),
        input_hint:
            "type a message or slash command; tab completes slash commands; ctrl-z undo; ctrl-y redo; alt-b/f moves by word; ctrl-w/alt-d deletes by word; /commands shows registry metadata"
                .into(),
    })
}

fn print_screen_provider_health_snapshot(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let report = active_provider_status_report(paths, runtime)?;
    let cell = screen_provider_health_cell(&report.health);
    println!("screen_provider_health: {}", cell.detail);
    Ok(())
}

fn print_screen_input_queue_snapshot(runtime: &InteractiveChatRuntime) {
    println!(
        "screen_input_queue: pending_inputs={} pending_attachments={}",
        runtime.pending_inputs.len(),
        runtime.pending_content_blocks.len()
    );
    if let Some(next) = runtime.pending_inputs.front() {
        println!("screen_input_next: {}", terminal_inline(next));
        println!("screen_input_clear: clear=/queue clear");
    }
}

fn workbench_screen_dimensions() -> (usize, usize) {
    let columns = std::env::var("COLUMNS").ok();
    let lines = std::env::var("LINES").ok();
    workbench_screen_dimensions_from_values(columns.as_deref(), lines.as_deref())
}

fn progress_budget_recovery_command(
    progress: Option<&WorkbenchProgressSnapshot>,
) -> Option<String> {
    let progress = progress?;
    (progress.error_kind.as_deref() == Some("budget_exceeded"))
        .then(|| crate::chat::suggested_budget_command(&progress.detail))
        .flatten()
}
