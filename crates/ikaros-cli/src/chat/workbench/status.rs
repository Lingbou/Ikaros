// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::{InteractiveChatRuntime, format_interactive_chat_status};
use crate::resolve_agent_instance;
use anyhow::Result;
use ikaros_automation::LocalScheduleStore;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_gateway::{GatewayMessage, GatewayMessageStatus, LocalGatewayStore};
use ikaros_harness::{ApprovalRecord, ApprovalStatus, ProcessRequest};
use ikaros_memory::{JsonlMemoryJournal, MemoryJournal};
use ikaros_models::{ModelUsageLedger, ProviderHealthLedger};
use ikaros_runtime::{
    ChatHistorySessionSummary, ChatHistoryStore, ChatRunOptions, base_body_status,
    gateway_session_id,
};
use ikaros_session::{AgentEventKind, SessionId, SessionReplay, SessionStore, SqliteSessionStore};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use super::{
    WorkbenchCell, WorkbenchCellKind, agent_event_cell, coding_event_cells, path_display,
    session_entry_cell, terminal_inline,
};

pub(in crate::chat) fn print_workbench_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    print_session_status(config, paths, runtime, options)?;
    print_unified_status(config, paths, workspace, runtime)?;
    let history_store =
        ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let body_status = base_body_status(paths)?;
    println!(
        "{}",
        format_interactive_chat_status(
            &runtime.agent,
            &runtime.session,
            &runtime.chat_session_id,
            options,
            &body_status.emotion,
            usage_ledger,
            &history_store,
        )
    );
    Ok(())
}

fn print_unified_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let model = &config.model.default;
    let provider_health = ProviderHealthLedger::new(&paths.audit_dir)
        .latest(&model.provider, &model.model)?
        .map(|record| format!("{:?}", record.status))
        .unwrap_or_else(|| "Unknown".into());
    let gateway_pending = LocalGatewayStore::new(&paths.gateway_dir)
        .list()?
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Pending)
        .count();
    let approvals_pending = runtime.session.pending_approvals()?.len();
    let continuations = continuation_count(config, paths, workspace, runtime)?;
    println!(
        "status_model: provider={} model={} runtime={} transport={} health={}",
        terminal_inline(&model.provider),
        terminal_inline(&model.model),
        terminal_inline(&model.runtime),
        terminal_inline(&model.transport),
        terminal_inline(&provider_health)
    );
    println!("status_workspace: {}", path_display(workspace));
    println!(
        "status_policy: workspace_writes={} shell={} network={}",
        runtime.agent.profile.workspace_writes,
        runtime.agent.profile.shell,
        runtime.agent.profile.network
    );
    println!("status_gateway_pending: {gateway_pending}");
    println!("status_approvals_pending: {approvals_pending}");
    println!("status_continuations: {continuations}");
    Ok(())
}

fn continuation_count(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<usize> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let mut count = 0;
    for state_db in state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if store.get_session(&session_id)?.is_some() {
            count += store.continuations(&session_id)?.len();
        }
    }
    Ok(count)
}

pub(in crate::chat) fn print_session_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<()> {
    let history_store =
        ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let records = history_store.read_session(&runtime.chat_session_id)?;
    println!(
        "workbench_session: {}",
        terminal_inline(&runtime.chat_session_id)
    );
    println!(
        "session_history_records: {} backend={} path={}",
        records.len(),
        history_store.backend_name(),
        path_display(history_store.path())
    );
    println!(
        "session_options: agent_loop={} stream={} no_context={} context_token_budget={} scope={}",
        options.agent_loop,
        options.stream,
        options.no_context,
        options.context_token_budget,
        options
            .scope
            .as_deref()
            .map(terminal_inline)
            .unwrap_or_else(|| "none".into())
    );
    Ok(())
}

