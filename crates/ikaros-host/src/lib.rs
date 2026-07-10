// SPDX-License-Identifier: GPL-3.0-only
//! Host-side assembly of runtime locations, execution sessions, and skill registries.

mod agent;
mod approval;
mod builder;
mod diagnostics;
mod location;
mod mcp;
mod memory;
mod model_http;
mod network;
mod persona;
mod provider_probe;
pub mod relationship;
mod services;

pub use agent::{agent_profile_report, agent_profiles_report};
pub use approval::record_approval_resolution;
pub use builder::{
    ChatModelServices, ChatRuntimeServices, RuntimeHarnessModelServices,
    chat_model_services_for_session, chat_model_services_shape_checked, chat_runtime_services,
    chat_runtime_services_for_instance, host_agent_context, host_agent_context_shape_checked,
    runtime_execution_env, runtime_harness, runtime_harness_model_provider,
    runtime_harness_model_services, session_and_registry, session_and_registry_for_agent,
    session_and_registry_for_instance, session_state_db_candidates,
    session_state_db_candidates_with_config, skill_environment,
};
pub use diagnostics::{
    AgentSummary, ExecutionSummary, ModelSummary, PersonaSummary, PluginSummary,
    ProviderDebugMatrixReport, ProviderHealthReport, ProviderInspectFallbackRow,
    ProviderInspectReport, ProviderMatrixReport, ProviderMatrixRow, ProviderProfileCatalogRow,
    ProviderProfilesReport, RagSummary, RuntimeDoctorReport, RuntimeInitReport, StoreSummary,
    WorkbenchModelBudgetStatus, WorkbenchModelCostStatus, WorkbenchProviderFallbackStatus,
    WorkbenchProviderHealthStatus, WorkbenchProviderStatusReport, configured_sandbox_debug_report,
    debug_sandbox_report, initialize_runtime_home, initialize_runtime_home_with_options,
    model_budget_status_report, provider_debug_matrix_report, provider_debug_report,
    provider_health_report, provider_inspect_report, provider_matrix_report,
    provider_profiles_report, runtime_doctor_report, sandbox_probe,
    workbench_provider_status_report,
};
pub use ikaros_core::{resolve_agent, resolve_agent_instance};
pub use ikaros_execution::harness::recent_policy_decisions;
pub use location::RuntimeLocation;
pub use mcp::{
    McpServerReport, McpServersReport, configured_mcp_server, mcp_servers_report,
    mcp_servers_report_from_config,
};
pub use memory::{
    MemoryCandidateStores, MemoryProjectionStores, local_memory_store, memory_candidate_stores,
    memory_projection_stores, memory_provider_registry,
};
pub use model_http::EgressModelHttpClient;
pub use network::provider_egress_allowed_hosts;
pub use persona::{
    PersonaPatch, PersonaWriteReport, render_persona_markdown, reset_persona, update_persona,
};
pub use provider_probe::{
    ModelProviderLiveProbeReport, embedding_provider_live_probe, model_provider_live_probe,
};
pub use relationship::{
    RelationshipMutationReport, RelationshipNote, RelationshipSnapshot,
    forget_relationship_note_by_id, forget_relationship_scope, relationship_snapshot,
    remember_relationship_note,
};
pub use services::{HostAgentContext, HostServices, RuntimeHarness};

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
        let overlay = harness
            .session
            .sandbox
            .agent
            .as_ref()
            .expect("agent overlay");
        assert_eq!(overlay.agent_id.as_deref(), Some("repo-build"));
        assert_eq!(overlay.profile_name, "build");

        let chat_services = chat_runtime_services(
            &paths,
            &harness.config,
            &fallback_workspace,
            Some("repo-build"),
        )
        .expect("chat runtime services");
        assert_eq!(chat_services.agent_instance.agent_id, "repo-build");
        assert_eq!(
            chat_services.session.sandbox.workspace_root,
            configured_workspace
        );
        assert!(chat_services.registry.get("fs_read").is_some());
        assert_eq!(
            chat_services.model.model_config.provider.to_string(),
            "mock"
        );
        assert_eq!(chat_services.model.model_config.model, "mock-ikaros");

        let host_context = host_agent_context(&paths, &fallback_workspace, Some("repo-build"))
            .expect("host agent context");
        assert_eq!(host_context.agent_instance.agent_id, "repo-build");
        assert_eq!(host_context.location.workspace, configured_workspace);
        assert_eq!(
            host_context.location.state_dir,
            paths.home.join("agents/repo-build")
        );

        let primary_state_db = configured_workspace.join("unused-state.db");
        assert!(!primary_state_db.exists());
        let active_state_db = paths.home.join("agents/repo-build/state.db");
        fs::create_dir_all(active_state_db.parent().expect("state db parent")).expect("state dir");
        fs::write(&active_state_db, "").expect("active state db");
        let other_state_db = paths.home.join("agents/other-agent/state.db");
        fs::create_dir_all(other_state_db.parent().expect("other state db parent"))
            .expect("other state dir");
        fs::write(&other_state_db, "").expect("other state db");

        let active_only = session_state_db_candidates_with_config(
            &paths,
            &harness.config,
            &fallback_workspace,
            Some("repo-build"),
            false,
            true,
        )
        .expect("active-only state db candidates");
        assert_eq!(active_only, vec![active_state_db.clone()]);

        let all_agents = session_state_db_candidates_with_config(
            &paths,
            &harness.config,
            &fallback_workspace,
            Some("repo-build"),
            true,
            true,
        )
        .expect("all-agent state db candidates");
        assert_eq!(all_agents.first(), Some(&active_state_db));
        assert!(all_agents.contains(&other_state_db));
    }
}
