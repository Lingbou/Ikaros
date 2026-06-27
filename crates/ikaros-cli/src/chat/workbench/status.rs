// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::{
    InteractiveChatRuntime, InteractiveChatStatusInput, format_interactive_chat_status,
};
use crate::resolve_agent_instance;
use anyhow::Result;
use ikaros_automation::LocalScheduleStore;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_gateway::{GatewayMessageStatus, LocalGatewayStore};
#[cfg(test)]
use ikaros_harness::{ApprovalRecord, ApprovalStatus};
use ikaros_models::{
    ModelProviderDescriptor, ModelUsageLedger, ProviderHealthLedger, ProviderRegistry,
};
use ikaros_runtime::{ChatRunOptions, base_body_status};
#[cfg(test)]
use ikaros_session::SessionReplay;
use ikaros_session::{AgentEventKind, SessionId, SessionStore, SqliteSessionStore};
#[cfg(test)]
use std::collections::VecDeque;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[cfg(test)]
use super::{WorkbenchCell, WorkbenchScreen};
use super::{agent_event_cell, path_display, terminal_inline};

mod api;
mod approval;
mod context;
mod diff;
mod gateway;
mod memory;
mod provider;
mod queue;
mod screen;
mod session;
mod timeline;
mod tools;

pub(in crate::chat) use api::{
    api_status_human_lines, print_api_status, print_api_status_for_human,
};
#[cfg(test)]
use approval::approval_overlay_json_line;
pub(in crate::chat) use approval::print_approval_status;
#[cfg(test)]
use approval::screen_approval_cells;
#[cfg(test)]
use context::screen_context_cells_from_replay;
pub(in crate::chat) use context::{
    context_status_human_lines, print_context_status, print_context_status_for_human,
};
pub(in crate::chat) use diff::{print_diff_status, print_diff_status_for_human};
#[cfg(test)]
use gateway::screen_gateway_status_cell;
pub(in crate::chat) use gateway::{
    gateway_status_human_lines, print_gateway_status, print_gateway_status_for_human,
};
pub(in crate::chat) use memory::{
    memory_status_human_lines, print_memory_status, print_memory_status_for_human,
};
#[cfg(test)]
use provider::screen_provider_cells;
use provider::{
    apply_configured_model_cost, format_model_cost_status, format_model_fallback_status,
    model_budget_json, model_context_window, model_default_output_tokens, model_policy,
    model_profile, model_profile_source, model_retry_without_parameters, model_tokenizer,
};
pub(in crate::chat) use provider::{
    format_model_budget_status, model_status_human_lines, print_model_status,
    print_model_status_for_human, print_provider_status_for_human, provider_status_human_lines,
};
use queue::continuation_count;
#[cfg(test)]
use queue::{screen_queue_status_cell, screen_side_cells};
pub(in crate::chat) use screen::{print_screen_status_with_state, selected_screen_primary_action};
#[cfg(test)]
use screen::{screen_progress_status_cell, workbench_screen_dimensions_from_values};
pub(in crate::chat) use session::{
    print_session_export, print_session_history, print_session_status, print_session_summaries,
    session_history_human_lines, session_status_human_lines, session_summaries_human_lines,
};
pub(in crate::chat) use timeline::{
    TimelineRequest, TimelineVerbosity, print_replay_status, print_replay_status_for_human,
    print_trace_status, print_trace_status_for_human,
};
#[cfg(test)]
use timeline::{
    screen_coding_cells_from_replay, screen_failure_cells_from_replay,
    screen_timeline_cells_from_replay, timeline_point_matches,
};
pub(in crate::chat) use tools::{
    mcp_status_human_lines, print_mcp_status, print_mcp_status_for_human, print_rag_status,
    print_rag_status_for_human, print_tools_status, print_tools_status_for_human,
    rag_status_human_lines, tools_status_human_lines,
};

pub(in crate::chat) fn print_workbench_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    print_session_status(config, paths, workspace, runtime, options)?;
    print_unified_status(config, paths, workspace, runtime, usage_ledger)?;
    let body_status = base_body_status(paths)?;
    println!(
        "{}",
        format_interactive_chat_status(InteractiveChatStatusInput {
            agent: &runtime.agent,
            session: &runtime.session,
            chat_session_id: &runtime.chat_session_id,
            state_dir: &runtime.state_dir,
            options,
            emotion: &body_status.emotion,
            usage_ledger,
        })
    );
    Ok(())
}

pub(in crate::chat) fn print_workbench_status_for_human(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    for line in
        workbench_status_human_lines(config, paths, workspace, runtime, options, usage_ledger)?
    {
        println!("{line}");
    }
    Ok(())
}