pub(in crate::chat) fn print_session_history(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    session_id: &str,
    limit: usize,
) -> Result<()> {
    let history_store =
        ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let records = history_store.read_session(session_id)?;
    println!("session_history: {}", terminal_inline(session_id));
    println!("records: {}", records.len());
    if records.is_empty() {
        println!("recent: none");
        return Ok(());
    }
    println!("recent:");
    let start = records.len().saturating_sub(limit);
    for record in &records[start..] {
        println!(
            "- turn={} provider={} model={} streamed={} user={} assistant={}",
            terminal_inline(&record.turn_id),
            terminal_inline(&record.provider),
            terminal_inline(&record.model),
            record.streamed,
            terminal_inline(&record.user_message),
            terminal_inline(&record.assistant_message)
        );
    }
    Ok(())
}

pub(in crate::chat) fn print_session_summaries(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    limit: usize,
) -> Result<()> {
    let history_store =
        ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let sessions = history_store.session_summaries(limit)?;
    println!("sessions: {}", sessions.len());
    println!("chat_history_backend: {}", history_store.backend_name());
    println!("chat_history: {}", path_display(history_store.path()));
    if sessions.is_empty() {
        println!("recent: none");
        return Ok(());
    }
    for summary in sessions {
        print_session_summary_line(&summary);
    }
    Ok(())
}

fn print_session_summary_line(summary: &ChatHistorySessionSummary) {
    println!(
        "- session={} turns={} first={} last={} last_turn={} agents={} providers={} models={}",
        terminal_inline(&summary.session_id),
        summary.turns,
        summary.first_created_at,
        summary.last_created_at,
        summary.last_turn_id,
        terminal_inline(&summary.agents.join(",")),
        terminal_inline(&summary.providers.join(",")),
        terminal_inline(&summary.models.join(","))
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum TimelineVerbosity {
    Timeline,
    Replay,
    Debug,
}

pub(in crate::chat) fn print_replay_status(
    label: &str,
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    verbosity: TimelineVerbosity,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let candidates = state_db_candidates(config, paths, workspace, runtime)?;
    for state_db in &candidates {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            let continuations = store.continuations(&session_id)?;
            println!("{label}: found");
            println!("session: {}", terminal_inline(session_id.as_str()));
            println!("state_db: {}", path_display(state_db));
            println!("entries: {}", replay.entries.len());
            println!("agent_events: {}", replay.agent_events.len());
            println!("approvals: {}", replay.approvals.len());
            println!("continuations: {}", continuations.len());
            print_recent_timeline(&replay, verbosity);
            return Ok(());
        }
    }
    println!("{label}: not_found");
    println!("session: {}", terminal_inline(session_id.as_str()));
    println!("state_db_candidates: {}", candidates.len());
    Ok(())
}

pub(in crate::chat) fn print_trace_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let candidates = state_db_candidates(config, paths, workspace, runtime)?;
    for state_db in &candidates {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            println!("trace_command: /trace");
            println!("trace: found");
            println!("session: {}", terminal_inline(session_id.as_str()));
            println!("state_db: {}", path_display(state_db));
            print_trace_summary(&replay);
            return Ok(());
        }
    }
    println!("trace_command: /trace");
    println!("trace: not_found");
    println!("session: {}", terminal_inline(session_id.as_str()));
    println!("state_db_candidates: {}", candidates.len());
    Ok(())
}

fn print_trace_summary(replay: &SessionReplay) {
    let mut global_counts = BTreeMap::<&'static str, usize>::new();
    let mut spans = BTreeMap::<String, BTreeMap<&'static str, usize>>::new();
    for event in &replay.agent_events {
        let category = trace_event_category(&event.kind);
        *global_counts.entry(category).or_default() += 1;
        *spans
            .entry(event.turn_id.to_string())
            .or_default()
            .entry(category)
            .or_default() += 1;
    }
    println!("trace_spans: {}", spans.len());
    println!(
        "trace_event_counts: {}",
        format_trace_counts(&global_counts)
    );
    if spans.is_empty() {
        println!("trace_cells: none");
        return;
    }
    println!("trace_cells:");
    let start = spans.len().saturating_sub(5);
    for (turn_id, counts) in spans.into_iter().skip(start) {
        let events = counts.values().sum::<usize>();
        let cell = WorkbenchCell {
            kind: WorkbenchCellKind::Session,
            title: format!("trace span turn={}", terminal_inline(&turn_id)),
            detail: format!("events={events} {}", format_trace_counts(&counts)),
        };
        println!("- {}", cell.render());
    }
}

