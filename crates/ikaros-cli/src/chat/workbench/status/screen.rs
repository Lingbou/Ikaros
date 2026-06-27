// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::attachments::{content_block_kind, content_block_summary};
use crate::chat::interactive::InteractiveChatRuntime;
use crate::chat::progress::WorkbenchProgressSnapshot;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths, STRUCTURED_TRACE_SCHEMA, redact_secrets};
use ikaros_gateway::LocalGatewayStore;
use ikaros_models::{ModelStreamEvent, ModelUsageLedger, ProviderHealthLedger, ProviderRegistry};
use ikaros_runtime::ChatRunOptions;
use ikaros_session::{
    AgentEvent, AgentEventKind, SessionEntry, SessionEntryKind, SessionId, SessionReplay,
    SessionStore, SqliteSessionStore,
};
use std::path::Path;

use super::super::{
    PersistentWorkbenchTerminal, SlashCommandPaletteItem, WorkbenchCell, WorkbenchCellKind,
    WorkbenchInputState, WorkbenchScreen, WorkbenchScreenState,
    draw_persistent_fullscreen_terminal_frame, format_workbench_input_model_detail,
    format_workbench_input_state, render_fullscreen_terminal_frame,
    render_fullscreen_workbench_with_state, render_persistent_fullscreen_terminal_frame,
    render_terminal_markdown, screen_json_line, screen_selected_actions_json_line,
    screen_selected_actions_line, screen_selected_cell_line, screen_selected_primary_action,
    slash_command_completion_candidates, slash_command_palette_items,
    slash_command_palette_summary, slash_command_registry_summary, terminal_inline,
    terminal_message,
};
use super::{
    api::screen_api_cell,
    approval::print_approval_overlay,
    context::screen_context_cells,
    gateway::screen_gateway_status_cell,
    memory::screen_memory_cell,
    print_workbench_status,
    provider::{apply_configured_model_cost, screen_provider_cells, screen_provider_health_cell},
    queue::{screen_continuations, screen_queue_status_cell, screen_side_cells},
    state_db_candidates,
    timeline::{
        TimelineRequest, TimelineVerbosity, print_replay_status, print_screen_trace_snapshot,
        screen_coding_cells, screen_failure_cells, screen_timeline_cells,
    },
    tools::{
        screen_browser_cell, screen_image_cell, screen_mcp_cell, screen_rag_cell,
        screen_vision_cell, screen_web_cell,
    },
    truncate_chars,
};

pub(in crate::chat) fn print_screen_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    print_screen_status_with_state(
        config,
        paths,
        workspace,
        runtime,
        options,
        usage_ledger,
        &WorkbenchScreenState::default(),
    )
}

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
        ScreenStatusRender {
            state: screen_state,
            persistent_fullscreen: false,
            input_state: None,
        },
    )
}

pub(in crate::chat) fn print_persistent_screen_status_with_state(
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
        ScreenStatusRender {
            state: screen_state,
            persistent_fullscreen: true,
            input_state: None,
        },
    )
}

pub(in crate::chat) struct WorkbenchScreenStatusContext<'a> {
    pub(in crate::chat) config: &'a IkarosConfig,
    pub(in crate::chat) paths: &'a IkarosPaths,
    pub(in crate::chat) workspace: &'a Path,
    pub(in crate::chat) runtime: &'a InteractiveChatRuntime,
    pub(in crate::chat) options: &'a ChatRunOptions,
    pub(in crate::chat) usage_ledger: &'a ModelUsageLedger,
}

pub(in crate::chat) fn print_persistent_screen_status_with_input_state(
    context: WorkbenchScreenStatusContext<'_>,
    screen_state: &WorkbenchScreenState,
    input_state: &WorkbenchInputState,
) -> Result<()> {
    print_screen_status_with_terminal_mode(
        context.config,
        context.paths,
        context.workspace,
        context.runtime,
        context.options,
        context.usage_ledger,
        ScreenStatusRender {
            state: screen_state,
            persistent_fullscreen: true,
            input_state: Some(input_state),
        },
    )
}

