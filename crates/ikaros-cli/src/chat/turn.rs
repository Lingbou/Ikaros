// SPDX-License-Identifier: GPL-3.0-only

use super::{
    errors::{interactive_chat_turn_error_kind, print_interactive_chat_turn_error},
    interactive::InteractiveChatRuntime,
    live::{
        WorkbenchLiveEventSink, emit_interactive_chat_turn_failure_evidence, print_live_event_cells,
    },
    notice::{WorkbenchNotice, WorkbenchNoticeKind},
    output::print_chat_content,
    progress::print_workbench_progress,
    screen::refresh_persistent_workbench_screen,
    workbench,
};
use anyhow::Result;
use crossterm::{
    event::{self, Event as CrosstermEvent, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, size as terminal_size},
};
use ikaros_core::{IkarosConfig, IkarosPaths, redact_secrets};
use ikaros_harness::SkillRegistry;
use ikaros_models::ModelUsageLedger;
use ikaros_runtime::{
    ChatRunOptions, ChatTurnEventOptions, ChatTurnReport, run_chat_turn_with_events,
};
use ikaros_session::{
    AgentEventSink, FanoutAgentEventSink, PersistingAgentTurnSink, SessionId, SessionSource,
    SessionStore, SqliteSessionStore, TurnId,
};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

pub(in crate::chat) async fn run_and_print_interactive_chat_turn_or_continue(
    input: &str,
    ctx: &InteractiveChatTurnContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    mut fullscreen_terminal: Option<&mut workbench::PersistentWorkbenchTerminal>,
) -> Result<bool> {
    let started = Instant::now();
    let pending_approvals_before = runtime.session.pending_approvals()?.len();
    let mut turn_options = options.clone();
    let mut consumed_content_blocks = Vec::new();
    if !runtime.pending_content_blocks.is_empty() {
        consumed_content_blocks = std::mem::take(&mut runtime.pending_content_blocks);
        let agent_loop_requested = turn_options.agent_loop;
        turn_options.content_blocks = consumed_content_blocks.clone();
        turn_options.agent_loop = false;
        if !runtime.fullscreen_stdout_quiet() {
            println!(
                "chat_attachments: sending count={} agent_loop_requested={} effective_agent_loop=false",
                turn_options.content_blocks.len(),
                agent_loop_requested
            );
        }
    }
    let use_fullscreen_pump = runtime.persistent_fullscreen
        && runtime.screen_state.fullscreen()
        && fullscreen_terminal.is_some();
    let initial_progress = WorkbenchProgressUpdate {
        kind: "chat_turn",
        status: "running",
        elapsed_ms: Some(0),
        detail: Some(input),
        error_kind: None,
    };
    if use_fullscreen_pump {
        print_workbench_progress(
            runtime,
            &turn_options,
            initial_progress.kind,
            initial_progress.status,
            initial_progress.elapsed_ms,
            initial_progress.detail,
            initial_progress.error_kind,
        );
    } else {
        print_and_refresh_workbench_progress(
            ctx,
            runtime,
            &turn_options,
            initial_progress,
            fullscreen_terminal.as_deref_mut(),
        )?;
    }
    let turn_result = if use_fullscreen_pump {
        run_and_print_interactive_chat_turn_with_fullscreen_pump(
            input,
            ctx,
            runtime,
            &turn_options,
            started,
            fullscreen_terminal
                .as_deref_mut()
                .expect("checked fullscreen terminal"),
        )
        .await
    } else {
        let _raw_mode_suspend = RawModeSuspendGuard::new(
            runtime.persistent_fullscreen && runtime.screen_state.fullscreen(),
        );
        run_and_print_interactive_chat_turn(
            input,
            ctx.persona,
            runtime,
            ctx.registry,
            &turn_options,
        )
        .await
    };
    match turn_result {
        Ok(_) => {
            let pending_approvals_after = runtime.session.pending_approvals()?.len();
            if pending_approvals_after > pending_approvals_before {
                print_and_refresh_workbench_progress(
                    ctx,
                    runtime,
                    &turn_options,
                    WorkbenchProgressUpdate {
                        kind: "chat_turn",
                        status: "approval_pending",
                        elapsed_ms: Some(started.elapsed().as_millis()),
                        detail: Some(&format!(
                            "pending_approvals={pending_approvals_after} new_approvals={}",
                            pending_approvals_after.saturating_sub(pending_approvals_before)
                        )),
                        error_kind: None,
                    },
                    fullscreen_terminal.as_deref_mut(),
                )?;
                return Ok(true);
            }
            print_and_refresh_workbench_progress(
                ctx,
                runtime,
                &turn_options,
                WorkbenchProgressUpdate {
                    kind: "chat_turn",
                    status: "completed",
                    elapsed_ms: Some(started.elapsed().as_millis()),
                    detail: None,
                    error_kind: None,
                },
                fullscreen_terminal.as_deref_mut(),
            )?;
            Ok(true)
        }
        Err(error) => {
            if !consumed_content_blocks.is_empty() && runtime.pending_content_blocks.is_empty() {
                runtime.pending_content_blocks = consumed_content_blocks;
                if !runtime.fullscreen_stdout_quiet() {
                    println!(
                        "chat_attachments_restored: pending={}",
                        runtime.pending_content_blocks.len()
                    );
                }
                runtime.push_notice(WorkbenchNotice::new(
                    WorkbenchNoticeKind::Context,
                    "attachments restored",
                    "turn failed before consuming pending multimodal attachments",
                ));
            }
            let error_kind = interactive_chat_turn_error_kind(&error.to_string());
            print_and_refresh_workbench_progress(
                ctx,
                runtime,
                &turn_options,
                WorkbenchProgressUpdate {
                    kind: "chat_turn",
                    status: "failed",
                    elapsed_ms: Some(started.elapsed().as_millis()),
                    detail: Some(&error.to_string()),
                    error_kind: Some(error_kind),
                },
                fullscreen_terminal.as_deref_mut(),
            )?;
            print_interactive_chat_turn_error(runtime, &error);
            runtime.push_notice(WorkbenchNotice::error(
                "chat turn failed",
                &error.to_string(),
            ));
            Ok(false)
        }
    }
}