fn trace_event_category(kind: &AgentEventKind) -> &'static str {
    match kind {
        AgentEventKind::ModelStream(_) => "model",
        AgentEventKind::ToolCallStarted
        | AgentEventKind::ToolCallOutputDelta
        | AgentEventKind::ToolCallCompleted
        | AgentEventKind::ToolCallFailed
        | AgentEventKind::ToolCallCancelled => "tool",
        AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted => "context",
        AgentEventKind::MemoryLifecycle => "memory",
        AgentEventKind::CodingTurn => "coding",
        AgentEventKind::AuditAnchor => "audit",
        AgentEventKind::ContinuationStarted
        | AgentEventKind::ContinuationCompleted
        | AgentEventKind::ContinuationFailed
        | AgentEventKind::ContinuationCancelled => "continuation",
        AgentEventKind::ApprovalRequested | AgentEventKind::ApprovalResolved => "approval",
        AgentEventKind::Error => "error",
        AgentEventKind::SessionStart
        | AgentEventKind::TurnStart
        | AgentEventKind::UserMessage
        | AgentEventKind::TurnEnd => "session",
    }
}

fn format_trace_counts(counts: &BTreeMap<&'static str, usize>) -> String {
    [
        "session",
        "model",
        "tool",
        "context",
        "memory",
        "coding",
        "audit",
        "continuation",
        "approval",
        "error",
    ]
    .into_iter()
    .map(|category| format!("{category}={}", counts.get(category).copied().unwrap_or(0)))
    .collect::<Vec<_>>()
    .join(" ")
}

fn print_recent_timeline(replay: &SessionReplay, verbosity: TimelineVerbosity) {
    let event_limit = match verbosity {
        TimelineVerbosity::Timeline => 5,
        TimelineVerbosity::Replay => 10,
        TimelineVerbosity::Debug => 20,
    };
    let entry_limit = match verbosity {
        TimelineVerbosity::Timeline => 3,
        TimelineVerbosity::Replay | TimelineVerbosity::Debug => 8,
    };
    if !replay.entries.is_empty() {
        println!("recent_entries:");
        let start = replay.entries.len().saturating_sub(entry_limit);
        for entry in &replay.entries[start..] {
            println!("- {}", session_entry_cell(entry).render());
        }
    }
    print_coding_groups(replay);
    if !replay.agent_events.is_empty() {
        println!("recent_events:");
        let start = replay.agent_events.len().saturating_sub(event_limit);
        for event in &replay.agent_events[start..] {
            println!("- {}", agent_event_cell(event).render());
        }
    }
}

fn print_coding_groups(replay: &SessionReplay) {
    let cells = coding_event_cells(&replay.agent_events);
    if cells.is_empty() {
        return;
    }
    for group in ["progress", "diff", "test", "review"] {
        let group_cells = cells
            .iter()
            .filter(|(candidate, _)| *candidate == group)
            .collect::<Vec<_>>();
        if group_cells.is_empty() {
            continue;
        }
        println!("coding_group: {group} count={}", group_cells.len());
        let start = group_cells.len().saturating_sub(3);
        for (_, cell) in &group_cells[start..] {
            println!("- {}", cell.render());
        }
    }
}

fn state_db_candidates(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    let agent = resolve_agent_instance(config, Some(&runtime.agent.name), workspace, &paths.home)?;
    push_state_db_candidate(&mut candidates, &mut seen, agent.state_dir.join("state.db"));
    let agents_dir = paths.home.join("agents");
    if agents_dir.is_dir() {
        for entry in fs::read_dir(&agents_dir)? {
            let entry = entry?;
            push_state_db_candidate(&mut candidates, &mut seen, entry.path().join("state.db"));
        }
    }
    Ok(candidates)
}

