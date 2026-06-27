// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosConfig, ResolvedAgentProfile};
use ikaros_harness::ExecutionSession;
use ikaros_models::ModelUsageLedger;
use ikaros_runtime::ChatRunOptions;

use super::super::workbench::terminal_inline;

pub(in crate::chat) fn available_agent_lines(config: &IkarosConfig, active: &str) -> Vec<String> {
    let mut lines = config
        .agent
        .profiles
        .iter()
        .map(|(name, profile)| {
            let marker = if name == active { "*" } else { " " };
            format!(
                "{marker} {} mode={} workspace_writes={} shell={} network={} - {}",
                terminal_inline(name),
                profile.mode,
                profile.workspace_writes,
                profile.shell,
                profile.network,
                terminal_inline(&profile.description)
            )
        })
        .collect::<Vec<_>>();
    for (name, instance) in &config.agent.instances {
        let marker = if name == active { "*" } else { " " };
        let workspace = instance
            .workspace
            .as_deref()
            .map(|workspace| workspace.display().to_string())
            .unwrap_or_else(|| "<default workspace>".into());
        lines.push(format!(
            "{marker} {} instance profile={} workspace={}",
            terminal_inline(name),
            terminal_inline(&instance.profile),
            terminal_inline(&workspace)
        ));
    }
    lines
}

pub(in crate::chat) fn available_agent_lines_for_human(
    config: &IkarosConfig,
    active: &str,
) -> Vec<String> {
    let mut lines = vec![
        "• Agents".to_owned(),
        format!("  active: {}", terminal_inline(active)),
    ];
    if config.agent.profiles.is_empty() && config.agent.instances.is_empty() {
        lines.push("  no configured agents".to_owned());
        return lines;
    }
    if !config.agent.profiles.is_empty() {
        lines.push("  profiles:".to_owned());
        for (name, profile) in &config.agent.profiles {
            let marker = if name == active { "*" } else { " " };
            lines.push(format!(
                "  {marker} {} mode={} writes={} shell={} network={}",
                terminal_inline(name),
                profile.mode,
                profile.workspace_writes,
                profile.shell,
                profile.network
            ));
            if !profile.description.trim().is_empty() {
                lines.push(format!("    {}", terminal_inline(&profile.description)));
            }
        }
    }
    if !config.agent.instances.is_empty() {
        lines.push("  instances:".to_owned());
        for (name, instance) in &config.agent.instances {
            let marker = if name == active { "*" } else { " " };
            let workspace = instance
                .workspace
                .as_deref()
                .map(|workspace| workspace.display().to_string())
                .unwrap_or_else(|| "<default workspace>".into());
            lines.push(format!(
                "  {marker} {} profile={} workspace={}",
                terminal_inline(name),
                terminal_inline(&instance.profile),
                terminal_inline(&workspace)
            ));
        }
    }
    lines
}

pub(in crate::chat) struct InteractiveChatStatusInput<'a> {
    pub(in crate::chat) agent: &'a ResolvedAgentProfile,
    pub(in crate::chat) session: &'a ExecutionSession,
    pub(in crate::chat) chat_session_id: &'a str,
    pub(in crate::chat) state_dir: &'a std::path::Path,
    pub(in crate::chat) options: &'a ChatRunOptions,
    pub(in crate::chat) emotion: &'a str,
    pub(in crate::chat) usage_ledger: &'a ModelUsageLedger,
}

pub(in crate::chat) fn format_interactive_chat_status(
    input: InteractiveChatStatusInput<'_>,
) -> String {
    format!(
        "agent={} mode={} emotion={} memory_context={} memory_search_limit={} rag_context={} history_context_limit={} history_summary_limit={} context_token_budget={} relationship_learning={} agent_loop={} effective_agent_loop={} stream={} no_context={} content_blocks={} scope={} chat_session={} audit={} model_usage={} session_state_db={} chat_timeline=session_store",
        terminal_inline(&input.agent.name),
        input.agent.mode(),
        terminal_inline(input.emotion),
        input.agent.profile.memory_context,
        input.options.memory_search_limit,
        input.agent.profile.rag_context,
        input.options.history_context_limit,
        input.options.history_summary_limit,
        input.options.context_token_budget,
        input.options.relationship_learning,
        input.options.agent_loop,
        input.options.agent_loop && input.options.content_blocks.is_empty(),
        input.options.stream,
        input.options.no_context,
        input.options.content_blocks.len(),
        input
            .options
            .scope
            .as_deref()
            .map(terminal_inline)
            .unwrap_or_else(|| "none".into()),
        terminal_inline(input.chat_session_id),
        terminal_inline(&input.session.audit.path().display().to_string()),
        terminal_inline(&input.usage_ledger.path().display().to_string()),
        terminal_inline(&input.state_dir.join("state.db").display().to_string()),
    )
}