struct RawModeSuspendGuard {
    restore: bool,
}

impl RawModeSuspendGuard {
    fn new(restore: bool) -> Self {
        if restore {
            let _ = disable_raw_mode();
        }
        Self { restore }
    }
}

impl Drop for RawModeSuspendGuard {
    fn drop(&mut self) {
        if self.restore {
            let _ = enable_raw_mode();
        }
    }
}

async fn run_and_print_interactive_chat_turn_with_fullscreen_pump(
    input: &str,
    ctx: &InteractiveChatTurnContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    started: Instant,
    terminal: &mut workbench::PersistentWorkbenchTerminal,
) -> Result<ChatTurnReport> {
    let mut cached_screen =
        workbench::build_screen_status(workbench::WorkbenchScreenStatusContext {
            config: ctx.config,
            paths: ctx.paths,
            workspace: ctx.workspace,
            runtime,
            options,
            usage_ledger: ctx.usage_ledger,
        })?;
    workbench::apply_pending_user_input_to_cached_screen(&mut cached_screen, input);
    let running_progress = runtime.last_progress.clone();
    let mut screen_state = runtime.screen_state.clone();
    let cancellation = options.cancellation.clone();
    let live_sink = WorkbenchLiveEventSink::with_stdout_updates(false);
    let turn = run_and_print_interactive_chat_turn_with_live_sink(
        input,
        ctx.persona,
        runtime,
        ctx.registry,
        options,
        &live_sink,
    );
    tokio::pin!(turn);
    terminal.draw(&cached_screen, &screen_state)?;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            result = &mut turn => return result,
            _ = tick.tick() => {
                let (width, height) = fullscreen_terminal_size();
                let _changed = drain_fullscreen_running_events(
                    &mut screen_state,
                    &cancellation,
                    &cached_screen,
                    width,
                    height,
                )?;
                if let Some(mut progress) = running_progress.clone() {
                    progress.elapsed_ms = Some(started.elapsed().as_millis());
                    if cancellation.is_cancelled() {
                        progress.status = "cancelled".into();
                        progress.detail = "cancel requested".into();
                    }
                    workbench::apply_progress_to_cached_screen(&mut cached_screen, &progress);
                }
                let live_events = live_sink.events()?;
                workbench::apply_live_model_stream_to_cached_screen(&mut cached_screen, &live_events);
                terminal.draw(&cached_screen, &screen_state)?;
            }
        }
    }
}

