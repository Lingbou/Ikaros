// SPDX-License-Identifier: GPL-3.0-only
//! Host-side assembly of runtime locations, execution sessions, and skill registries.

mod builder;
mod location;
mod network;
mod services;

pub use builder::{
    recent_policy_decisions, resolve_agent, resolve_agent_instance, runtime_execution_env,
    runtime_harness, session_and_registry, session_and_registry_for_agent,
    session_and_registry_for_instance, skill_environment,
};
pub use location::RuntimeLocation;
pub use network::provider_egress_allowed_hosts;
pub use services::{HostServices, RuntimeHarness};

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_core::IkarosPaths;
    use std::fs;

    #[test]
    fn runtime_harness_resolves_agent_instance_location_and_services() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path().join("home"));
        paths.ensure().expect("paths");
        let configured_workspace = temp.path().join("instance-workspace");
        fs::create_dir_all(&configured_workspace).expect("workspace");
        let configured_workspace_yaml =
            serde_json::to_string(&configured_workspace.display().to_string())
                .expect("workspace yaml string");
        fs::write(
            &paths.config,
            format!(
                r#"
schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

agent:
  default: build
  instances:
    repo-build:
      profile: build
      workspace: {configured_workspace_yaml}

rag:
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
"#,
            ),
        )
        .expect("config");

        let fallback_workspace = temp.path().join("fallback-workspace");
        let harness = runtime_harness(&paths, &fallback_workspace, Some("repo-build"))
            .expect("runtime harness");

        assert_eq!(harness.agent_instance.agent_id, "repo-build");
        assert_eq!(harness.agent_instance.profile_name, "build");
        assert_eq!(harness.location.agent_id, "repo-build");
        assert_eq!(harness.location.profile_name, "build");
        assert_eq!(harness.location.workspace, configured_workspace);
        assert_eq!(
            harness.location.state_dir,
            paths.home.join("agents/repo-build")
        );
        assert_eq!(harness.location.audit_dir, paths.audit_dir);
        assert_eq!(harness.session.sandbox.workspace_root, configured_workspace);
        assert!(harness.registry.get("fs_read").is_some());
        assert!(harness.registry.get("vision_describe").is_some());
        let overlay = harness
            .session
            .sandbox
            .agent
            .as_ref()
            .expect("agent overlay");
        assert_eq!(overlay.agent_id.as_deref(), Some("repo-build"));
        assert_eq!(overlay.profile_name, "build");
    }
}