pub(in crate::chat) fn workbench_status_human_lines(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<Vec<String>> {
    let mut lines = session_status_human_lines(config, paths, workspace, runtime, options)?;
    lines.extend(unified_status_human_lines(
        config,
        paths,
        workspace,
        runtime,
        usage_ledger,
    )?);
    Ok(lines)
}

fn print_unified_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    let model = &runtime.model_config;
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
    let mut descriptor = ProviderRegistry
        .descriptor_with_profile(
            &model.provider,
            &runtime.model_provider.base_url,
            &model.model,
            &model.compat_profile,
        )
        .ok();
    if let Some(descriptor) = &mut descriptor {
        apply_configured_model_cost(descriptor, &model.cost);
    }
    println!(
        "status_model: provider={} model={} profile={} profile_source={} context_window={} default_output_tokens={} tokenizer={} runtime={} transport={} health={} fallback_count={}",
        terminal_inline(&model.provider),
        terminal_inline(&model.model),
        model_profile(&descriptor),
        model_profile_source(&model.provider, &model.compat_profile, descriptor.as_ref()),
        model_context_window(&descriptor),
        model_default_output_tokens(&descriptor),
        model_tokenizer(&descriptor),
        terminal_inline(&model.runtime),
        terminal_inline(&model.transport),
        terminal_inline(&provider_health),
        model.fallbacks.len()
    );
    println!(
        "status_model_policy: temperature={} reasoning={} message={} tool_schema={} request_body={} prompt_cache={} retry_without_parameters={}",
        model_policy(&descriptor, |policy| &policy.temperature),
        model_policy(&descriptor, |policy| &policy.reasoning),
        model_policy(&descriptor, |policy| &policy.message),
        model_policy(&descriptor, |policy| &policy.tool_schema),
        model_policy(&descriptor, |policy| &policy.request_body),
        model_policy(&descriptor, |policy| &policy.prompt_cache),
        model_retry_without_parameters(&descriptor),
    );
    println!(
        "status_model_budget: {}",
        format_model_budget_status(model, usage_ledger)?
    );
    println!(
        "status_model_cost: {}",
        format_model_cost_status(descriptor.as_ref(), usage_ledger)?
    );
    println!(
        "status_model_fallbacks: {}",
        format_model_fallback_status(model)
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
    println!(
        "{}",
        workbench_status_json_line(WorkbenchStatusJsonInput {
            model,
            runtime,
            workspace,
            provider_health: &provider_health,
            descriptor: descriptor.as_ref(),
            usage_ledger,
            gateway_pending,
            approvals_pending,
            continuations,
        })?
    );
    Ok(())
}

fn unified_status_human_lines(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    usage_ledger: &ModelUsageLedger,
) -> Result<Vec<String>> {
    let model = &runtime.model_config;
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
    let descriptor = ProviderRegistry
        .descriptor_with_profile(
            &model.provider,
            &runtime.model_provider.base_url,
            &model.model,
            &model.compat_profile,
        )
        .ok();

    Ok(vec![
        "• Status".to_owned(),
        format!("  workspace: {}", path_display(workspace)),
        format!(
            "  model: {} ({})",
            terminal_inline(&model.model),
            terminal_inline(&model.provider)
        ),
        format!(
            "  profile: {} ({})",
            model_profile(&descriptor),
            model_profile_source(&model.provider, &model.compat_profile, descriptor.as_ref())
        ),
        format!("  health: {}", terminal_inline(&provider_health)),
        format!(
            "  budget: {}",
            human_model_budget_status(model, usage_ledger)?
        ),
        "  permissions:".to_owned(),
        format!(
            "    workspace writes: {}",
            runtime.agent.profile.workspace_writes
        ),
        format!("    shell: {}", runtime.agent.profile.shell),
        format!("    network: {}", runtime.agent.profile.network),
        "  pending:".to_owned(),
        format!("    gateway: {gateway_pending}"),
        format!("    approvals: {approvals_pending}"),
        format!("    queued continuations: {continuations}"),
    ])
}

fn human_model_budget_status(
    model: &ikaros_core::ModelConfig,
    usage_ledger: &ModelUsageLedger,
) -> Result<String> {
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let used = usage_ledger.total_for_day(&today)?;
    Ok(match model.daily_token_budget {
        Some(budget) => {
            let remaining = budget.saturating_sub(used);
            format!("{used} used today, {remaining} remaining of {budget}")
        }
        None => format!("{used} used today, no daily limit"),
    })
}

struct WorkbenchStatusJsonInput<'a> {
    model: &'a ikaros_core::ModelConfig,
    runtime: &'a InteractiveChatRuntime,
    workspace: &'a Path,
    provider_health: &'a str,
    descriptor: Option<&'a ModelProviderDescriptor>,
    usage_ledger: &'a ModelUsageLedger,
    gateway_pending: usize,
    approvals_pending: usize,
    continuations: usize,
}