fn drain_fullscreen_running_events(
    screen_state: &mut workbench::WorkbenchScreenState,
    cancellation: &ikaros_harness::CancellationToken,
    screen: &workbench::WorkbenchScreen,
    width: usize,
    height: usize,
) -> Result<bool> {
    let mut changed = false;
    while event::poll(Duration::from_millis(0))? {
        match event::read()? {
            CrosstermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Esc
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('c' | 'C')))
                {
                    cancellation.cancel();
                    changed = true;
                    continue;
                }
                changed |= workbench::apply_workbench_screen_key_event_with_view(
                    screen_state,
                    key,
                    Some(screen),
                    width,
                    height,
                );
            }
            CrosstermEvent::Mouse(mouse) => {
                changed |= workbench::apply_workbench_screen_mouse_event_with_view(
                    screen_state,
                    mouse,
                    Some(screen),
                    width,
                    height,
                );
            }
            CrosstermEvent::Resize(_, _) => {
                changed = true;
            }
            _ => {}
        }
    }
    Ok(changed)
}

fn fullscreen_terminal_size() -> (usize, usize) {
    terminal_size()
        .map(|(width, height)| (usize::from(width), usize::from(height)))
        .unwrap_or((80, 24))
}

pub(in crate::chat) struct InteractiveChatTurnContext<'a> {
    pub(in crate::chat) config: &'a IkarosConfig,
    pub(in crate::chat) paths: &'a IkarosPaths,
    pub(in crate::chat) workspace: &'a Path,
    pub(in crate::chat) persona: &'a ikaros_soul::PersonaProfile,
    pub(in crate::chat) registry: &'a SkillRegistry,
    pub(in crate::chat) usage_ledger: &'a ModelUsageLedger,
}

struct WorkbenchProgressUpdate<'a> {
    kind: &'a str,
    status: &'a str,
    elapsed_ms: Option<u128>,
    detail: Option<&'a str>,
    error_kind: Option<&'a str>,
}

fn print_and_refresh_workbench_progress(
    ctx: &InteractiveChatTurnContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    update: WorkbenchProgressUpdate<'_>,
    fullscreen_terminal: Option<&mut workbench::PersistentWorkbenchTerminal>,
) -> Result<()> {
    print_workbench_progress(
        runtime,
        options,
        update.kind,
        update.status,
        update.elapsed_ms,
        update.detail,
        update.error_kind,
    );
    if runtime.persistent_fullscreen && runtime.screen_state.fullscreen() {
        refresh_persistent_workbench_screen(
            ctx.config,
            ctx.paths,
            ctx.workspace,
            runtime,
            options,
            ctx.usage_ledger,
            fullscreen_terminal,
        )?;
    }
    Ok(())
}

