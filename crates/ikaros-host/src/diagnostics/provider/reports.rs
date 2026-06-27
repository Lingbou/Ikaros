// SPDX-License-Identifier: GPL-3.0-only

use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ProviderDebugMatrixReport {
    pub health_log: PathBuf,
    pub rows: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchProviderStatusReport {
    pub provider: String,
    pub model: String,
    pub configured_profile: String,
    pub provider_profile: String,
    pub profile_source: &'static str,
    pub context_window: String,
    pub default_output_tokens: String,
    pub tokenizer: String,
    pub runtime: String,
    pub transport: String,
    pub temperature_policy: String,
    pub reasoning_policy: String,
    pub message_policy: String,
    pub tool_schema_policy: String,
    pub request_body_policy: String,
    pub prompt_cache_policy: String,
    pub retry_without_parameters: String,
    pub streaming: String,
    pub tool_calls: String,
    pub reasoning: String,
    pub json_mode: String,
    pub network: String,
    pub image_input: String,
    pub audio_input: String,
    pub file_input: String,
    pub health: WorkbenchProviderHealthStatus,
    pub budget: WorkbenchModelBudgetStatus,
    pub cost: WorkbenchModelCostStatus,
    pub fallback_count: usize,
    pub fallback_chain: String,
    pub fallbacks: Vec<WorkbenchProviderFallbackStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchModelBudgetStatus {
    pub daily_token_budget: Option<u32>,
    pub used_today: u32,
    pub remaining_today: Option<u32>,
    pub budget_status: &'static str,
    pub suggested_daily_token_budget: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchModelCostStatus {
    pub currency: String,
    pub input_per_million: String,
    pub output_per_million: String,
    pub cache_read_per_million: String,
    pub cache_write_per_million: String,
    pub estimated_cost_today: String,
    pub cache_read_tokens_today: u64,
    pub cache_write_tokens_today: u64,
    pub cache_accounting: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchProviderHealthStatus {
    pub status: String,
    pub consecutive_failures: u32,
    pub last_error_kind: String,
    pub last_error_summary: String,
    pub cooldown_until: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchProviderFallbackStatus {
    pub index: usize,
    pub provider: String,
    pub model: String,
    pub configured_profile: String,
    pub provider_profile: String,
    pub live_smoke: &'static str,
    pub streaming: String,
    pub tool_calls: String,
    pub reasoning: String,
    pub network: String,
    pub image_input: String,
    pub audio_input: String,
    pub file_input: String,
    pub context_window: String,
    pub default_output_tokens: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMatrixReport {
    pub live: bool,
    pub rows: Vec<ProviderMatrixRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMatrixRow {
    pub kind: String,
    pub provider: String,
    pub model: String,
    pub base_url_configured: bool,
    pub api_key_configured: bool,
    pub live_smoke: &'static str,
    pub live_probe: String,
    pub probe_detail: String,
    pub health_status: String,
    pub consecutive_failures: u32,
    pub cooldown_until: Option<String>,
    pub configured_profile: String,
    pub provider_profile: String,
    pub profile_source: &'static str,
    pub temperature_policy: String,
    pub reasoning_policy: String,
    pub message_policy: String,
    pub tool_schema_policy: String,
    pub request_body_policy: String,
    pub retry_without_parameters: String,
    pub context_window: String,
    pub default_output_tokens: String,
    pub tokenizer: String,
    pub streaming: String,
    pub tool_calls: String,
    pub reasoning: String,
    pub json_mode: String,
    pub network: String,
    pub image_input: String,
    pub audio_input: String,
    pub file_input: String,
    pub cost_input_per_million: String,
    pub cost_output_per_million: String,
    pub cost_cache_read_per_million: String,
    pub cost_cache_write_per_million: String,
    pub cost_currency: String,
    pub usage_requests_today: usize,
    pub usage_prompt_tokens_today: u64,
    pub usage_completion_tokens_today: u64,
    pub usage_total_tokens_today: u64,
    pub cache_read_tokens_today: u64,
    pub cache_write_tokens_today: u64,
    pub estimated_cost_today: String,
    pub cache_accounting: &'static str,
    pub fallback_role: &'static str,
    pub fallback_count: usize,
    pub fallback_models: Vec<String>,
    pub debug_hint: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderInspectReport {
    pub provider: String,
    pub model: String,
    pub configured_profile: String,
    pub profile: String,
    pub profile_source: &'static str,
    pub temperature_policy: String,
    pub reasoning_policy: String,
    pub message_policy: String,
    pub tool_schema_policy: String,
    pub request_body_policy: String,
    pub retry_without_parameters: String,
    pub fallback_rows: Vec<ProviderInspectFallbackRow>,
    pub context_window: u32,
    pub default_output_tokens: u32,
    pub tokenizer: String,
    pub streaming: bool,
    pub tool_calls: bool,
    pub reasoning: bool,
    pub json_mode: bool,
    pub network: bool,
    pub image_input: bool,
    pub audio_input: bool,
    pub file_input: bool,
    pub health: String,
    pub cost_input_per_million: Option<f64>,
    pub cost_output_per_million: Option<f64>,
    pub cost_cache_read_per_million: Option<f64>,
    pub cost_cache_write_per_million: Option<f64>,
    pub cost_currency: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderInspectFallbackRow {
    pub index: usize,
    pub provider: String,
    pub model: String,
    pub configured_profile: String,
    pub profile: String,
    pub live_smoke: &'static str,
    pub streaming: bool,
    pub tool_calls: bool,
    pub reasoning: bool,
    pub network: bool,
    pub image_input: bool,
    pub audio_input: bool,
    pub file_input: bool,
    pub context_window: u32,
    pub default_output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderProfilesReport {
    pub provider: String,
    pub rows: Vec<ProviderProfileCatalogRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderProfileCatalogRow {
    pub provider: String,
    pub profile: String,
    pub auto_base_url_markers: String,
    pub auto_model_markers: String,
    pub auto_model_tail_prefixes: String,
    pub temperature_policy: String,
    pub reasoning_policy: String,
    pub message_policy: String,
    pub tool_schema_policy: String,
    pub request_body_policy: String,
    pub retry_without_parameters: String,
    pub context_window: u32,
    pub default_output_tokens: u32,
    pub tokenizer: String,
    pub streaming: bool,
    pub tool_calls: bool,
    pub reasoning: bool,
    pub json_mode: bool,
    pub network: bool,
    pub image_input: bool,
    pub audio_input: bool,
    pub file_input: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderHealthReport {
    LiveOk {
        provider: String,
        model: String,
        usage_total: u32,
    },
    LiveFailed {
        error: String,
    },
    Local {
        provider: String,
        model: String,
        health: String,
        consecutive_failures: u32,
        last_error_kind: Option<String>,
        last_error_summary: Option<String>,
        cooldown_until: Option<String>,
        health_log: PathBuf,
    },
}