fn push_state_db_candidate(
    candidates: &mut Vec<PathBuf>,
    seen: &mut BTreeSet<PathBuf>,
    candidate: PathBuf,
) {
    if seen.insert(candidate.clone()) {
        candidates.push(candidate);
    }
}

pub(in crate::chat) fn print_gateway_status(paths: &IkarosPaths) -> Result<()> {
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let messages = store.list()?;
    let deliveries = store.deliveries()?;
    let pending = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Pending)
        .count();
    let processing = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Processing)
        .count();
    let processed = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Processed)
        .count();
    let failed = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Failed)
        .count();
    println!("gateway_inbox: {}", path_display(store.inbox_path()));
    println!("gateway_outbox: {}", path_display(store.outbox_path()));
    println!("gateway_pending: {pending}");
    println!("gateway_processing: {processing}");
    println!("gateway_processed: {processed}");
    println!("gateway_failed: {failed}");
    println!("gateway_deliveries: {}", deliveries.len());
    print_gateway_sessions(&messages);
    Ok(())
}

fn print_gateway_sessions(messages: &[GatewayMessage]) {
    let mut sessions = messages
        .iter()
        .map(|message| {
            let session_id = gateway_session_id(message);
            (
                session_id.to_string(),
                message.source.as_str(),
                message
                    .session_source
                    .as_ref()
                    .and_then(|source| source.thread.as_deref())
                    .unwrap_or(message.id.as_str()),
                message,
            )
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.0.cmp(&right.0));
    sessions.dedup_by(|left, right| left.0 == right.0);
    println!("gateway_sessions: {}", sessions.len());
    for (session_id, source, thread, message) in sessions.into_iter().rev().take(5) {
        println!(
            "gateway_session: session={} source={} thread={} last_status={:?}",
            terminal_inline(&session_id),
            terminal_inline(source),
            terminal_inline(thread),
            message.status
        );
        println!(
            "  resume: ikaros chat --chat-session {} --message \"...\"",
            terminal_inline(&session_id)
        );
    }
}

pub(in crate::chat) fn print_tasks_status(paths: &IkarosPaths) -> Result<()> {
    let store = LocalScheduleStore::new(&paths.automation_dir);
    let jobs = store.list()?;
    let due = store.due_now()?;
    let enabled = jobs.iter().filter(|job| job.enabled).count();
    let disabled = jobs.len().saturating_sub(enabled);
    println!("tasks_store: {}", path_display(store.path()));
    println!("tasks_total: {}", jobs.len());
    println!("tasks_enabled: {enabled}");
    println!("tasks_disabled: {disabled}");
    println!("tasks_due: {}", due.len());
    Ok(())
}

pub(in crate::chat) fn print_approval_status(runtime: &InteractiveChatRuntime) -> Result<()> {
    let pending = runtime.session.pending_approvals()?;
    let records = runtime.session.approval_records()?;
    let approved = records
        .iter()
        .filter(|record| record.status == ApprovalStatus::Approved)
        .count();
    let rejected = records
        .iter()
        .filter(|record| record.status == ApprovalStatus::Denied)
        .count();
    println!("approvals_pending: {}", pending.len());
    println!("approvals_total: {}", records.len());
    println!("approvals_approved: {approved}");
    println!("approvals_rejected: {rejected}");
    if let Some(log) = runtime.session.approvals.log() {
        println!("approvals_log: {}", path_display(log.path()));
    } else {
        println!("approvals_log: none");
    }
    print_approval_overlay(runtime, &pending);
    Ok(())
}

fn print_approval_overlay(runtime: &InteractiveChatRuntime, pending: &[ApprovalRecord]) {
    if pending.is_empty() {
        println!("approval_overlay: none");
        return;
    }
    println!("approval_overlay:");
    for record in pending {
        let context = record.request.context.as_ref();
        println!(
            "approval_item: id={} tool={} risk={:?} status={:?}",
            terminal_inline(&record.request.id),
            terminal_inline(&record.request.call.name),
            record.request.call.risk,
            record.status
        );
        println!("  reason: {}", terminal_inline(&record.request.reason));
        if let Some(workspace) = record.request.workspace_root.as_ref() {
            println!("  workspace: {}", path_display(workspace));
        } else {
            println!("  workspace: {}", path_display(&runtime.workspace));
        }
        println!(
            "  provider_call: {}",
            approval_bool(context, &["operations", "provider_call"])
        );
        println!(
            "  workspace_write: {}",
            approval_bool(context, &["operations", "workspace_write"])
        );
        let shell_requested = approval_bool(context, &["operations", "shell"]);
        println!("  shell: {shell_requested}");
        print_approval_shell_commands(context, shell_requested);
        println!("  network: {}", runtime.agent.profile.network);
        println!(
            "  provider: {}",
            approval_str(context, &["provider", "name"])
                .map(terminal_inline)
                .unwrap_or_else(|| "not_configured".into())
        );
        println!(
            "  session: {} turn={}",
            approval_str(context, &["session", "session_id"])
                .map(terminal_inline)
                .unwrap_or_else(|| "<generated>".into()),
            approval_str(context, &["session", "turn_id"])
                .map(terminal_inline)
                .unwrap_or_else(|| "<generated>".into())
        );
        println!(
            "  diff_size: {}",
            approval_u64(context, &["patch", "candidate_diff_chars"]).unwrap_or(0)
        );
        println!(
            "  replay: ikaros approval approve {}",
            terminal_inline(&record.request.id)
        );
    }
}

fn print_approval_shell_commands(context: Option<&serde_json::Value>, shell_requested: bool) {
    let commands = context
        .and_then(|context| context.pointer("/operations/shell_commands"))
        .and_then(serde_json::Value::as_array);
    match commands {
        Some(commands) if !commands.is_empty() => {
            println!("  shell_commands:");
            for command in commands {
                let command_text = command["command"].as_str().unwrap_or("<unknown>");
                let reason = command["reason"].as_str().unwrap_or("unspecified");
                println!(
                    "    - {} ({})",
                    terminal_inline(command_text),
                    terminal_inline(reason)
                );
            }
        }
        _ => {
            let inferred = approval_bool(context, &["operations", "shell_commands_inferred"]);
            if shell_requested && inferred {
                println!("  shell_commands: inferred from workspace");
            } else {
                println!("  shell_commands: none");
            }
        }
    }
}

fn approval_bool(context: Option<&serde_json::Value>, path: &[&str]) -> bool {
    approval_value(context, path)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn approval_str<'a>(context: Option<&'a serde_json::Value>, path: &[&str]) -> Option<&'a str> {
    approval_value(context, path).and_then(serde_json::Value::as_str)
}

