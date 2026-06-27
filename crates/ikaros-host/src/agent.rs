// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{AgentProfile, IkarosConfig, IkarosPaths, Result, redact_json, resolve_agent};
use serde_json::{Map, Value, json};

pub fn agent_profiles_report(paths: &IkarosPaths) -> Result<Value> {
    let config = IkarosConfig::load(&paths.config)?;
    Ok(agent_profiles_report_from_config(&config))
}

pub fn agent_profile_report(paths: &IkarosPaths, agent_override: Option<&str>) -> Result<Value> {
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent(&config, agent_override)?;
    Ok(agent_profile_report_from_profile(
        &agent.name,
        &agent.profile,
    ))
}

fn agent_profiles_report_from_config(config: &IkarosConfig) -> Value {
    let profiles = config
        .agent
        .profiles
        .iter()
        .map(|(name, profile)| {
            (
                name.clone(),
                json!({
                    "mode": profile.mode,
                    "description": profile.description,
                    "memory_context": profile.memory_context,
                    "rag_context": profile.rag_context,
                    "permissions": permissions_json(profile),
                }),
            )
        })
        .collect::<Map<_, _>>();
    redact_json(json!({
        "default": config.agent.default,
        "profiles": profiles,
    }))
}

fn agent_profile_report_from_profile(name: &str, profile: &AgentProfile) -> Value {
    redact_json(json!({
        "name": name,
        "mode": profile.mode,
        "description": profile.description,
        "persona_overlay": profile.persona_overlay,
        "memory_context": profile.memory_context,
        "rag_context": profile.rag_context,
        "permissions": permissions_json(profile),
    }))
}

fn permissions_json(profile: &AgentProfile) -> Value {
    json!({
        "workspace_writes": profile.workspace_writes,
        "shell": profile.shell,
        "network": profile.network,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_core::AgentPermission;

    #[test]
    fn agent_profile_report_redacts_secret_like_profile_text() {
        let mut profile = AgentProfile::general();
        profile.description = "use token=abc123 for nothing".into();
        profile.persona_overlay = "never echo sk-test-secret".into();

        let rendered = serde_json::to_string(&agent_profile_report_from_profile("safe", &profile))
            .expect("json");

        assert!(!rendered.contains("abc123"));
        assert!(!rendered.contains("sk-test-secret"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn permissions_snapshot_preserves_policy_intent() {
        let mut profile = AgentProfile::plan();
        profile.workspace_writes = AgentPermission::Deny;
        profile.shell = AgentPermission::Ask;
        profile.network = AgentPermission::Allow;

        let rendered = permissions_json(&profile);

        assert_eq!(rendered["workspace_writes"], "deny");
        assert_eq!(rendered["shell"], "ask");
        assert_eq!(rendered["network"], "allow");
    }
}
