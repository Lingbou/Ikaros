// SPDX-License-Identifier: GPL-3.0-only

use super::types::{
    AgentSummary, AutomationSummary, ConfigIssueSummary, ConfigSummary, ExecutionSummary,
    GatewaySummary, ModelSummary, PersonaSummary, PluginSummary, RagSummary, RuntimeDoctorReport,
    StoreSummary, VoiceSummary,
};
use crate::environment::{resolve_agent_instance, skill_environment};
use ikaros_automation::LocalScheduleStore;
use ikaros_core::{IkarosConfig, IkarosPaths, Result};
use ikaros_gateway::LocalGatewayStore;
use ikaros_harness::PluginCatalog;
use ikaros_memory::{LocalMemoryStore, MemoryProviderRegistry, MemoryStore};
use ikaros_models::ModelUsageLedger;
use ikaros_rag::{LocalRagStore, RagStore, embedding_provider_uses_network};
use ikaros_skills::builtin_registry;
use ikaros_soul::{EmotionState, load_or_default};
use std::path::Path;

pub fn runtime_doctor_report(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<RuntimeDoctorReport> {
    paths.ensure()?;
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    let config_report = config.validate();
    let persona = load_or_default(&paths.persona)?;
    let active_agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let registry = builtin_registry(skill_environment(paths, &active_agent.workspace, &config)?);
    let plugins = PluginCatalog::load(&paths.skills_dir)?;
    let model = active_agent.model_config(&config.model.default);
    let model_provider = active_agent
        .effective_model_provider_config(&config.model.default, &config.providers.model);
    let agent_mode = active_agent.profile.mode.as_str().to_owned();
    let memory_store = LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)?;
    let memory_providers = MemoryProviderRegistry::from_config(
        &paths.memory_dir,
        &config.memory.backend,
        &config.memory.external_providers,
    )?;
    let rag_store = LocalRagStore::new(&paths.rag_dir, &config.rag.backend)?;
    let gateway_store = LocalGatewayStore::new(&paths.gateway_dir);
    let usage_ledger = ModelUsageLedger::new(&paths.audit_dir);
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let daily_token_used_today = usage_ledger.total_for_day(&today)?;
    let (daily_token_remaining_today, daily_token_budget_status) =
        model_budget_status(model.daily_token_budget, daily_token_used_today);

    Ok(RuntimeDoctorReport {
        home: paths.home.clone(),
        workspace: active_agent.workspace.clone(),
        config: ConfigSummary {
            schema_version: config.schema_version,
            valid: config_report.is_valid(),
            issues: config_validation_issues(&config_report),
        },
        persona: PersonaSummary {
            name: persona.identity.name,
            role: persona.identity.role,
        },
        agent: AgentSummary {
            name: active_agent.agent_id.clone(),
            mode: agent_mode,
            workspace_writes: active_agent.profile.workspace_writes.as_str().into(),
            shell: active_agent.profile.shell.as_str().into(),
            network: active_agent.profile.network.as_str().into(),
        },
        agent_profiles: config.agent.profiles.keys().cloned().collect(),
        emotion: format!("{:?}", EmotionState::Neutral),
        model: ModelSummary {
            provider: model.provider.to_string(),
            model: model.model.clone(),
            runtime: model.runtime.clone(),
            transport: model.transport.to_string(),
            api_key_configured: secret_configured(&model_provider.api_key),
            rate_limit_per_minute: model.rate_limit_per_minute,
            daily_token_budget: model.daily_token_budget,
            daily_token_used_today,
            daily_token_remaining_today,
            daily_token_budget_status,
        },
        model_usage_path: usage_ledger.path().to_path_buf(),
        execution: ExecutionSummary {
            sandbox_backend: config.execution.sandbox.backend.to_string(),
            sandbox_image: config.execution.sandbox.image.clone(),
            sandbox_read_scope: config.execution.sandbox.read_scope.to_string(),
            network_enabled: config.execution.network.enabled,
            allow_provider_hosts: config.execution.network.allow_provider_hosts,
            allowed_hosts: config.execution.network.allowed_hosts.len(),
            network_timeout_ms: config.execution.network.timeout_ms,
        },
        memory: StoreSummary {
            backend: config.memory.backend.to_string(),
            path: memory_store.path().to_path_buf(),
        },
        memory_providers,
        rag: RagSummary {
            backend: config.rag.backend.to_string(),
            embedding_uses_network: embedding_provider_uses_network(&config.rag.embedding_provider),
            embedding_egress: rag_embedding_egress_mode(&config.rag.embedding_provider).into(),
            embedding_provider: config.rag.embedding_provider.to_string(),
            embedding_model: config.rag.embedding_model,
            embedding_api_key_configured: secret_configured(&config.providers.embedding.api_key),
            embedding_base_url_configured: !config.providers.embedding.base_url.trim().is_empty(),
            path: rag_store.path().to_path_buf(),
        },
        voice: VoiceSummary {
            tts_provider: config.voice.tts.provider.to_string(),
            tts_model: config.voice.tts.model,
            asr_provider: config.voice.asr.provider.to_string(),
            asr_model: config.voice.asr.model,
        },
        automation: AutomationSummary {
            schedules_path: LocalScheduleStore::new(&paths.automation_dir)
                .path()
                .to_path_buf(),
        },
        gateway: GatewaySummary {
            inbox_path: gateway_store.inbox_path().to_path_buf(),
            outbox_path: gateway_store.outbox_path().to_path_buf(),
        },
        skills: registry.names(),
        plugins: PluginSummary {
            plugin_count: plugins.plugin_count(),
            enabled_plugin_count: plugins.enabled_plugin_count(),
            disabled_plugin_count: plugins.disabled_plugin_count(),
            active_declared_skill_count: plugins.declared_skill_count(),
            warning_count: plugins.warnings.len(),
        },
        audit_path: paths.audit_dir.join("audit.jsonl"),
    })
}

fn rag_embedding_egress_mode(provider: &str) -> &'static str {
    if embedding_provider_uses_network(provider) {
        "session_network_egress"
    } else {
        "local"
    }
}

fn model_budget_status(budget: Option<u32>, used: u32) -> (Option<u32>, String) {
    match budget {
        Some(budget) => {
            let remaining = budget.saturating_sub(used);
            let status = if used >= budget {
                "exhausted"
            } else if used.saturating_mul(10) >= budget.saturating_mul(9) {
                "near_limit"
            } else {
                "ok"
            };
            (Some(remaining), status.into())
        }
        None => (None, "unbounded".into()),
    }
}

fn secret_configured(inline: &str) -> bool {
    !inline.trim().is_empty()
}

fn config_validation_issues(
    report: &ikaros_core::ConfigValidationReport,
) -> Vec<ConfigIssueSummary> {
    report
        .errors
        .iter()
        .map(|issue| ConfigIssueSummary {
            severity: "error".into(),
            path: issue.path.clone(),
            message: issue.message.clone(),
        })
        .chain(report.warnings.iter().map(|issue| ConfigIssueSummary {
            severity: "warning".into(),
            path: issue.path.clone(),
            message: issue.message.clone(),
        }))
        .collect()
}
