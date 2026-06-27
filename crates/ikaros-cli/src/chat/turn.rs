// SPDX-License-Identifier: GPL-3.0-only

use super::{
    errors::{interactive_chat_turn_error_kind, print_interactive_chat_turn_error},
    interactive::InteractiveChatRuntime,
    live::{
        WorkbenchLiveEventSink, emit_interactive_chat_turn_failure_evidence, print_live_event_cells,
    },
    notice::{WorkbenchNotice, WorkbenchNoticeKind},
    output::{print_chat_content, print_chat_content_for_human_transcript},
    progress::print_workbench_progress,
    terminal::{RunningTurnInputCapture, print_inline_turn_worked_separator},
};
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths, redact_secrets};
use ikaros_harness::SkillRegistry;
use ikaros_memory::{
    JsonlMemoryJournal, LocalMemoryStore, MemoryProvider, MemoryTurnRecord, MemoryTurnStart,
};
use ikaros_runtime::{
    ChatRunOptions, ChatTurnEventOptions, ChatTurnReport, apply_chat_memory_policy,
    chat_memory_policy_from_config, emit_chat_memory_lifecycle_report, run_chat_turn_with_events,
};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, FanoutAgentEventSink,
    PersistingAgentTurnSink, SessionId, SessionSource, SessionStore, SqliteSessionStore, TurnId,
};
use serde_json::json;
use std::{sync::Arc, time::Instant};

pub(in crate::chat) async fn run_and_print_interactive_chat_turn_or_continue(
    input: &str,
    ctx: &InteractiveChatTurnContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
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
        if !runtime.machine_stdout_quiet() {
            println!(
                "chat_attachments: sending count={} agent_loop_requested={} effective_agent_loop=false",
                turn_options.content_blocks.len(),
                agent_loop_requested
            );
        }
    }
    let initial_progress = WorkbenchProgressUpdate {
        kind: "chat_turn",
        status: "running",
        elapsed_ms: Some(0),
        detail: Some(input),
        error_kind: None,
    };
    print_and_refresh_workbench_progress(ctx, runtime, &turn_options, initial_progress)?;
    let turn_result = run_and_print_interactive_chat_turn(input, ctx, runtime, &turn_options).await;
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

pub(in crate::chat) struct InteractiveChatTurnContext<'a> {
    pub(in crate::chat) config: &'a IkarosConfig,
    pub(in crate::chat) paths: &'a IkarosPaths,
    pub(in crate::chat) persona: &'a ikaros_soul::PersonaProfile,
    pub(in crate::chat) registry: &'a SkillRegistry,
}

struct WorkbenchProgressUpdate<'a> {
    kind: &'a str,
    status: &'a str,
    elapsed_ms: Option<u128>,
    detail: Option<&'a str>,
    error_kind: Option<&'a str>,
}

fn print_and_refresh_workbench_progress(
    _ctx: &InteractiveChatTurnContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    update: WorkbenchProgressUpdate<'_>,
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
    Ok(())
}

