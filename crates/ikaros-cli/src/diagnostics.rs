// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use clap::Args;
use ikaros_core::IkarosPaths;
use ikaros_runtime::{
    RuntimeDoctorReport, RuntimeInitReport, initialize_runtime_home, runtime_doctor_report,
};
use std::path::Path;

#[derive(Debug, Args, Default)]
pub(crate) struct DoctorArgs {}

pub(crate) fn init(paths: &IkarosPaths) -> Result<()> {
    let report = initialize_runtime_home(paths)?;
    println!("Ikaros initialized");
    print_init_report(&report);
    Ok(())
}

pub(crate) fn doctor(
    _args: DoctorArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let report = runtime_doctor_report(paths, workspace, agent_override)?;
    println!("Ikaros doctor");
    print_doctor_report(&report);
    Ok(())
}

fn print_init_report(report: &RuntimeInitReport) {
    println!("home: {}", report.home.display());
    println!(
        "config: {} ({})",
        report.config.display(),
        if report.config_created {
            "created"
        } else {
            "existing"
        }
    );
    println!(
        "persona: {} ({})",
        report.persona.display(),
        if report.persona_created {
            "created"
        } else {
            "existing"
        }
    );
    println!("memory: {}", report.memory_dir.display());
    println!("rag: {}", report.rag_dir.display());
    println!("automation: {}", report.automation_dir.display());
    println!("gateway: {}", report.gateway_dir.display());
    println!("audit: {}", report.audit_dir.display());
}

fn print_doctor_report(report: &RuntimeDoctorReport) {
    println!("home: {}", report.home.display());
    println!("workspace: {}", report.workspace.display());
    println!("persona: {} ({})", report.persona.name, report.persona.role);
    println!(
        "agent: {} mode={} writes={} shell={} network={}",
        report.agent.name,
        report.agent.mode,
        report.agent.workspace_writes,
        report.agent.shell,
        report.agent.network
    );
    println!("agent_profiles: {}", report.agent_profiles.join(", "));
    println!("emotion: {}", report.emotion);
    println!(
        "model: provider={} model={} key_configured={}",
        report.model.provider, report.model.model, report.model.api_key_configured
    );
    println!(
        "model_limits: rate_limit_per_minute={:?} daily_token_budget={:?}",
        report.model.rate_limit_per_minute, report.model.daily_token_budget
    );
    println!("model_usage: {}", report.model_usage_path.display());
    println!(
        "memory: backend={} path={}",
        report.memory.backend,
        report.memory.path.display()
    );
    let active_external = report
        .memory_providers
        .active_external()
        .map(|provider| provider.id.as_str())
        .unwrap_or("none");
    println!(
        "memory_providers: local={} external_active={} external_configured={} issues={}",
        report.memory_providers.active_local.id,
        active_external,
        report.memory_providers.external.len(),
        report.memory_providers.issues.len()
    );
    for issue in &report.memory_providers.issues {
        println!("memory_provider_issue: {issue}");
    }
    println!(
        "rag: backend={} embedding_provider={} embedding_model={} embedding_key_configured={} path={}",
        report.rag.backend,
        report.rag.embedding_provider,
        report.rag.embedding_model,
        report.rag.embedding_api_key_configured,
        report.rag.path.display()
    );
    println!(
        "voice: tts_provider={} tts_model={} asr_provider={} asr_model={}",
        report.voice.tts_provider,
        report.voice.tts_model,
        report.voice.asr_provider,
        report.voice.asr_model
    );
    println!(
        "automation: schedules={}",
        report.automation.schedules_path.display()
    );
    println!(
        "gateway: inbox={} outbox={}",
        report.gateway.inbox_path.display(),
        report.gateway.outbox_path.display()
    );
    println!("skills: {}", report.skills.join(", "));
    println!(
        "plugins: {} plugin(s), {} enabled, {} disabled, {} active declared skill(s), {} warning(s)",
        report.plugins.plugin_count,
        report.plugins.enabled_plugin_count,
        report.plugins.disabled_plugin_count,
        report.plugins.active_declared_skill_count,
        report.plugins.warning_count
    );
    println!("audit: {}", report.audit_path.display());
}