pub(in crate::chat) fn draw_persistent_screen_status_with_state(
    context: WorkbenchScreenStatusContext<'_>,
    screen_state: &WorkbenchScreenState,
    terminal: &mut PersistentWorkbenchTerminal,
) -> Result<()> {
    let screen = build_workbench_screen(
        context.config,
        context.paths,
        context.workspace,
        context.runtime,
        context.options,
        context.usage_ledger,
    )?;
    terminal.draw(&screen, screen_state)
}

pub(in crate::chat) fn draw_persistent_screen_status_with_input_state(
    context: WorkbenchScreenStatusContext<'_>,
    screen_state: &WorkbenchScreenState,
    input_state: &WorkbenchInputState,
    terminal: &mut PersistentWorkbenchTerminal,
) -> Result<()> {
    let mut screen = build_workbench_screen(
        context.config,
        context.paths,
        context.workspace,
        context.runtime,
        context.options,
        context.usage_ledger,
    )?;
    apply_input_state_to_screen(&mut screen, input_state);
    terminal.draw(&screen, screen_state)
}

pub(in crate::chat) fn build_screen_status(
    context: WorkbenchScreenStatusContext<'_>,
) -> Result<WorkbenchScreen> {
    build_workbench_screen(
        context.config,
        context.paths,
        context.workspace,
        context.runtime,
        context.options,
        context.usage_ledger,
    )
}

pub(in crate::chat) fn apply_input_state_to_cached_screen(
    screen: &mut WorkbenchScreen,
    input_state: &WorkbenchInputState,
) {
    apply_input_state_to_screen(screen, input_state);
}

pub(in crate::chat) fn apply_progress_to_cached_screen(
    screen: &mut WorkbenchScreen,
    progress: &WorkbenchProgressSnapshot,
) {
    let progress_cell = screen_progress_status_cell(Some(progress));
    if let Some(existing) = screen
        .status
        .iter_mut()
        .find(|cell| cell.title == "progress")
    {
        *existing = progress_cell;
    } else {
        screen.status.push(progress_cell);
    }
}

pub(in crate::chat) fn apply_pending_user_input_to_cached_screen(
    screen: &mut WorkbenchScreen,
    input: &str,
) {
    let detail = terminal_message(input.trim());
    if detail.is_empty() {
        return;
    }
    if screen
        .main
        .iter()
        .rev()
        .find(|cell| is_conversation_cell(cell))
        .is_some_and(|cell| cell.title.starts_with("user turn=") && cell.detail == detail)
    {
        return;
    }
    let cell = WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "user turn=pending".into(),
        detail,
    };
    if let Some(existing) = screen
        .main
        .iter_mut()
        .find(|cell| cell.title == "user turn=pending")
    {
        *existing = cell;
        return;
    }
    let insert_at = screen
        .main
        .iter()
        .rposition(is_conversation_cell)
        .map(|index| index + 1)
        .unwrap_or(0);
    screen.main.insert(insert_at, cell);
}

pub(in crate::chat) fn apply_live_model_stream_to_cached_screen(
    screen: &mut WorkbenchScreen,
    events: &[AgentEvent],
) {
    let mut content = String::new();
    let mut has_stream_event = false;
    let mut done = false;
    for event in events {
        let AgentEventKind::ModelStream(stream_event) = &event.kind else {
            continue;
        };
        has_stream_event = true;
        match stream_event {
            ModelStreamEvent::TextDelta(text) => content.push_str(text),
            ModelStreamEvent::Done => done = true,
            _ => {}
        }
    }
    if !has_stream_event || content.trim().is_empty() {
        return;
    }
    let title = if done {
        "assistant turn=streaming done"
    } else {
        "assistant turn=streaming"
    };
    let cell = WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: title.into(),
        detail: terminal_message(&content),
    };
    if let Some(existing) = screen
        .main
        .iter_mut()
        .find(|cell| cell.title.starts_with("assistant turn=streaming"))
    {
        *existing = cell;
        return;
    }
    let insert_at = screen
        .main
        .iter()
        .rposition(is_conversation_cell)
        .map(|index| index + 1)
        .unwrap_or(0);
    screen.main.insert(insert_at, cell);
}