async fn run_and_print_interactive_chat_turn(
    input: &str,
    ctx: &InteractiveChatTurnContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<ChatTurnReport> {
    // Default `ikaros` sets `options.stream = true` before entering the interactive loop.
    // For OpenAI-compatible providers that reaches `provider.stream_with_events`, which
    // sends `stream: true` to `/chat/completions` and forwards SSE content deltas here.
    // Non-streaming turns still emit synthetic model stream events for the timeline, so
    // keep stdout deltas gated on the requested stream path and let the final human
    // transcript renderer handle generated responses once.
    let running_input = runtime
        .default_inline_stdout()
        .then(RunningTurnInputCapture::start)
        .flatten();
    let live_sink = WorkbenchLiveEventSink::with_text_delta_stdout_and_terminal(
        should_emit_text_delta_stdout(runtime.default_inline_stdout(), options.stream),
        running_input
            .as_ref()
            .map(RunningTurnInputCapture::terminal),
    );
    run_and_print_interactive_chat_turn_with_live_sink(
        input,
        ctx,
        runtime,
        options,
        &live_sink,
        running_input,
    )
    .await
}

fn should_emit_text_delta_stdout(default_inline_stdout: bool, stream_requested: bool) -> bool {
    default_inline_stdout && stream_requested
}

fn queue_running_turn_inputs(runtime: &mut InteractiveChatRuntime, inputs: Vec<String>) {
    for input in inputs {
        if input.trim().is_empty() {
            continue;
        }
        runtime.pending_inputs.push_back(input);
    }
    if !runtime.pending_inputs.is_empty() {
        runtime.push_notice(WorkbenchNotice::new(
            WorkbenchNoticeKind::Continuation,
            "pending input",
            &format!("queued={}", runtime.pending_inputs.len()),
        ));
    }
}

async fn run_and_print_interactive_chat_turn_with_live_sink(
    input: &str,
    ctx: &InteractiveChatTurnContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    live_sink: &WorkbenchLiveEventSink,
    running_input: Option<RunningTurnInputCapture>,
) -> Result<ChatTurnReport> {
    let emit_machine_stdout = !runtime.machine_stdout_quiet();
    let emit_human_stdout = runtime.default_inline_stdout();
    if emit_machine_stdout {
        println!(
            "chat_turn: started session={} stream={} agent_loop={} effective_agent_loop={} content_blocks={}",
            redact_secrets(&runtime.chat_session_id),
            options.stream,
            options.agent_loop,
            options.agent_loop && options.content_blocks.is_empty(),
            options.content_blocks.len()
        );
    }
    let started = Instant::now();
    let output = run_interactive_chat_turn(input, ctx, runtime, options, live_sink).await;
    if let Some(capture) = running_input {
        queue_running_turn_inputs(runtime, capture.finish());
    }
    let output = output?;
    let report = output.report;
    if emit_human_stdout {
        print_interactive_chat_report_for_human(&report, started.elapsed().as_millis())?;
    } else if emit_machine_stdout {
        print_interactive_chat_report(&report)?;
    }
    print_live_event_cells(runtime, &output.turn_id)?;
    if emit_machine_stdout {
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
    ctx: &InteractiveChatTurnContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    live_sink: &WorkbenchLiveEventSink,
) -> Result<InteractiveChatTurnOutput> {
    let chat_session_id = options
        .session_id
        .clone()
        .unwrap_or_else(|| runtime.chat_session_id.clone());
    let session_id = SessionId::from(chat_session_id.clone());
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
    let memory_lifecycle = InteractiveMemoryLifecycle::new(ctx, runtime, chat_session_id, input)?;
    memory_lifecycle.emit_turn_start(&fanout_event_sink, &session_id, &turn_id)?;
    let report = match run_chat_turn_with_events(
        input,
        ctx.persona,
        runtime.provider.as_ref(),
        &runtime.agent,
        &runtime.session,
        ctx.registry,
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
    memory_lifecycle.sync_after_turn(&fanout_event_sink, &session_id, &turn_id, &report, runtime);
    event_sink.commit()?;
    Ok(InteractiveChatTurnOutput { report, turn_id })
}

struct InteractiveMemoryLifecycle {
    provider: LocalMemoryStore,
    journal: JsonlMemoryJournal,
    policy: ikaros_memory::MemoryPolicy,
    agent_id: String,
    chat_session_id: String,
    input: String,
}

impl InteractiveMemoryLifecycle {
    fn new(
        ctx: &InteractiveChatTurnContext<'_>,
        runtime: &InteractiveChatRuntime,
        chat_session_id: String,
        input: &str,
    ) -> Result<Self> {
        Ok(Self {
            provider: LocalMemoryStore::new(&ctx.paths.memory_dir, &ctx.config.memory.backend)?,
            journal: JsonlMemoryJournal::new(&ctx.paths.memory_dir),
            policy: chat_memory_policy_from_config(&ctx.config.memory.policy),
            agent_id: runtime.agent_id.clone(),
            chat_session_id,
            input: redact_secrets(input),
        })
    }

    fn emit_turn_start(
        &self,
        sink: &dyn AgentEventSink,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) -> Result<()> {
        let report = self.provider.turn_start(MemoryTurnStart {
            session_id: Some(self.chat_session_id.clone()),
            agent_id: Some(self.agent_id.clone()),
            user_input: self.input.clone(),
        })?;
        emit_chat_memory_lifecycle_report(
            sink,
            session_id,
            turn_id,
            &self.agent_id,
            &self.chat_session_id,
            &report,
        )?;
        Ok(())
    }

    fn sync_after_turn(
        &self,
        sink: &dyn AgentEventSink,
        session_id: &SessionId,
        turn_id: &TurnId,
        report: &ChatTurnReport,
        runtime: &mut InteractiveChatRuntime,
    ) {
        if let Err(error) = self.sync_after_turn_result(sink, session_id, turn_id, report) {
            let message = error.to_string();
            runtime.push_notice(WorkbenchNotice::error("memory sync failed", &message));
            let _ =
                emit_interactive_memory_failure(sink, session_id, turn_id, "memory_sync", &message);
        }
    }

    fn sync_after_turn_result(
        &self,
        sink: &dyn AgentEventSink,
        session_id: &SessionId,
        turn_id: &TurnId,
        report: &ChatTurnReport,
    ) -> Result<()> {
        let sync_report = self.provider.sync_turn(MemoryTurnRecord {
            session_id: report
                .chat_session_id
                .clone()
                .or_else(|| Some(self.chat_session_id.clone())),
            turn_id: Some(turn_id.to_string()),
            agent_id: Some(self.agent_id.clone()),
            user_input: self.input.clone(),
            assistant_output: report.response.content.clone(),
        })?;
        emit_chat_memory_lifecycle_report(
            sink,
            session_id,
            turn_id,
            &self.agent_id,
            &self.chat_session_id,
            &sync_report,
        )?;
        apply_chat_memory_policy(&self.provider, &self.journal, &self.policy, &sync_report)?;
        Ok(())
    }
}

fn emit_interactive_memory_failure(
    sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    phase: &str,
    error: &str,
) -> Result<()> {
    sink.emit(&AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Memory,
        AgentEventKind::Error,
        json!({
            "phase": phase,
            "message": redact_secrets(error),
        }),
    ))?;
    Ok(())
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
        println!("stream_chunks: {}", report.stream_chunks.len());
    }
    print_chat_content(report)?;
    Ok(())
}

fn print_interactive_chat_report_for_human(
    report: &ChatTurnReport,
    elapsed_ms: u128,
) -> Result<()> {
    // Streamed turns have already written readable TextDelta output to stdout. Rendering the
    // complete assistant message here would duplicate the answer; non-streamed turns use the
    // same assistant markdown renderer so they do not fall back to raw/debug transcript text.
    print_chat_content_for_human_transcript(report, report.streamed)?;
    print_inline_turn_worked_separator(elapsed_ms)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_delta_stdout_requires_default_inline_ui_and_stream_request() {
        assert!(should_emit_text_delta_stdout(true, true));
        assert!(!should_emit_text_delta_stdout(true, false));
        assert!(!should_emit_text_delta_stdout(false, true));
        assert!(!should_emit_text_delta_stdout(false, false));
    }
}
