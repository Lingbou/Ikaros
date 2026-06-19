// SPDX-License-Identifier: GPL-3.0-only
//! Runtime coordination over config, harness sessions, skills, and task execution.

mod agent;
mod agent_harness;
mod agent_loop;
mod body;
mod chat;
mod diagnostics;
mod emotion;
mod environment;
mod message;
mod persona;
mod relationship;
mod schedule;
mod session;
mod task;

pub use agent::{
    AgentHandoffReport, AgentPoolItemReport, AgentPoolReport, AgentPoolTask, run_agent_handoff,
    run_agent_handoff_with_options, run_agent_pool, run_agent_pool_with_options,
};
pub use agent_harness::{
    AgentHarness, AgentHarnessConfig, AgentHarnessMessage, AgentHarnessPendingCounts,
    AgentHarnessPhase, AgentHarnessTurn,
};
pub use agent_loop::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopHookEvent,
    AgentLoopHooks, AgentLoopInput, AgentLoopOptions, AgentLoopReport, AgentLoopStopReason,
    AgentLoopToolCall, AgentLoopToolCallDiagnostic, AgentLoopToolCallParseStrategy,
    AgentLoopToolDefinition, AgentLoopToolResult, AgentRuntime, HarnessAgentRuntime,
    RecordingAgentRuntime, agent_loop_tool_definitions, noop_agent_event_sink, run_agent_loop,
    run_agent_loop_with_events,
};
pub use body::{
    audit_event_to_body_event, audit_event_to_body_event_for_body, base_body_status,
    body_event_kind_from_audit, current_body_frame,
};
pub use chat::{
    ChatContext, ChatHistoryRecord, ChatHistorySessionSummary, ChatHistoryStore, ChatMessageResult,
    ChatRunOptions, ChatTurnEventOptions, ChatTurnReport, CompactInput, CompactReport,
    ContextAssembleInput, ContextBundle, ContextEngine, ContextEvent, ContextModelBudget,
    DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET, LocalChatContextEngine, TurnRecord, build_chat_context,
    build_chat_context_bundle_with_engine, build_chat_context_bundle_with_model_context,
    build_chat_context_with_engine, context_lookup_is_safe_read, extract_rag_context,
    extract_retrieved_memory_context, new_chat_session_id, render_chat_system_prompt,
    render_persona_agent_context, run_chat_message, run_chat_turn, run_chat_turn_with_events,
};
pub use diagnostics::{
    AgentSummary, AutomationSummary, GatewaySummary, ModelSummary, PersonaSummary, PluginSummary,
    RagSummary, RuntimeDoctorReport, RuntimeInitReport, StoreSummary, VoiceSummary,
    initialize_runtime_home, runtime_doctor_report,
};
pub use emotion::{
    EMOTION_EVENT_KIND, latest_emotion_from_events, parse_emotion_state, record_emotion_signal,
};
pub use environment::{
    RuntimeHarness, recent_policy_decisions, resolve_agent, resolve_agent_instance,
    session_and_registry, session_and_registry_for_agent, session_and_registry_for_instance,
    skill_environment,
};
pub use message::{
    GatewayDrainContext, GatewayDrainReport, GatewayWorkerTickReport, drain_gateway_message,
    drain_gateway_messages, run_gateway_worker_tick,
};
pub use persona::{
    PersonaPatch, PersonaWriteReport, render_persona_markdown, reset_persona, update_persona,
};
pub use relationship::{
    RelationshipMutationReport, RelationshipNote, RelationshipSnapshot,
    forget_relationship_note_by_id, forget_relationship_scope, relationship_context_lines,
    relationship_snapshot, relationship_snapshot_from_session, remember_relationship_note,
};
pub use schedule::{
    ScheduleDeliveryReport, ScheduleWorkerTickReport, ScheduledJobRunReport, run_due_jobs,
    run_schedule_worker_tick, run_scheduled_job,
};
pub use session::record_approval_resolution;
pub use task::{
    RuntimeTaskExecution, RuntimeTaskPlan, TaskRunOptions, build_task_plan,
    execute_task_for_automation, execute_task_text, execute_task_text_with_options,
    task_report_succeeded, task_report_summary, task_steps,
};