fn is_conversation_cell(cell: &WorkbenchCell) -> bool {
    cell.title.starts_with("user turn=") || cell.title.starts_with("assistant turn=")
}

struct ScreenStatusRender<'a> {
    state: &'a WorkbenchScreenState,
    persistent_fullscreen: bool,
    input_state: Option<&'a WorkbenchInputState>,
}

fn print_screen_status_with_terminal_mode(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
    render: ScreenStatusRender<'_>,
) -> Result<()> {
    let mut screen =
        build_workbench_screen(config, paths, workspace, runtime, options, usage_ledger)?;
    if let Some(input_state) = render.input_state {
        apply_input_state_to_screen(&mut screen, input_state);
    }
    if render.persistent_fullscreen
        && render.state.fullscreen()
        && draw_persistent_fullscreen_terminal_frame(&screen, render.state)?
    {
        return Ok(());
    }
    print!("\x1b[2J\x1b[H");
    let (width, height) = workbench_screen_dimensions();
    let rendered = if render.state.fullscreen() {
        if render.persistent_fullscreen {
            render_persistent_fullscreen_terminal_frame(&screen, render.state, width, height)
        } else {
            render_fullscreen_terminal_frame(&screen, render.state, width, height)
        }
    } else {
        render_fullscreen_workbench_with_state(&screen, render.state, width, height)
    };
    print!("{rendered}");
    if render.state.raw_mode() && !render.state.fullscreen() {
        print_screen_status_diagnostics(
            config,
            paths,
            workspace,
            runtime,
            options,
            usage_ledger,
            &screen,
            render.state,
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
    usage_ledger: &ModelUsageLedger,
) -> Result<WorkbenchScreen> {
    let pending = runtime.session.pending_approvals()?;
    let timeline_cells = screen_timeline_cells(config, paths, workspace, runtime)?;
    let continuations = screen_continuations(config, paths, workspace, runtime)?;
    let mut descriptor = ProviderRegistry
        .descriptor_with_profile(
            &runtime.model_config.provider,
            &runtime.model_provider.base_url,
            &runtime.model_config.model,
            &runtime.model_config.compat_profile,
        )
        .ok();
    if let Some(descriptor) = &mut descriptor {
        apply_configured_model_cost(descriptor, &runtime.model_config.cost);
    }
    let provider_health = ProviderHealthLedger::new(&paths.audit_dir);
    let mut main_cells = screen_conversation_cells(config, paths, workspace, runtime)?;
    main_cells.extend(screen_active_work_cells(
        runtime.last_progress.as_ref(),
        pending.len(),
        continuations.len(),
    ));
    main_cells.extend(screen_notice_cells(runtime, 6));
    main_cells.extend(screen_attachment_cells(runtime));
    main_cells.extend(screen_provider_cells(
        &runtime.model_config,
        descriptor.as_ref(),
        usage_ledger,
        &provider_health,
    )?);
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
                    "id={} agent={} attachments={}",
                    terminal_inline(&runtime.chat_session_id),
                    terminal_inline(&runtime.agent.name),
                    runtime.pending_content_blocks.len()
                ),
            },
            screen_attachment_status_cell(runtime),
            screen_bottom_pane_status_cell(&pending, &continuations, runtime),
            screen_queue_status_cell(&continuations),
            gateway_status_cell,
            screen_progress_status_cell(runtime.last_progress.as_ref()),
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

fn apply_input_state_to_screen(screen: &mut WorkbenchScreen, input_state: &WorkbenchInputState) {
    screen.input_hint = format_workbench_input_state("terminal", input_state);
    let mut cells = vec![screen_composer_state_cell(input_state)];
    cells.extend(screen_input_history_search_cells(input_state));
    cells.extend(screen_input_command_cells(input_state));
    let mut main = Vec::with_capacity(screen.main.len() + cells.len());
    main.extend(cells);
    main.append(&mut screen.main);
    screen.main = main;
}

fn screen_composer_state_cell(input_state: &WorkbenchInputState) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "composer state".into(),
        detail: format_workbench_input_model_detail("terminal", input_state),
    }
}

