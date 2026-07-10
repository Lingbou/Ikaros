// SPDX-License-Identifier: GPL-3.0-only

use ikaros_host::{RuntimeDoctorReport, RuntimeInitReport};

pub(super) fn print_init_report(report: &RuntimeInitReport) {
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
        "persona_dir: {} ({})",
        report.persona_dir.display(),
        if report.persona_created {
            "created"
        } else {
            "existing"
        }
    );
    println!("persona_profile: {}", report.persona_profile.display());
    println!("memory: {}", report.memory_dir.display());
    println!("rag: {}", report.rag_dir.display());
    println!("audit: {}", report.audit_dir.display());
}

pub(super) fn print_doctor_report(report: &RuntimeDoctorReport) {
    println!("home: {}", report.home.display());
    println!("workspace: {}", report.workspace.display());
    println!("config_schema_version: {}", report.config.schema_version);
    println!("config_valid: {}", report.config.valid);
    for issue in &report.config.issues {
        println!(
            "config_issue: {}: {}: {}",
            issue.severity, issue.path, issue.message
        );
    }
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
        "model_limits: rate_limit_per_minute={:?} daily_token_budget={:?} daily_token_used_today={} daily_token_remaining_today={:?} daily_token_budget_status={}",
        report.model.rate_limit_per_minute,
        report.model.daily_token_budget,
        report.model.daily_token_used_today,
        report.model.daily_token_remaining_today,
        report.model.daily_token_budget_status
    );
    println!("model_usage: {}", report.model_usage_path.display());
    println!(
        "execution: sandbox_backend={} sandbox_image={} read_scope={} network_enabled={} allow_provider_hosts={} allowed_hosts={} network_timeout_ms={}",
        report.execution.sandbox_backend,
        display_optional_config_value(&report.execution.sandbox_image),
        report.execution.sandbox_read_scope,
        report.execution.network_enabled,
        report.execution.allow_provider_hosts,
        report.execution.allowed_hosts,
        report.execution.network_timeout_ms
    );
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
        "rag: backend={} embedding_provider={} embedding_model={} embedding_key_configured={} embedding_base_url_configured={} embedding_uses_network={} embedding_egress={} path={}",
        report.rag.backend,
        report.rag.embedding_provider,
        report.rag.embedding_model,
        report.rag.embedding_api_key_configured,
        report.rag.embedding_base_url_configured,
        report.rag.embedding_uses_network,
        report.rag.embedding_egress,
        report.rag.path.display()
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

fn display_optional_config_value(value: &str) -> &str {
    if value.trim().is_empty() {
        "none"
    } else {
        value
    }
}

pub(super) fn display_optional_model(model: &str) -> &str {
    if model.is_empty() { "none" } else { model }
}
