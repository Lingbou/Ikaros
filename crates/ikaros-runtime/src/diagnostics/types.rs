// SPDX-License-Identifier: GPL-3.0-only

use ikaros_memory::MemoryProviderRegistry;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeInitReport {
    pub home: PathBuf,
    pub config: PathBuf,
    pub persona: PathBuf,
    pub memory_dir: PathBuf,
    pub rag_dir: PathBuf,
    pub automation_dir: PathBuf,
    pub gateway_dir: PathBuf,
    pub audit_dir: PathBuf,
    pub config_created: bool,
    pub persona_created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeDoctorReport {
    pub home: PathBuf,
    pub workspace: PathBuf,
    pub config: ConfigSummary,
    pub persona: PersonaSummary,
    pub agent: AgentSummary,
    pub agent_profiles: Vec<String>,
    pub emotion: String,
    pub model: ModelSummary,
    pub model_usage_path: PathBuf,
    pub execution: ExecutionSummary,
    pub memory: StoreSummary,
    pub memory_providers: MemoryProviderRegistry,
    pub rag: RagSummary,
    pub voice: VoiceSummary,
    pub automation: AutomationSummary,
    pub gateway: GatewaySummary,
    pub skills: Vec<String>,
    pub plugins: PluginSummary,
    pub audit_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigSummary {
    pub schema_version: u32,
    pub valid: bool,
    pub issues: Vec<ConfigIssueSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigIssueSummary {
    pub severity: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonaSummary {
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSummary {
    pub name: String,
    pub mode: String,
    pub workspace_writes: String,
    pub shell: String,
    pub network: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelSummary {
    pub provider: String,
    pub model: String,
    pub runtime: String,
    pub transport: String,
    pub api_key_configured: bool,
    pub rate_limit_per_minute: Option<u32>,
    pub daily_token_budget: Option<u32>,
    pub daily_token_used_today: u32,
    pub daily_token_remaining_today: Option<u32>,
    pub daily_token_budget_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionSummary {
    pub sandbox_backend: String,
    pub sandbox_image: String,
    pub sandbox_read_scope: String,
    pub network_enabled: bool,
    pub allow_provider_hosts: bool,
    pub allowed_hosts: usize,
    pub network_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreSummary {
    pub backend: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RagSummary {
    pub backend: String,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_api_key_configured: bool,
    pub embedding_base_url_configured: bool,
    pub embedding_uses_network: bool,
    pub embedding_egress: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceSummary {
    pub tts_provider: String,
    pub tts_model: String,
    pub asr_provider: String,
    pub asr_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationSummary {
    pub schedules_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewaySummary {
    pub inbox_path: PathBuf,
    pub outbox_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginSummary {
    pub plugin_count: usize,
    pub enabled_plugin_count: usize,
    pub disabled_plugin_count: usize,
    pub active_declared_skill_count: usize,
    pub warning_count: usize,
}