fn screen_input_history_search_cells(input_state: &WorkbenchInputState) -> Vec<WorkbenchCell> {
    if !input_state.history_search_active() {
        return Vec::new();
    }
    let candidates = input_state.history_search_candidates(6);
    let mut cells = vec![WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "history search".into(),
        detail: format!(
            "{} ctrl-r=older ctrl-s=newer backspace=query-delete enter=run esc=cancel",
            input_state.history_search_summary(),
        ),
    }];
    if candidates.is_empty() {
        cells.push(WorkbenchCell {
            kind: WorkbenchCellKind::Error,
            title: "history search empty".into(),
            detail: "matches=0 action=type_more_or_escape".into(),
        });
        return cells;
    }
    cells.extend(
        candidates
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| WorkbenchCell {
                kind: WorkbenchCellKind::Session,
                title: format!("history match {}", index + 1),
                detail: format!(
                    "action=enter_to_run_from_search text={}",
                    terminal_inline(&candidate),
                ),
            }),
    );
    cells
}

fn screen_input_command_cells(input_state: &WorkbenchInputState) -> Vec<WorkbenchCell> {
    let query = input_state.completion_query();
    if query.is_empty() {
        return Vec::new();
    }
    let candidates = slash_command_completion_candidates(&query, 6);
    if candidates.is_empty() {
        return vec![WorkbenchCell {
            kind: WorkbenchCellKind::Error,
            title: "command completion".into(),
            detail: format!(
                "query={} candidates=0 action=tab_to_retry command=/commands {}",
                terminal_inline(&query),
                terminal_inline(&query)
            ),
        }];
    }
    let mut cells = vec![WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "command completion".into(),
        detail: format!(
            "query={} candidates={} selected={} tab=cycle enter=run command=/commands {}",
            terminal_inline(&query),
            candidates.len(),
            terminal_inline(
                input_state
                    .completion_selected()
                    .unwrap_or(candidates[0].name)
            ),
            terminal_inline(&query)
        ),
    }];
    cells.extend(candidates.into_iter().map(|candidate| WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: format!("command {}", candidate.name),
        detail: format!(
            "usage={} args={} effect={} command={} summary={}",
            terminal_inline(candidate.usage),
            candidate.argument_model,
            candidate.effect,
            terminal_inline(candidate.name),
            terminal_inline(candidate.summary)
        ),
    }));
    cells.extend(screen_command_palette_cells(Some(&query), 6));
    cells
}

fn screen_command_palette_cells(query: Option<&str>, limit: usize) -> Vec<WorkbenchCell> {
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

fn screen_active_work_cells(
    progress: Option<&WorkbenchProgressSnapshot>,
    pending_approvals: usize,
    continuations: usize,
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
        &progress.detail,
    ));
    vec![WorkbenchCell {
        kind: progress_cell_kind(&progress.status),
        title: title.into(),
        detail,
    }]
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
        "workspace-inspection" | "workspace-mutation" => WorkbenchCellKind::Coding,
        "provider-probe" => WorkbenchCellKind::Model,
        "queue-mutation" | "interrupt" => WorkbenchCellKind::Continuation,
        "config-mutation" | "agent-mutation" | "session-mutation" => WorkbenchCellKind::Session,
        _ => WorkbenchCellKind::Session,
    }
}