fn approval_u64(context: Option<&serde_json::Value>, path: &[&str]) -> Option<u64> {
    approval_value(context, path).and_then(serde_json::Value::as_u64)
}

fn approval_value<'a>(
    context: Option<&'a serde_json::Value>,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut current = context?;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

pub(in crate::chat) fn print_context_status(
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<()> {
    println!(
        "context_session: {}",
        terminal_inline(&runtime.chat_session_id)
    );
    println!("context_token_budget: {}", options.context_token_budget);
    println!("context_history_limit: {}", options.history_context_limit);
    println!(
        "context_history_summary_limit: {}",
        options.history_summary_limit
    );
    println!("context_memory_limit: {}", options.memory_limit);
    println!("context_rag_top_k: {}", options.rag_top_k);
    println!(
        "context_relationship_learning: {}",
        options.relationship_learning
    );
    println!("context_disabled: {}", options.no_context);
    print_filtered_event_cells(runtime, "context", |kind| {
        matches!(
            kind,
            AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted
        )
    })?;
    Ok(())
}

pub(in crate::chat) fn print_memory_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    println!(
        "memory_backend: {}",
        terminal_inline(&config.memory.backend)
    );
    println!("memory_dir: {}", path_display(&paths.memory_dir));
    println!(
        "memory_context_enabled: {}",
        runtime.agent.profile.memory_context
    );
    println!(
        "memory_policy: promote={} demote={} forget={} max_records_per_scope={}",
        config.memory.policy.promote_threshold,
        config.memory.policy.demote_threshold,
        config.memory.policy.forget_threshold,
        config.memory.policy.max_records_per_scope
    );
    println!(
        "memory_external_providers: {}",
        config.memory.external_providers.len()
    );
    println!(
        "- {}",
        WorkbenchCell {
            kind: WorkbenchCellKind::Memory,
            title: "memory status".into(),
            detail: format!(
                "backend={} context_enabled={} external_providers={}",
                terminal_inline(&config.memory.backend),
                runtime.agent.profile.memory_context,
                config.memory.external_providers.len()
            ),
        }
        .render()
    );
    print_filtered_event_cells(runtime, "memory", |kind| {
        matches!(kind, AgentEventKind::MemoryLifecycle)
    })?;
    print_memory_journal_cells(paths)?;
    Ok(())
}