fn workbench_status_json_line(input: WorkbenchStatusJsonInput<'_>) -> Result<String> {
    let descriptor_owned = input.descriptor.cloned();
    let payload = serde_json::json!({
        "schema": "ikaros-workbench-status-v1",
        "version": 1,
        "session": {
            "session_id": terminal_inline(&input.runtime.chat_session_id),
            "state_db": terminal_inline(&input.runtime.state_dir.join("state.db").display().to_string()),
        },
        "agent": {
            "id": terminal_inline(&input.runtime.agent_id),
            "profile": terminal_inline(&input.runtime.agent.name),
            "mode": input.runtime.agent.mode().to_string(),
            "policy": {
                "workspace_writes": input.runtime.agent.profile.workspace_writes.to_string(),
                "shell": input.runtime.agent.profile.shell.to_string(),
                "network": input.runtime.agent.profile.network.to_string(),
            },
        },
        "workspace": terminal_inline(&input.workspace.display().to_string()),
        "model": {
            "provider": terminal_inline(&input.model.provider),
            "model": terminal_inline(&input.model.model),
            "profile": model_profile(&descriptor_owned),
            "profile_source": model_profile_source(&input.model.provider, &input.model.compat_profile, input.descriptor),
            "context_window": model_context_window(&descriptor_owned),
            "default_output_tokens": model_default_output_tokens(&descriptor_owned),
            "tokenizer": model_tokenizer(&descriptor_owned),
            "runtime": terminal_inline(&input.model.runtime),
            "transport": terminal_inline(&input.model.transport),
            "health": terminal_inline(input.provider_health),
            "fallback_count": input.model.fallbacks.len(),
            "budget": model_budget_json(input.model, input.usage_ledger)?,
        },
        "counts": {
            "gateway_pending": input.gateway_pending,
            "approvals_pending": input.approvals_pending,
            "continuations": input.continuations,
        },
        "actions": {
            "screen": "/screen",
            "timeline": "/timeline",
            "trace": "/trace",
            "provider_debug": "/provider debug",
            "provider_matrix": "/provider matrix",
            "approval": "/approval",
            "cancel_all": "/cancel all",
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-status-v1","version":1,"error":"serialization_failed"}"#
            .to_owned()
    });
    Ok(format!("workbench_status_json: {encoded}"))
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut truncated = String::new();
    for (index, ch) in input.chars().enumerate() {
        if index >= max_chars {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
    }
    truncated
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

fn task_counts(paths: &IkarosPaths) -> Result<(PathBuf, usize, usize, usize, usize)> {
    let store = LocalScheduleStore::new(&paths.automation_dir);
    let jobs = store.list()?;
    let due = store.due_now()?;
    let enabled = jobs.iter().filter(|job| job.enabled).count();
    let disabled = jobs.len().saturating_sub(enabled);
    Ok((
        store.path().to_path_buf(),
        jobs.len(),
        enabled,
        disabled,
        due.len(),
    ))
}

pub(in crate::chat) fn print_tasks_status(paths: &IkarosPaths) -> Result<()> {
    let (store_path, total, enabled, disabled, due) = task_counts(paths)?;
    println!("tasks_store: {}", path_display(&store_path));
    println!("tasks_total: {total}");
    println!("tasks_enabled: {enabled}");
    println!("tasks_disabled: {disabled}");
    println!("tasks_due: {due}");
    Ok(())
}

#[allow(dead_code)]
pub(in crate::chat) fn print_tasks_status_for_human(paths: &IkarosPaths) -> Result<()> {
    for line in tasks_status_human_lines(paths)? {
        println!("{line}");
    }
    Ok(())
}

pub(in crate::chat) fn tasks_status_human_lines(paths: &IkarosPaths) -> Result<Vec<String>> {
    let (store_path, total, enabled, disabled, due) = task_counts(paths)?;
    Ok(vec![
        "• Tasks".to_owned(),
        format!("  store: {}", path_display(&store_path)),
        format!("  total: {total}"),
        format!("  enabled: {enabled}"),
        format!("  disabled: {disabled}"),
        format!("  due now: {due}"),
    ])
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

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_core::ModelConfig;
    use ikaros_gateway::GatewayMessageKind;
    use ikaros_models::{
        ModelContextProfile, ModelTokenizerKind, ModelUsageLedger, ModelUsageRecord,
    };
    use tempfile::tempdir;

    #[test]
    fn model_budget_status_reports_daily_usage_and_remaining_tokens() {
        let temp = tempdir().expect("tempdir");
        let ledger = ModelUsageLedger::from_file(temp.path().join("model-usage.jsonl"));
        let today = time::OffsetDateTime::now_utc().date().to_string();
        ledger
            .append(ModelUsageRecord {
                id: "usage-one".into(),
                at: format!("{today}T01:00:00Z"),
                provider: "openai-compatible".into(),
                model: "kimi-k2.6".into(),
                prompt_tokens: Some(40),
                completion_tokens: Some(10),
                total_tokens: 50,
                cache_read_tokens: None,
                cache_write_tokens: None,
                estimated: false,
            })
            .expect("append usage");
        let model = ModelConfig {
            daily_token_budget: Some(100),
            ..ModelConfig::default()
        };

        let rendered = format_model_budget_status(&model, &ledger).expect("budget status");

        assert_eq!(
            rendered,
            "daily_token_budget=100 used_today=50 remaining_today=50 budget_status=ok"
        );
    }

    #[test]
    fn model_budget_status_reports_disabled_daily_budget() {
        let temp = tempdir().expect("tempdir");
        let ledger = ModelUsageLedger::from_file(temp.path().join("model-usage.jsonl"));
        let model = ModelConfig {
            daily_token_budget: None,
            ..ModelConfig::default()
        };

        let rendered = format_model_budget_status(&model, &ledger).expect("budget status");

        assert_eq!(
            rendered,
            "daily_token_budget=disabled used_today=0 remaining_today=unbounded budget_status=unbounded"
        );
    }

    #[test]
    fn model_cost_status_estimates_today_cost_when_pricing_is_known() {
        let temp = tempdir().expect("tempdir");
        let ledger = ModelUsageLedger::from_file(temp.path().join("model-usage.jsonl"));
        let today = time::OffsetDateTime::now_utc().date().to_string();
        ledger
            .append(ModelUsageRecord {
                id: "usage-cost".into(),
                at: format!("{today}T02:00:00Z"),
                provider: "openai-compatible".into(),
                model: "priced-model".into(),
                prompt_tokens: Some(100),
                completion_tokens: Some(50),
                total_tokens: 150,
                cache_read_tokens: Some(25),
                cache_write_tokens: Some(10),
                estimated: false,
            })
            .expect("append usage");
        let descriptor = ModelProviderDescriptor {
            provider: "openai-compatible".into(),
            model: "priced-model".into(),
            profile: "generic".into(),
            profile_policy: ikaros_models::ModelProviderProfilePolicy::native("generic"),
            capabilities: ikaros_models::ModelProviderCapabilities {
                chat: true,
                streaming: true,
                tool_calls: true,
                reasoning: false,
                json_mode: true,
                network: true,
                image_input: true,
                audio_input: true,
                file_input: true,
            },
            context: ikaros_models::ModelContextProfile::default(),
            cost: ikaros_models::ModelProviderCost {
                currency: "USD".into(),
                input_per_million: Some(2.0),
                output_per_million: Some(10.0),
                cache_read_per_million: Some(0.2),
                cache_write_per_million: Some(2.5),
            },
            health: ikaros_models::ProviderHealthState::new("openai-compatible", "priced-model"),
        };

        let rendered = format_model_cost_status(Some(&descriptor), &ledger).expect("cost status");

        assert_eq!(
            rendered,
            "currency=USD input_per_million=2.0000 output_per_million=10.0000 cache_read_per_million=0.2000 cache_write_per_million=2.5000 estimated_cost_today=0.000660 cache_read_tokens_today=25 cache_write_tokens_today=10 cache_accounting=priced"
        );
    }

    #[test]
    fn model_cost_status_uses_configured_pricing_overlay() {
        let temp = tempdir().expect("tempdir");
        let ledger = ModelUsageLedger::from_file(temp.path().join("model-usage.jsonl"));
        let today = time::OffsetDateTime::now_utc().date().to_string();
        ledger
            .append(ModelUsageRecord {
                id: "usage-configured-cost".into(),
                at: format!("{today}T02:00:00Z"),
                provider: "openai-compatible".into(),
                model: "configured-cost-model".into(),
                prompt_tokens: Some(20),
                completion_tokens: Some(10),
                total_tokens: 30,
                cache_read_tokens: Some(5),
                cache_write_tokens: Some(5),
                estimated: false,
            })
            .expect("append usage");
        let mut descriptor = ModelProviderDescriptor {
            provider: "openai-compatible".into(),
            model: "configured-cost-model".into(),
            profile: "generic".into(),
            profile_policy: ikaros_models::ModelProviderProfilePolicy::native("generic"),
            capabilities: ikaros_models::ModelProviderCapabilities {
                chat: true,
                streaming: true,
                tool_calls: true,
                reasoning: false,
                json_mode: true,
                network: true,
                image_input: true,
                audio_input: true,
                file_input: true,
            },
            context: ikaros_models::ModelContextProfile::default(),
            cost: ikaros_models::ModelProviderCost {
                currency: "USD".into(),
                input_per_million: None,
                output_per_million: None,
                cache_read_per_million: None,
                cache_write_per_million: None,
            },
            health: ikaros_models::ProviderHealthState::new(
                "openai-compatible",
                "configured-cost-model",
            ),
        };

        apply_configured_model_cost(
            &mut descriptor,
            &ikaros_core::ModelCostConfig {
                currency: "CNY".into(),
                input_per_million: Some(4.0),
                output_per_million: Some(16.0),
                cache_read_per_million: Some(0.4),
                cache_write_per_million: Some(4.0),
            },
        );
        let rendered = format_model_cost_status(Some(&descriptor), &ledger).expect("cost status");

        assert_eq!(
            rendered,
            "currency=CNY input_per_million=4.0000 output_per_million=16.0000 cache_read_per_million=0.4000 cache_write_per_million=4.0000 estimated_cost_today=0.000222 cache_read_tokens_today=5 cache_write_tokens_today=5 cache_accounting=priced"
        );
    }

    #[test]
    fn model_fallback_status_redacts_endpoint_and_secret_values() {
        let model = ModelConfig {
            fallbacks: vec![ikaros_core::ModelFallbackConfig {
                provider: "openai-compatible".into(),
                model: "fallback-one".into(),
                compat_profile: "auto".into(),
                base_url: "https://api.example.test/v1".into(),
                api_key: "sk-secret-value".into(),
                ..ikaros_core::ModelFallbackConfig::default()
            }],
            ..ModelConfig::default()
        };

        let rendered = format_model_fallback_status(&model);

        assert_eq!(
            rendered,
            "fallback_count=1 fallback_chain=0:openai-compatible/fallback-one profile=auto"
        );
        assert!(!rendered.contains("api.example.test"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn screen_provider_cells_summarize_model_cost_fallback_and_debug_without_secrets() {
        let temp = tempdir().expect("tempdir");
        let ledger = ModelUsageLedger::from_file(temp.path().join("model-usage.jsonl"));
        let today = time::OffsetDateTime::now_utc().date().to_string();
        ledger
            .append(ModelUsageRecord {
                id: "provider-screen-usage".into(),
                at: format!("{today}T03:00:00Z"),
                provider: "openai-compatible".into(),
                model: "screen-model".into(),
                prompt_tokens: Some(40),
                completion_tokens: Some(20),
                total_tokens: 60,
                cache_read_tokens: Some(5),
                cache_write_tokens: Some(3),
                estimated: false,
            })
            .expect("append usage");
        let model = ModelConfig {
            provider: "openai-compatible".into(),
            model: "screen-model".into(),
            compat_profile: "local-openai-compatible".into(),
            fallbacks: vec![ikaros_core::ModelFallbackConfig {
                provider: "openai-compatible".into(),
                model: "fallback-screen".into(),
                compat_profile: "auto".into(),
                base_url: "https://api.example.test/v1".into(),
                api_key: "sk-secret-value".into(),
                ..ikaros_core::ModelFallbackConfig::default()
            }],
            ..ModelConfig::default()
        };
        let descriptor = ModelProviderDescriptor {
            provider: "openai-compatible".into(),
            model: "screen-model".into(),
            profile: "local-openai-compatible".into(),
            profile_policy: ikaros_models::ModelProviderProfilePolicy::native(
                "local-openai-compatible",
            ),
            capabilities: ikaros_models::ModelProviderCapabilities {
                chat: true,
                streaming: true,
                tool_calls: true,
                reasoning: false,
                json_mode: true,
                network: false,
                image_input: true,
                audio_input: true,
                file_input: true,
            },
            context: ModelContextProfile::new(
                131_072,
                65_536,
                ModelTokenizerKind::OpenAiCompatible,
                "test-screen-provider",
            ),
            cost: ikaros_models::ModelProviderCost {
                currency: "USD".into(),
                input_per_million: Some(1.0),
                output_per_million: Some(2.0),
                cache_read_per_million: None,
                cache_write_per_million: None,
            },
            health: ikaros_models::ProviderHealthState::new("openai-compatible", "screen-model"),
        };

        let health = ProviderHealthLedger::from_file(temp.path().join("provider-health.jsonl"));
        let cells = screen_provider_cells(&model, Some(&descriptor), &ledger, &health)
            .expect("provider screen cells");
        let rendered = cells
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(cells.len() >= 5);
        assert!(rendered.contains("provider matrix"));
        assert!(rendered.contains("model budget"));
        assert!(rendered.contains("provider recovery"));
        assert!(rendered.contains("profile=local-openai-compatible"));
        assert!(rendered.contains("context_window=131072"));
        assert!(rendered.contains("estimated_cost_today="));
        assert!(rendered.contains("cache_read_tokens_today="));
        assert!(rendered.contains("cache_accounting=tracked"));
        assert!(rendered.contains("provider health"));
        assert!(rendered.contains("health_status=Unknown"));
        assert!(rendered.contains("fallback_count=1"));
        assert!(rendered.contains("/provider matrix --live"));
        assert!(rendered.contains("title=provider cost"));
        assert!(
            rendered.contains("currency=USD input_per_million=1.0000 output_per_million=2.0000")
        );
        assert!(rendered.contains("command=/provider debug matrix=/provider matrix"));
        assert!(rendered.contains(
            "matrix=/provider matrix live=/provider matrix --live health=/provider health debug=/provider debug inspect=/provider inspect"
        ));
        assert!(rendered.contains("/provider debug"));
        assert!(!rendered.contains("api.example.test"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn screen_provider_cells_show_health_cooldown_and_error_without_secrets() {
        let temp = tempdir().expect("tempdir");
        let usage = ModelUsageLedger::from_file(temp.path().join("model-usage.jsonl"));
        let health = ProviderHealthLedger::from_file(temp.path().join("provider-health.jsonl"));
        health
            .append(ikaros_models::ProviderHealthRecord {
                at: "2026-06-23T01:00:00Z".into(),
                provider: "openai-compatible".into(),
                model: "screen-model".into(),
                status: ikaros_models::ProviderHealthStatus::Unavailable,
                consecutive_failures: 3,
                last_error_kind: Some(ikaros_models::ProviderErrorKind::RateLimited),
                last_error_summary: "429 rate limited api_key=sk-secret-value".into(),
                cooldown_until: Some("2026-06-23T01:01:00Z".into()),
            })
            .expect("append health");
        let model = ModelConfig {
            provider: "openai-compatible".into(),
            model: "screen-model".into(),
            ..ModelConfig::default()
        };

        let cells =
            screen_provider_cells(&model, None, &usage, &health).expect("provider screen cells");
        let rendered = cells
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("provider health"));
        assert!(rendered.contains("health_status=Unavailable"));
        assert!(rendered.contains("consecutive_failures=3"));
        assert!(rendered.contains("last_error_kind=RateLimited"));
        assert!(rendered.contains("cooldown_until=2026-06-23T01:01:00Z"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn screen_gateway_status_cell_reports_worker_lock_without_secret_leakage() {
        let temp = tempdir().expect("tempdir");
        let gateway_dir = temp.path().join("gateway");
        fs::create_dir_all(&gateway_dir).expect("gateway dir");
        fs::write(
            gateway_dir.join("message-worker.lock"),
            "pid=123\nowner=worker-token=abc123\n",
        )
        .expect("worker lock");
        let store = LocalGatewayStore::new(&gateway_dir);
        let message = store
            .enqueue(ikaros_gateway::GatewayRoute::new(
                "cli",
                GatewayMessageKind::Task,
                "cancel from screen",
                None,
            ))
            .expect("enqueue");
        store
            .cancel(&message.id, "screen cancel token=abc123")
            .expect("cancel");
        let delivery = store
            .deliver(
                "message-one",
                "chat_response",
                "screen delivery token=abc123",
            )
            .expect("delivery");
        let claim = store
            .claim_pending_deliveries_with_owner(1, "screen-adapter")
            .expect("claim")
            .pop()
            .expect("delivery claim");
        assert_eq!(claim.id, delivery.id);
        store
            .record_delivery_failure_for_claim(&claim, "delivery token=abc123", 2, 30)
            .expect("delivery retry");

        let cell = screen_gateway_status_cell(&store).expect("gateway status cell");
        let rendered = cell.render();

        assert!(rendered.contains("kind=continuation"));
        assert!(rendered.contains("gateway"));
        assert!(rendered.contains("cancelled=1"));
        assert!(rendered.contains("delivery_pending=1"));
        assert!(rendered.contains("delivery_dead_lettered=0"));
        assert!(rendered.contains("lock=present"));
        assert!(rendered.contains("owner=pid=123 owner=[REDACTED_SECRET]"));
        assert!(!rendered.contains("abc123"));
    }

    #[test]
    fn screen_context_cells_summarize_context_diff_compaction_and_references() {
        let session_id = SessionId::from("screen-context-session");
        let turn_id = ikaros_session::TurnId::from("screen-context-turn");
        let context_diff = ikaros_session::AgentEvent {
            event_id: ikaros_session::EventId::from("context-diff"),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc() - time::Duration::seconds(2),
            source: ikaros_session::AgentEventSource::Context,
            kind: AgentEventKind::ContextDiff,
            payload: serde_json::json!({
                "budget": {
                    "max_tokens": 4096,
                    "used_tokens": 1536,
                    "estimator": "heuristic-v1",
                    "context_window": 8192,
                    "reserved_output_tokens": 1024,
                    "source": "provider-window"
                },
                "sections": [
                    {
                        "kind": "references",
                        "label": "references",
                        "estimated_tokens": 240,
                        "source_kind": "explicit_reference",
                        "trust_level": "high",
                        "freshness": "current",
                        "scope": "workspace",
                        "injection_reason": "explicit @file",
                        "lines": ["src/lib.rs contains token=sk-secret-value"]
                    },
                    {
                        "kind": "history",
                        "label": "history",
                        "estimated_tokens": 90,
                        "source_kind": "session_history",
                        "trust_level": "medium",
                        "freshness": "recent",
                        "scope": "session",
                        "injection_reason": "recent turns"
                    }
                ],
                "references": [
                    {
                        "kind": { "type": "file", "path": "src/lib.rs", "line_range": [1, 12] },
                        "raw": "@file:src/lib.rs:1-12",
                        "resolved_path": "src/lib.rs",
                        "estimated_tokens": 240
                    }
                ],
                "prompt_stable_prefix_hash": "fnv1a64:feedface12345678",
                "prompt_stable_prefix_message_count": 1,
                "prompt_stable_prefix_estimated_tokens": 88,
                "compression_summary": "older context compressed token=sk-secret-value",
                "continuation_prompt": "continue from compacted context"
            }),
        };
        let compacted = ikaros_session::AgentEvent {
            event_id: ikaros_session::EventId::from("context-compacted"),
            session_id: session_id.clone(),
            turn_id,
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc() - time::Duration::seconds(1),
            source: ikaros_session::AgentEventSource::Context,
            kind: AgentEventKind::ContextCompacted,
            payload: serde_json::json!({
                "summary": "kept protected reference and removed secret sk-secret-value",
                "compressed_sections": [
                    { "kind": "history", "original_tokens": 300, "compressed_tokens": 75 }
                ],
                "continuation_prompt": "continue from compacted context"
            }),
        };
        let replay = SessionReplay {
            session: ikaros_session::SessionRecord::new(
                session_id,
                ikaros_session::SessionSource::Cli,
            ),
            entries: Vec::new(),
            agent_events: vec![context_diff, compacted],
            approvals: Vec::new(),
        };

        let cells = screen_context_cells_from_replay(&replay).expect("context cells");
        let rendered = cells
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("context budget"));
        assert!(rendered.contains("turn=screen-context-turn"));
        assert!(rendered.contains("trace=/trace screen-context-turn"));
        assert!(rendered.contains("estimator=heuristic-v1"), "{rendered}");
        assert!(rendered.contains("used=1536"));
        assert!(rendered.contains("context_window=8192"));
        assert!(rendered.contains("prompt_cache_hash=fnv1a64:feedface12345678"));
        assert!(rendered.contains("prompt_cache_messages=1"));
        assert!(rendered.contains("prompt_cache_estimated=88"));
        assert!(rendered.contains("section references estimated=240"));
        assert!(rendered.contains("trust=high"));
        assert!(rendered.contains("freshness=current"));
        assert!(rendered.contains("scope=workspace"));
        assert!(rendered.contains("section history estimated=90"));
        assert!(rendered.contains("trust=medium"));
        assert!(rendered.contains("freshness=recent"));
        assert!(rendered.contains("scope=session"));
        assert!(rendered.contains("reference @file:src/lib.rs:1-12"));
        assert!(rendered.contains("context compacted"));
        assert!(rendered.contains("compressed_sections=1"));
        assert!(rendered.contains("continuation_prompt=yes"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn screen_failure_cells_surface_latest_failure_recovery_actions_without_secrets() {
        let session_id = SessionId::from("screen-failure-session");
        let turn_id = ikaros_session::TurnId::from("screen-failure-turn");
        let event = ikaros_session::AgentEvent {
            event_id: ikaros_session::EventId::from("screen-failure-event"),
            session_id: session_id.clone(),
            turn_id,
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Runtime,
            kind: AgentEventKind::Error,
            payload: serde_json::json!({
                "phase": "provider_generate",
                "message": "model daily token budget exceeded: api_key=sk-secret-value",
            }),
        };
        let replay = SessionReplay {
            session: ikaros_session::SessionRecord::new(
                session_id,
                ikaros_session::SessionSource::Cli,
            ),
            entries: Vec::new(),
            agent_events: vec![event],
            approvals: Vec::new(),
        };

        let cells = screen_failure_cells_from_replay(&replay);
        let rendered = cells
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(cells.len(), 1);
        assert!(rendered.contains("latest error"));
        assert!(rendered.contains("kind=budget_exceeded"));
        assert!(rendered.contains("command=/status"));
        assert!(rendered.contains("budget=/budget"));
        assert!(rendered.contains("disable=/budget disable"));
        assert!(rendered.contains("trace=/trace --failed"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn timeline_point_filters_match_failure_and_approval_events() {
        assert!(timeline_point_matches(&AgentEventKind::Error, "failed"));
        assert!(timeline_point_matches(
            &AgentEventKind::ToolCallFailed,
            "failed"
        ));
        assert!(timeline_point_matches(
            &AgentEventKind::ContinuationFailed,
            "failed"
        ));
        assert!(timeline_point_matches(
            &AgentEventKind::ModelDiagnostic(ikaros_models::ModelRequestDiagnostic {
                kind: "provider_retry_failed".into(),
                message: "retry failed for provider".into(),
                parameter: None,
            }),
            "failed"
        ));
        assert!(timeline_point_matches(
            &AgentEventKind::ApprovalRequested,
            "approval"
        ));
        assert!(timeline_point_matches(
            &AgentEventKind::ApprovalResolved,
            "approval"
        ));

        assert!(!timeline_point_matches(&AgentEventKind::TurnEnd, "failed"));
        assert!(!timeline_point_matches(
            &AgentEventKind::ToolCallCompleted,
            "approval"
        ));
    }

    #[test]
    fn screen_timeline_cells_use_recent_session_replay_entries_and_events() {
        let session_id = SessionId::from("screen-session");
        let mut older = ikaros_session::SessionEntry::new(
            session_id.clone(),
            ikaros_session::SessionEntryKind::UserMessage,
        );
        older.visible_text = Some("old prompt".into());
        older.at = time::OffsetDateTime::now_utc() - time::Duration::minutes(2);
        let mut recent = ikaros_session::SessionEntry::new(
            session_id.clone(),
            ikaros_session::SessionEntryKind::AssistantMessage,
        );
        recent.visible_text = Some("recent answer".into());
        recent.at = time::OffsetDateTime::now_utc() - time::Duration::minutes(1);
        let event = ikaros_session::AgentEvent {
            event_id: ikaros_session::EventId::from("event-recent"),
            session_id: session_id.clone(),
            turn_id: ikaros_session::TurnId::from("turn-recent"),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Runtime,
            kind: AgentEventKind::TurnEnd,
            payload: serde_json::Value::Null,
        };
        let replay = SessionReplay {
            session: ikaros_session::SessionRecord::new(
                session_id,
                ikaros_session::SessionSource::Cli,
            ),
            entries: vec![older, recent],
            agent_events: vec![event],
            approvals: Vec::new(),
        };

        let cells = screen_timeline_cells_from_replay(&replay, 2);
        let rendered = cells
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(cells.len() >= 2);
        assert!(rendered.contains("recent answer"));
        assert!(rendered.contains("turn_end"));
        assert!(rendered.contains("timeline navigator"));
        assert!(!rendered.contains("old prompt"));
    }

    #[test]
    fn screen_coding_cells_summarize_latest_workflow_groups_with_actions() {
        let session_id = SessionId::from("coding-screen-session");
        let turn_id = ikaros_session::TurnId::from("coding-turn");
        let events = [
            ("plan_prepared", "plan ready", time::Duration::minutes(4)),
            (
                "diff_updated",
                "2 files changed token=sk-secret-value",
                time::Duration::minutes(3),
            ),
            (
                "test_evidence_recorded",
                "cargo test failed",
                time::Duration::minutes(2),
            ),
            (
                "review_completed",
                "review found missing assertion",
                time::Duration::minutes(1),
            ),
        ]
        .into_iter()
        .map(|(kind, summary, age)| ikaros_session::AgentEvent {
            event_id: ikaros_session::EventId::new(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc() - age,
            source: ikaros_session::AgentEventSource::Tool,
            kind: AgentEventKind::CodingTurn,
            payload: serde_json::json!({
                "kind": kind,
                "summary": summary,
            }),
        })
        .collect::<Vec<_>>();
        let replay = SessionReplay {
            session: ikaros_session::SessionRecord::new(
                session_id,
                ikaros_session::SessionSource::Cli,
            ),
            entries: Vec::new(),
            agent_events: events,
            approvals: Vec::new(),
        };

        let cells = screen_coding_cells_from_replay(&replay);
        let rendered = cells
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(cells.len() >= 4);
        assert!(rendered.contains("coding progress"));
        assert!(rendered.contains("coding diff"));
        assert!(rendered.contains("coding test"));
        assert!(rendered.contains("coding review"));
        assert!(rendered.contains("command=/diff"));
        assert!(rendered.contains("plan=/code plan"));
        assert!(rendered.contains("test=/code test"));
        assert!(rendered.contains("review=/code review"));
        assert!(rendered.contains("rollback=/code rollback"));
        assert!(rendered.contains("turn=coding-turn"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn screen_side_cells_show_continuation_queue_when_no_approvals_are_pending() {
        let queued = test_continuation(
            "queued-continuation",
            ikaros_session::SessionContinuationKind::NextTurn,
            ikaros_session::SessionContinuationStatus::Queued,
            None,
        );
        let running = test_continuation(
            "running-continuation",
            ikaros_session::SessionContinuationKind::ToolResult,
            ikaros_session::SessionContinuationStatus::Running,
            Some("worker-one"),
        );

        let cells = screen_side_cells(&[], &[queued, running], &VecDeque::new(), &[]);
        let rendered = cells
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("kind=continuation"));
        assert!(rendered.contains("next_turn"));
        assert!(rendered.contains("status=queued"));
        assert!(rendered.contains("tool_result"));
        assert!(rendered.contains("status=running"));
        assert!(rendered.contains("lease_owner=worker-one"));
        assert!(rendered.contains("cancel=/cancel queued-continuation"));
        assert!(rendered.contains("cancel=/cancel running-continuation"));
    }

    #[test]
    fn screen_queue_status_cell_summarizes_continuation_state() {
        let queued = test_continuation(
            "queued-continuation",
            ikaros_session::SessionContinuationKind::NextTurn,
            ikaros_session::SessionContinuationStatus::Queued,
            None,
        );
        let running = test_continuation(
            "running-continuation",
            ikaros_session::SessionContinuationKind::ToolResult,
            ikaros_session::SessionContinuationStatus::Running,
            Some("worker-one"),
        );
        let completed = test_continuation(
            "completed-continuation",
            ikaros_session::SessionContinuationKind::Retry,
            ikaros_session::SessionContinuationStatus::Completed,
            None,
        );

        let rendered = screen_queue_status_cell(&[queued, running, completed]).render();

        assert!(rendered.contains("queued=1"));
        assert!(rendered.contains("running=1"));
        assert!(rendered.contains("completed=1"));
        assert!(rendered.contains("active_kind=tool_result"));
        assert!(rendered.contains("active_id=running-continuation"));
        assert!(rendered.contains("lease_owner=worker-one"));
        assert!(rendered.contains("command=/debug continuations"));
    }

    #[test]
    fn screen_queue_status_cell_redacts_failed_continuation_error() {
        let mut failed = test_continuation(
            "failed-continuation",
            ikaros_session::SessionContinuationKind::FollowUp,
            ikaros_session::SessionContinuationStatus::Failed,
            None,
        );
        failed.error = Some("provider key sk-secret-value failed".into());

        let rendered = screen_queue_status_cell(&[failed]).render();

        assert!(rendered.contains("failed=1"));
        assert!(rendered.contains("active_kind=follow_up"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn screen_progress_status_cell_renders_latest_progress_without_secret_leakage() {
        let progress = crate::chat::progress::WorkbenchProgressSnapshot {
            kind: "chat_turn".into(),
            status: "failed".into(),
            elapsed_ms: Some(42),
            detail: "provider key [REDACTED_SECRET] failed".into(),
            error_kind: Some("provider_error".into()),
        };

        let rendered = screen_progress_status_cell(Some(&progress)).render();

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
    fn cached_screen_progress_can_be_replaced_for_running_tick() {
        let mut screen = WorkbenchScreen {
            title: "Ikaros".into(),
            status: vec![screen_progress_status_cell(None)],
            timeline: Vec::new(),
            main: Vec::new(),
            side: Vec::new(),
            footer: String::new(),
            input_hint: String::new(),
        };
        let progress = crate::chat::progress::WorkbenchProgressSnapshot {
            kind: "chat_turn".into(),
            status: "running".into(),
            elapsed_ms: Some(1_500),
            detail: "hello".into(),
            error_kind: None,
        };

        screen::apply_progress_to_cached_screen(&mut screen, &progress);

        let rendered = screen
            .status
            .iter()
            .find(|cell| cell.title == "progress")
            .expect("progress cell")
            .render();
        assert!(rendered.contains("status=running"));
        assert!(rendered.contains("elapsed_ms=1500"));
        assert!(rendered.contains("detail=hello"));
    }

    #[test]
    fn cached_screen_live_stream_updates_assistant_conversation_cell() {
        let mut screen = WorkbenchScreen {
            title: "Ikaros".into(),
            status: Vec::new(),
            timeline: Vec::new(),
            main: vec![WorkbenchCell {
                kind: super::super::WorkbenchCellKind::Session,
                title: "user turn=turn-one".into(),
                detail: "你好".into(),
            }],
            side: Vec::new(),
            footer: String::new(),
            input_hint: String::new(),
        };
        let session_id = ikaros_session::SessionId::from("session-one");
        let turn_id = ikaros_session::TurnId::from("turn-one");
        let events = vec![
            ikaros_session::AgentEvent::new(
                session_id.clone(),
                turn_id.clone(),
                None,
                ikaros_session::AgentEventSource::Model,
                ikaros_session::AgentEventKind::ModelStream(
                    ikaros_models::ModelStreamEvent::TextDelta("你好，".into()),
                ),
                serde_json::Value::Null,
            ),
            ikaros_session::AgentEvent::new(
                session_id,
                turn_id,
                None,
                ikaros_session::AgentEventSource::Model,
                ikaros_session::AgentEventKind::ModelStream(
                    ikaros_models::ModelStreamEvent::TextDelta("我是 Ikaros".into()),
                ),
                serde_json::Value::Null,
            ),
        ];

        screen::apply_live_model_stream_to_cached_screen(&mut screen, &events);

        let assistant = screen
            .main
            .iter()
            .find(|cell| cell.title == "assistant turn=streaming")
            .expect("streaming assistant cell");
        assert_eq!(assistant.kind, super::super::WorkbenchCellKind::Model);
        assert_eq!(assistant.detail, "你好，我是 Ikaros");
    }

    #[test]
    fn cached_screen_pending_user_input_is_inserted_after_previous_turn() {
        let mut screen = WorkbenchScreen {
            title: "Ikaros".into(),
            status: Vec::new(),
            timeline: Vec::new(),
            main: vec![
                WorkbenchCell {
                    kind: super::super::WorkbenchCellKind::Session,
                    title: "user turn=turn-one".into(),
                    detail: "first question".into(),
                },
                WorkbenchCell {
                    kind: super::super::WorkbenchCellKind::Model,
                    title: "assistant turn=turn-one".into(),
                    detail: "first answer".into(),
                },
            ],
            side: Vec::new(),
            footer: String::new(),
            input_hint: String::new(),
        };

        screen::apply_pending_user_input_to_cached_screen(&mut screen, "second question");

        let pending = screen.main.last().expect("pending user cell");
        assert_eq!(pending.kind, super::super::WorkbenchCellKind::Session);
        assert_eq!(pending.title, "user turn=pending");
        assert_eq!(pending.detail, "second question");
    }

    #[test]
    fn screen_progress_status_cell_surfaces_approval_pending_actions() {
        let progress = crate::chat::progress::WorkbenchProgressSnapshot {
            kind: "chat_turn".into(),
            status: "approval_pending".into(),
            elapsed_ms: Some(17),
            detail: "pending_approvals=1 new_approvals=1".into(),
            error_kind: None,
        };

        let rendered = screen_progress_status_cell(Some(&progress)).render();

        assert!(rendered.contains("cell kind=approval title=progress"));
        assert!(rendered.contains("status=approval_pending"));
        assert!(rendered.contains("pending_approvals=1"));
        assert!(rendered.contains("command=/approval"));
        assert!(rendered.contains("approve=/screen approve-selected"));
        assert!(rendered.contains("deny=/screen deny-selected"));
        assert!(rendered.contains("trace=/trace --approval"));
    }

    #[test]
    fn screen_side_cells_show_pending_input_queue_with_clear_action() {
        let mut pending_inputs = std::collections::VecDeque::new();
        pending_inputs.push_back("queued follow up token=sk-secret-value".to_owned());

        let rendered = screen_side_cells(&[], &[], &pending_inputs, &[])
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("input queue"));
        assert!(rendered.contains("pending_inputs=1"));
        assert!(rendered.contains("index=1"));
        assert!(rendered.contains("clear=/queue remove 1"));
        assert!(rendered.contains("clear_all=/queue clear"));
        assert!(rendered.contains("message=queued follow up"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn screen_approval_cells_include_inline_approve_and_deny_actions() {
        let pending = ApprovalRecord {
            request: ikaros_harness::ApprovalRequest {
                id: "approval-one".into(),
                call: ikaros_core::ToolCall {
                    id: "call-one".into(),
                    name: "write_file".into(),
                    risk: ikaros_core::RiskLevel::LocalWrite,
                    input: serde_json::json!({ "api_key": "sk-secret-value" }),
                },
                reason: "needs write confirmation".into(),
                created_at: "2026-06-23T00:00:00Z".into(),
                workspace_root: Some(std::path::PathBuf::from("/tmp/workspace")),
                context: Some(serde_json::json!({
                    "source": "workbench",
                    "scope": "workspace"
                })),
            },
            status: ApprovalStatus::Pending,
            updated_at: "2026-06-23T00:00:01Z".into(),
            note: None,
            result: None,
        };

        let rendered = screen_approval_cells(&[pending])
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("pending approval-one"));
        assert!(rendered.contains("approval_id=approval-one"));
        assert!(rendered.contains("call_id=call-one"));
        assert!(rendered.contains("approve=/approval approve approval-one"));
        assert!(rendered.contains("deny=/approval deny approval-one"));
        assert!(rendered.contains("tool=write_file"));
        assert!(rendered.contains("risk=LocalWrite"));
        assert!(rendered.contains("scope=workspace"));
        assert!(rendered.contains("input_preview="));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn screen_approval_cells_default_scope_to_session_when_unscoped() {
        let pending = ApprovalRecord {
            request: ikaros_harness::ApprovalRequest {
                id: "approval-session".into(),
                call: ikaros_core::ToolCall {
                    id: "call-session".into(),
                    name: "task_summarize".into(),
                    risk: ikaros_core::RiskLevel::SafeRead,
                    input: serde_json::json!({ "text": "summarize this" }),
                },
                reason: "local summary".into(),
                created_at: "2026-06-23T00:00:00Z".into(),
                workspace_root: None,
                context: None,
            },
            status: ApprovalStatus::Pending,
            updated_at: "2026-06-23T00:00:01Z".into(),
            note: None,
            result: None,
        };

        let rendered = screen_approval_cells(&[pending])
            .iter()
            .map(WorkbenchCell::render)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("approval_id=approval-session"));
        assert!(rendered.contains("scope=session"));
    }

    #[test]
    fn approval_overlay_json_line_exports_redacted_pending_actions() {
        let pending = ApprovalRecord {
            request: ikaros_harness::ApprovalRequest {
                id: "approval-json".into(),
                call: ikaros_core::ToolCall {
                    id: "call-json".into(),
                    name: "code_workflow".into(),
                    risk: ikaros_core::RiskLevel::LocalWrite,
                    input: serde_json::json!({ "api_key": "sk-secret-value" }),
                },
                reason: "approve candidate patch with sk-secret-value".into(),
                created_at: "2026-06-23T00:00:00Z".into(),
                workspace_root: None,
                context: Some(serde_json::json!({
                    "operations": {
                        "provider_call": true,
                        "workspace_write": true,
                        "shell": false
                    },
                    "provider": {"name": "mock"},
                    "session": {"session_id": "approval-session", "turn_id": "approval-turn"},
                    "patch": {"candidate_diff_chars": 42}
                })),
            },
            status: ApprovalStatus::Pending,
            updated_at: "2026-06-23T00:00:01Z".into(),
            note: None,
            result: None,
        };

        let line = approval_overlay_json_line(std::slice::from_ref(&pending), None);
        let payload = line
            .strip_prefix("approval_overlay_json: ")
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .expect("approval overlay JSON payload");

        assert_eq!(payload["schema"], "ikaros-workbench-approval-overlay-v1");
        assert_eq!(payload["version"], 1);
        assert_eq!(payload["pending_count"], 1);
        assert_eq!(payload["items"][0]["id"], "approval-json");
        assert_eq!(payload["items"][0]["tool"], "code_workflow");
        assert_eq!(
            payload["items"][0]["actions"]["approve"],
            "/approval approve approval-json"
        );
        assert_eq!(
            payload["items"][0]["context"]["operations"]["provider_call"],
            true
        );
        assert_eq!(
            payload["items"][0]["context"]["session"]["turn_id"],
            "approval-turn"
        );
        let serialized = serde_json::to_string(&payload).expect("serialize payload");
        assert!(!serialized.contains("sk-secret-value"));
        assert!(serialized.contains("[REDACTED_SECRET]"));
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

    fn test_continuation(
        id: &str,
        kind: ikaros_session::SessionContinuationKind,
        status: ikaros_session::SessionContinuationStatus,
        lease_owner: Option<&str>,
    ) -> ikaros_session::SessionContinuation {
        let now = time::OffsetDateTime::now_utc();
        ikaros_session::SessionContinuation {
            continuation_id: ikaros_session::ContinuationId::from(id),
            session_id: SessionId::from("screen-session"),
            turn_id: Some(ikaros_session::TurnId::from("turn-one")),
            parent_continuation_id: None,
            kind,
            status,
            status_reason: None,
            priority: kind.default_priority(),
            payload: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
            claimed_at: None,
            completed_at: None,
            lease_owner: lease_owner.map(str::to_owned),
            lease_expires_at: None,
            attempt_count: 0,
            error: None,
        }
    }
}