fn screen_notice_cells(runtime: &InteractiveChatRuntime, limit: usize) -> Vec<WorkbenchCell> {
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

fn screen_side_notice_cells(runtime: &InteractiveChatRuntime, limit: usize) -> Vec<WorkbenchCell> {
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

fn screen_attachment_status_cell(runtime: &InteractiveChatRuntime) -> WorkbenchCell {
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

fn screen_bottom_pane_status_cell(
    pending: &[ikaros_harness::ApprovalRecord],
    continuations: &[ikaros_session::SessionContinuation],
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
            ikaros_session::SessionContinuationStatus::Queued
                | ikaros_session::SessionContinuationStatus::Running
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

fn screen_attachment_cells(runtime: &InteractiveChatRuntime) -> Vec<WorkbenchCell> {
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

fn print_screen_provider_health_snapshot(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let record = ProviderHealthLedger::new(&paths.audit_dir)
        .latest(&runtime.model_config.provider, &runtime.model_config.model)?;
    let cell = screen_provider_health_cell(record.as_ref());
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

pub(super) fn screen_progress_status_cell(
    progress: Option<&WorkbenchProgressSnapshot>,
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
                &progress.detail,
            )
        ),
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

fn progress_footer_summary(progress: Option<&WorkbenchProgressSnapshot>) -> String {
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

fn progress_recovery_commands(status: &str, error_kind: Option<&str>, detail: &str) -> String {
    if status == "approval_pending" {
        return " command=/approval approve=/screen approve-selected deny=/screen deny-selected trace=/trace --approval".into();
    }
    match error_kind {
        Some("budget_exceeded") => format!(
            " command=/status budget=/budget raise={} disable=/budget disable trace=/trace --failed",
            terminal_inline(
                &crate::chat::suggested_budget_command(detail)
                    .unwrap_or_else(|| "/budget set <tokens>".into())
            )
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

fn workbench_screen_dimensions() -> (usize, usize) {
    let columns = std::env::var("COLUMNS").ok();
    let lines = std::env::var("LINES").ok();
    workbench_screen_dimensions_from_values(columns.as_deref(), lines.as_deref())
}

#[cfg(test)]
pub(super) fn workbench_screen_dimensions_from_values(
    columns: Option<&str>,
    lines: Option<&str>,
) -> (usize, usize) {
    workbench_screen_dimensions_from_values_impl(columns, lines)
}

#[cfg(not(test))]
fn workbench_screen_dimensions_from_values(
    columns: Option<&str>,
    lines: Option<&str>,
) -> (usize, usize) {
    workbench_screen_dimensions_from_values_impl(columns, lines)
}

fn workbench_screen_dimensions_from_values_impl(
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

fn screen_conversation_cells(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<Vec<WorkbenchCell>> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    for state_db in state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            return Ok(screen_conversation_cells_from_replay(&replay, 4));
        }
    }
    Ok(Vec::new())
}

fn screen_conversation_cells_from_replay(
    replay: &SessionReplay,
    limit: usize,
) -> Vec<WorkbenchCell> {
    let mut entries = replay
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.kind,
                SessionEntryKind::UserMessage | SessionEntryKind::AssistantMessage
            )
        })
        .rev()
        .take(limit.max(1))
        .collect::<Vec<_>>();
    entries.reverse();
    entries
        .into_iter()
        .map(screen_conversation_cell)
        .collect::<Vec<_>>()
}

fn screen_conversation_cell(entry: &SessionEntry) -> WorkbenchCell {
    let role = match entry.kind {
        SessionEntryKind::AssistantMessage => "assistant",
        SessionEntryKind::UserMessage => "user",
        _ => "entry",
    };
    let text = entry_visible_text(entry);
    let detail = if matches!(entry.kind, SessionEntryKind::AssistantMessage) {
        render_terminal_markdown(&text)
    } else {
        terminal_message(&text)
    };
    WorkbenchCell {
        kind: match entry.kind {
            SessionEntryKind::AssistantMessage => WorkbenchCellKind::Model,
            _ => WorkbenchCellKind::Session,
        },
        title: format!(
            "{} turn={}",
            role,
            entry
                .turn_id
                .as_ref()
                .map(|turn_id| terminal_inline(turn_id.as_str()))
                .unwrap_or_else(|| "none".into())
        ),
        detail,
    }
}

fn entry_visible_text(entry: &SessionEntry) -> String {
    entry
        .visible_text
        .clone()
        .or_else(|| {
            entry
                .payload
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "none".into())
}

fn screen_sandbox_cell(config: &IkarosConfig) -> WorkbenchCell {
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

fn screen_state_db_cell(runtime: &InteractiveChatRuntime) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "state db".into(),
        detail: format!(
            "path={} command=/debug state-db dump=/debug dump logs=/debug logs",
            terminal_inline(&runtime.state_dir.join("state.db").display().to_string())
        ),
    }
}

fn screen_observability_cell() -> WorkbenchCell {
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