fn print_memory_journal_cells(paths: &IkarosPaths) -> Result<()> {
    let journal = JsonlMemoryJournal::new(&paths.memory_dir);
    let entries = journal.list()?;
    println!("memory_journal_entries: {}", entries.len());
    let start = entries.len().saturating_sub(5);
    for entry in &entries[start..] {
        let title = format!("journal {:?}", entry.action);
        let detail = format!(
            "scope={} reason={} source={}",
            entry.scope.as_deref().unwrap_or("none"),
            terminal_inline(&entry.reason),
            entry
                .source_ref
                .as_ref()
                .map(|source| format!("{source:?}"))
                .unwrap_or_else(|| "none".into())
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title,
                detail,
            }
            .render()
        );
    }
    Ok(())
}

fn print_filtered_event_cells(
    runtime: &InteractiveChatRuntime,
    label: &str,
    filter: impl Fn(&AgentEventKind) -> bool,
) -> Result<()> {
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let Some(replay) = store.replay_session(&session_id)? else {
        println!("{label}_timeline_events: 0");
        return Ok(());
    };
    let events = replay
        .agent_events
        .iter()
        .filter(|event| filter(&event.kind))
        .collect::<Vec<_>>();
    println!("{label}_timeline_events: {}", events.len());
    let start = events.len().saturating_sub(5);
    for event in &events[start..] {
        println!("- {}", agent_event_cell(event).render());
    }
    Ok(())
}

pub(in crate::chat) fn print_rag_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    options: &ChatRunOptions,
) {
    println!("rag_backend: {}", terminal_inline(&config.rag.backend));
    println!(
        "rag_embedding_provider: {}",
        terminal_inline(&config.rag.embedding_provider)
    );
    println!(
        "rag_embedding_model: {}",
        terminal_inline(&config.rag.embedding_model)
    );
    println!("rag_top_k: {}", options.rag_top_k);
    println!("rag_dir: {}", path_display(&paths.rag_dir));
}

pub(in crate::chat) async fn print_diff_status(
    runtime: &InteractiveChatRuntime,
    workspace: &Path,
) -> Result<()> {
    let output = runtime
        .session
        .env
        .run_process(
            ProcessRequest::program(
                "git",
                vec!["diff".into(), "--stat".into(), "--".into()],
                workspace,
            )
            .with_timeout_ms(2_000)
            .with_max_output_bytes(8 * 1024),
        )
        .await?;
    println!("diff_status: {}", output.status);
    let stdout = output.stdout.trim();
    let stderr = output.stderr.trim();
    if stdout.is_empty() {
        println!("diff_stat: clean_or_unavailable");
    } else {
        println!("diff_stat:");
        for line in stdout.lines() {
            println!("{}", terminal_inline(line));
        }
    }
    if !stderr.is_empty() {
        println!("diff_error: {}", terminal_inline(stderr));
    }
    Ok(())
}
