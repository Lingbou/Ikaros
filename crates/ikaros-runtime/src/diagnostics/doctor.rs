// SPDX-License-Identifier: GPL-3.0-only

use super::types::{
    AgentSummary, AutomationSummary, GatewaySummary, ModelSummary, PersonaSummary, PluginSummary,
    RagSummary, RuntimeDoctorReport, StoreSummary, VoiceSummary,
};
use crate::environment::{resolve_agent, skill_environment};
use ikaros_automation::LocalScheduleStore;
use ikaros_core::{IkarosConfig, IkarosPaths, Result};
use ikaros_gateway::LocalGatewayStore;
use ikaros_harness::PluginCatalog;
use ikaros_memory::{LocalMemoryStore, MemoryProviderRegistry, MemoryStore};
use ikaros_models::ModelUsageLedger;
use ikaros_rag::{LocalRagStore, RagStore};
use ikaros_skills::builtin_registry;
use ikaros_soul::{EmotionState, load_or_default};
use std::path::Path;

pub fn runtime_doctor_report(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<RuntimeDoctorReport> {
    paths.ensure()?;
    let config = IkarosConfig::load(&paths.config)?;
    let persona = load_or_default(&paths.persona)?;
    let registry = builtin_registry(skill_environment(paths, workspace, &config)?);
    let plugins = PluginCatalog::load(&paths.skills_dir)?;
    let active_agent = resolve_agent(&config, agent_override)?;
    let agent_mode = active_agent.mode().as_str().to_owned();
    let memory_store = LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)?;
    let memory_providers = MemoryProviderRegistry::from_config(
        &paths.memory_dir,
        &config.memory.backend,
        &config.memory.external_providers,
    )?;
    let rag_store = LocalRagStore::new(&paths.rag_dir, &config.rag.backend)?;
    let gateway_store = LocalGatewayStore::new(&paths.gateway_dir);

    Ok(RuntimeDoctorReport {
        home: paths.home.clone(),
        workspace: workspace.to_path_buf(),
        persona: PersonaSummary {
            name: persona.identity.name,
            role: persona.identity.role,
        },
        agent: AgentSummary {
            name: active_agent.name,
            mode: agent_mode,
            workspace_writes: active_agent.profile.workspace_writes.as_str().into(),
            shell: active_agent.profile.shell.as_str().into(),
            network: active_agent.profile.network.as_str().into(),
        },
        agent_profiles: config.agent.profiles.keys().cloned().collect(),
        emotion: format!("{:?}", EmotionState::Neutral),
        model: ModelSummary {
            provider: config.model.default.provider,
            model: config.model.default.model,
            runtime: config.model.default.runtime,
            transport: config.model.default.transport,
            api_key_configured: secret_configured(&config.providers.model.api_key),
            rate_limit_per_minute: config.model.default.rate_limit_per_minute,
            daily_token_budget: config.model.default.daily_token_budget,
        },
        model_usage_path: ModelUsageLedger::new(&paths.audit_dir).path().to_path_buf(),
        memory: StoreSummary {
            backend: config.memory.backend,
            path: memory_store.path().to_path_buf(),
        },
        memory_providers,
        rag: RagSummary {
            backend: config.rag.backend,
            embedding_provider: config.rag.embedding_provider,
            embedding_model: config.rag.embedding_model,
            embedding_api_key_configured: secret_configured(&config.providers.embedding.api_key),
            path: rag_store.path().to_path_buf(),
        },
        voice: VoiceSummary {
            tts_provider: config.voice.tts.provider,
            tts_model: config.voice.tts.model,
            asr_provider: config.voice.asr.provider,
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

fn secret_configured(inline: &str) -> bool {
    !inline.trim().is_empty()
}