async fn run_and_print_interactive_chat_turn(
    input: &str,
    persona: &ikaros_soul::PersonaProfile,
    runtime: &mut InteractiveChatRuntime,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<ChatTurnReport> {
    let live_sink = WorkbenchLiveEventSink::with_stdout_updates(false);
    run_and_print_interactive_chat_turn_with_live_sink(
        input, persona, runtime, registry, options, &live_sink,
    )
    .await
}

async fn run_and_print_interactive_chat_turn_with_live_sink(
    input: &str,
    persona: &ikaros_soul::PersonaProfile,
    runtime: &mut InteractiveChatRuntime,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
    live_sink: &WorkbenchLiveEventSink,
) -> Result<ChatTurnReport> {
    let emit_stdout = !runtime.fullscreen_stdout_quiet();
    if emit_stdout {
        println!(
            "chat_turn: started session={} stream={} agent_loop={} effective_agent_loop={} content_blocks={}",
            redact_secrets(&runtime.chat_session_id),
            options.stream,
            options.agent_loop,
            options.agent_loop && options.content_blocks.is_empty(),
            options.content_blocks.len()
        );
    }
    let output =
        run_interactive_chat_turn(input, persona, runtime, registry, options, live_sink).await?;
    let report = output.report;
    if emit_stdout {
        print_interactive_chat_report(&report)?;
    }
    print_live_event_cells(runtime, &output.turn_id)?;
    if emit_stdout {
        println!(
            "chat_turn: completed session={} streamed={} stream_chunks={}",
            redact_secrets(&runtime.chat_session_id),
            report.streamed,
            report.stream_chunks.len()
        );
    }
    Ok(report)
}

struct InteractiveChatTurnOutput {
    report: ChatTurnReport,
    turn_id: TurnId,
}

async fn run_interactive_chat_turn(
    input: &str,
    persona: &ikaros_soul::PersonaProfile,
    runtime: &mut InteractiveChatRuntime,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
    live_sink: &WorkbenchLiveEventSink,
) -> Result<InteractiveChatTurnOutput> {
    let chat_session_id = options
        .session_id
        .clone()
        .unwrap_or_else(|| runtime.chat_session_id.clone());
    let session_id = SessionId::from(chat_session_id);
    let turn_id = TurnId::new();
    let session_store: Arc<dyn SessionStore> =
        Arc::new(SqliteSessionStore::new(&runtime.state_dir));
    let parent_entry_id = session_store
        .get_session(&session_id)?
        .and_then(|session| session.active_leaf_entry_id);
    let event_sink = PersistingAgentTurnSink::new(session_store)
        .with_source(options.session_source.clone().unwrap_or(SessionSource::Cli))
        .with_agent_id(runtime.agent_id.clone())
        .with_workspace(runtime.workspace.clone());
    let fanout_event_sink =
        FanoutAgentEventSink::new([&event_sink as &dyn AgentEventSink, live_sink]);
    let report = match run_chat_turn_with_events(
        input,
        persona,
        runtime.provider.as_ref(),
        &runtime.agent,
        &runtime.session,
        registry,
        ChatTurnEventOptions {
            options,
            request_options: Some(&runtime.request_options),
            event_sink: &fanout_event_sink,
            session_sink: Some(&event_sink),
            parent_entry_id,
            turn_id: Some(turn_id.clone()),
        },
    )
    .await
    {
        Ok(report) => report,
        Err(error) => {
            let error: anyhow::Error = error.into();
            let _ = emit_interactive_chat_turn_failure_evidence(
                &fanout_event_sink,
                live_sink,
                &session_id,
                &turn_id,
                &error,
            );
            if event_sink.commit().is_err() {
                let _ = event_sink.rollback();
            }
            return Err(error);
        }
    };
    event_sink.commit()?;
    Ok(InteractiveChatTurnOutput { report, turn_id })
}

fn print_interactive_chat_report(report: &ChatTurnReport) -> Result<()> {
    println!(
        "context: relationship={} references={} history={} memory={} rag={} relationship_candidates_created={}",
        report.relationship_hits,
        report.reference_hits,
        report.history_hits,
        report.memory_hits,
        report.rag_hits,
        report.relationship_candidates_created
    );
    if report.streamed {
        println!("chat_stream: start");
        println!("stream_chunks: {}", report.stream_chunks.len());
    }
    print_chat_content(report)?;
    if report.streamed {
        println!("chat_stream: done");
    }
    Ok(())
}
