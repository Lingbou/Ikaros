// SPDX-License-Identifier: GPL-3.0-only

use super::schema_sanitizer::sanitize_moonshot_tool_definitions;
use crate::types::{
    ModelContextProfile, ModelRequestOptions, ModelTokenizerKind, ModelToolDefinition,
    ReasoningEffort,
};
use ikaros_core::{IkarosError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProfile {
    pub id: &'static str,
    pub default_max_tokens: Option<u32>,
    pub context: ModelContextProfile,
    pub temperature_policy: TemperaturePolicy,
    pub reasoning_policy: ReasoningPolicy,
    pub message_policy: MessagePolicy,
    pub tool_schema_policy: ToolSchemaPolicy,
    pub request_body_policy: RequestBodyPolicy,
    pub retry_without_parameters: &'static [&'static str],
    pub network_access: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderProfileSpec {
    pub id: &'static str,
    pub auto_base_url_markers: &'static [&'static str],
    pub auto_model_markers: &'static [&'static str],
    pub auto_model_tail_prefixes: &'static [&'static str],
    pub default_max_tokens: Option<u32>,
    pub temperature_policy: TemperaturePolicy,
    pub reasoning_policy: ReasoningPolicy,
    pub message_policy: MessagePolicy,
    pub tool_schema_policy: ToolSchemaPolicy,
    pub request_body_policy: RequestBodyPolicy,
    pub retry_without_parameters: &'static [&'static str],
    pub network_access: bool,
    context_window: ContextWindowPolicy,
    default_output_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextWindowPolicy {
    Fixed(u32),
    InferGeneric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemperaturePolicy {
    PassThrough,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningPolicy {
    None,
    MoonshotKimi,
    DeepSeek,
    GeminiOpenAi,
    OpenRouterAnthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagePolicy {
    Plain,
    QwenTextPartsWithSystemCache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSchemaPolicy {
    PassThrough,
    MoonshotSubset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestBodyPolicy {
    None,
    QwenHighResolutionImages,
}

impl TemperaturePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PassThrough => "pass-through",
            Self::Omit => "omit",
        }
    }
}

impl ReasoningPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::MoonshotKimi => "moonshot-kimi",
            Self::DeepSeek => "deepseek",
            Self::GeminiOpenAi => "gemini-openai",
            Self::OpenRouterAnthropic => "openrouter-anthropic",
        }
    }
}

impl MessagePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::QwenTextPartsWithSystemCache => "qwen-text-parts-system-cache",
        }
    }
}

impl ToolSchemaPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PassThrough => "pass-through",
            Self::MoonshotSubset => "moonshot-subset",
        }
    }
}

impl RequestBodyPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::QwenHighResolutionImages => "qwen-high-resolution-images",
        }
    }
}

const OPENAI_COMPAT_PROFILE_SPECS: &[ProviderProfileSpec] = &[
    ProviderProfileSpec {
        id: "generic",
        auto_base_url_markers: &[],
        auto_model_markers: &[],
        auto_model_tail_prefixes: &[],
        default_max_tokens: None,
        temperature_policy: TemperaturePolicy::PassThrough,
        reasoning_policy: ReasoningPolicy::None,
        message_policy: MessagePolicy::Plain,
        tool_schema_policy: ToolSchemaPolicy::PassThrough,
        request_body_policy: RequestBodyPolicy::None,
        retry_without_parameters: &["temperature", "max_tokens"],
        network_access: true,
        context_window: ContextWindowPolicy::InferGeneric,
        default_output_tokens: 4_096,
    },
    ProviderProfileSpec {
        id: "moonshot-kimi",
        auto_base_url_markers: &["api.moonshot.ai", "api.moonshot.cn", "api.kimi.com"],
        auto_model_markers: &["kimi", "moonshot"],
        auto_model_tail_prefixes: &["kimi-", "kimi_"],
        default_max_tokens: Some(32_000),
        temperature_policy: TemperaturePolicy::Omit,
        reasoning_policy: ReasoningPolicy::MoonshotKimi,
        message_policy: MessagePolicy::Plain,
        tool_schema_policy: ToolSchemaPolicy::MoonshotSubset,
        request_body_policy: RequestBodyPolicy::None,
        retry_without_parameters: &[],
        network_access: true,
        context_window: ContextWindowPolicy::Fixed(128_000),
        default_output_tokens: 32_000,
    },
    ProviderProfileSpec {
        id: "deepseek",
        auto_base_url_markers: &["deepseek"],
        auto_model_markers: &[],
        auto_model_tail_prefixes: &["deepseek-"],
        default_max_tokens: None,
        temperature_policy: TemperaturePolicy::PassThrough,
        reasoning_policy: ReasoningPolicy::DeepSeek,
        message_policy: MessagePolicy::Plain,
        tool_schema_policy: ToolSchemaPolicy::PassThrough,
        request_body_policy: RequestBodyPolicy::None,
        retry_without_parameters: &["temperature", "max_tokens"],
        network_access: true,
        context_window: ContextWindowPolicy::Fixed(128_000),
        default_output_tokens: 8_192,
    },
    ProviderProfileSpec {
        id: "gemini-openai",
        auto_base_url_markers: &["generativelanguage.googleapis.com"],
        auto_model_markers: &[],
        auto_model_tail_prefixes: &["gemini-"],
        default_max_tokens: None,
        temperature_policy: TemperaturePolicy::PassThrough,
        reasoning_policy: ReasoningPolicy::GeminiOpenAi,
        message_policy: MessagePolicy::Plain,
        tool_schema_policy: ToolSchemaPolicy::PassThrough,
        request_body_policy: RequestBodyPolicy::None,
        retry_without_parameters: &["temperature", "max_tokens"],
        network_access: true,
        context_window: ContextWindowPolicy::Fixed(1_048_576),
        default_output_tokens: 8_192,
    },
    ProviderProfileSpec {
        id: "openrouter",
        auto_base_url_markers: &["openrouter.ai"],
        auto_model_markers: &[],
        auto_model_tail_prefixes: &[],
        default_max_tokens: None,
        temperature_policy: TemperaturePolicy::PassThrough,
        reasoning_policy: ReasoningPolicy::OpenRouterAnthropic,
        message_policy: MessagePolicy::Plain,
        tool_schema_policy: ToolSchemaPolicy::PassThrough,
        request_body_policy: RequestBodyPolicy::None,
        retry_without_parameters: &["temperature", "max_tokens"],
        network_access: true,
        context_window: ContextWindowPolicy::Fixed(128_000),
        default_output_tokens: 8_192,
    },
    ProviderProfileSpec {
        id: "qwen",
        auto_base_url_markers: &["dashscope", "portal.qwen.ai"],
        auto_model_markers: &[],
        auto_model_tail_prefixes: &["qwen"],
        default_max_tokens: Some(65_536),
        temperature_policy: TemperaturePolicy::PassThrough,
        reasoning_policy: ReasoningPolicy::None,
        message_policy: MessagePolicy::QwenTextPartsWithSystemCache,
        tool_schema_policy: ToolSchemaPolicy::PassThrough,
        request_body_policy: RequestBodyPolicy::QwenHighResolutionImages,
        retry_without_parameters: &["temperature"],
        network_access: true,
        context_window: ContextWindowPolicy::Fixed(128_000),
        default_output_tokens: 65_536,
    },
    ProviderProfileSpec {
        id: "local-openai-compatible",
        auto_base_url_markers: &["localhost", "127.0.0.1", "[::1]", "0.0.0.0"],
        auto_model_markers: &[],
        auto_model_tail_prefixes: &[],
        default_max_tokens: Some(65_536),
        temperature_policy: TemperaturePolicy::PassThrough,
        reasoning_policy: ReasoningPolicy::None,
        message_policy: MessagePolicy::Plain,
        tool_schema_policy: ToolSchemaPolicy::PassThrough,
        request_body_policy: RequestBodyPolicy::None,
        retry_without_parameters: &["temperature", "max_tokens"],
        network_access: false,
        context_window: ContextWindowPolicy::Fixed(131_072),
        default_output_tokens: 65_536,
    },
];

impl ProviderProfile {
    pub fn catalog() -> &'static [ProviderProfileSpec] {
        OPENAI_COMPAT_PROFILE_SPECS
    }

    pub fn resolve_configured(configured: &str, base_url: &str, model: &str) -> Result<Self> {
        let configured = configured.trim().to_ascii_lowercase();
        if !configured.is_empty() && configured != "auto" {
            return Self::resolve_profile_id(&configured, base_url, model);
        }
        let spec = ProviderProfileSpec::auto(base_url, model);
        Ok(Self::resolve_spec(spec, base_url, model))
    }

    pub fn resolve_profile_id(profile_id: &str, base_url: &str, model: &str) -> Result<Self> {
        let spec = ProviderProfileSpec::resolve_profile_id(profile_id)?;
        Ok(Self::resolve_spec(spec, base_url, model))
    }

    pub fn resolve_spec(spec: &ProviderProfileSpec, base_url: &str, model: &str) -> Self {
        let default_max_tokens = spec.default_max_tokens;
        let context_window = match spec.context_window {
            ContextWindowPolicy::Fixed(tokens) => tokens,
            ContextWindowPolicy::InferGeneric => infer_generic_context_window(model),
        };
        let default_output_tokens = default_max_tokens.unwrap_or(spec.default_output_tokens);
        let id = spec.id;
        let context = ModelContextProfile::new(
            context_window,
            default_output_tokens,
            ModelTokenizerKind::OpenAiCompatible,
            format!("openai-compatible:{id}"),
        );
        let tool_schema_policy = if spec.tool_schema_policy == ToolSchemaPolicy::MoonshotSubset
            || moonshot_kimi_profile_spec().auto_detects(
                &base_url.trim().trim_end_matches('/').to_ascii_lowercase(),
                &model.to_ascii_lowercase(),
            ) {
            ToolSchemaPolicy::MoonshotSubset
        } else {
            spec.tool_schema_policy
        };
        Self {
            id,
            default_max_tokens,
            context,
            temperature_policy: spec.temperature_policy,
            reasoning_policy: spec.reasoning_policy,
            message_policy: spec.message_policy,
            tool_schema_policy,
            request_body_policy: spec.request_body_policy,
            retry_without_parameters: spec.retry_without_parameters,
            network_access: spec.network_access,
        }
    }

    pub fn prepare_messages(&self, messages: &mut serde_json::Value) {
        match self.message_policy {
            MessagePolicy::Plain => {}
            MessagePolicy::QwenTextPartsWithSystemCache => prepare_qwen_messages(messages),
        }
    }

    pub fn prepare_tools(&self, tools: Vec<ModelToolDefinition>) -> Vec<ModelToolDefinition> {
        match self.tool_schema_policy {
            ToolSchemaPolicy::PassThrough => tools,
            ToolSchemaPolicy::MoonshotSubset => sanitize_moonshot_tool_definitions(tools),
        }
    }

    pub fn apply_profile_fields(
        &self,
        body: &mut serde_json::Map<String, serde_json::Value>,
        model: &str,
        base_url: &str,
        options: &ModelRequestOptions,
    ) {
        match self.reasoning_policy {
            ReasoningPolicy::None => {}
            ReasoningPolicy::MoonshotKimi => apply_kimi_fields(body, options),
            ReasoningPolicy::DeepSeek => apply_deepseek_fields(body, model, options),
            ReasoningPolicy::GeminiOpenAi => {
                apply_gemini_openai_fields(body, model, base_url, options)
            }
            ReasoningPolicy::OpenRouterAnthropic => apply_openrouter_fields(body, model, options),
        }
        match self.request_body_policy {
            RequestBodyPolicy::None => {}
            RequestBodyPolicy::QwenHighResolutionImages => apply_qwen_fields(body),
        }
    }

    pub fn can_retry_without_parameter(&self, parameter: &str) -> bool {
        self.retry_without_parameters.contains(&parameter)
    }
}

impl ProviderProfileSpec {
    pub fn resolve_profile_id(profile_id: &str) -> Result<&'static Self> {
        let profile_id = profile_id.trim().to_ascii_lowercase();
        ProviderProfile::catalog()
            .iter()
            .find(|spec| spec.id == profile_id)
            .ok_or_else(|| {
                IkarosError::Message(format!(
                    "unsupported OpenAI-compatible profile `{profile_id}`"
                ))
            })
    }

    pub fn auto(base_url: &str, model: &str) -> &'static Self {
        let base = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
        let model_lower = model.trim().to_ascii_lowercase();
        ProviderProfile::catalog()
            .iter()
            .find(|spec| spec.id != "generic" && spec.auto_detects(&base, &model_lower))
            .unwrap_or_else(generic_profile_spec)
    }

    fn auto_detects(&self, base_url: &str, model: &str) -> bool {
        let tail = model_tail(model);
        self.auto_base_url_markers
            .iter()
            .any(|marker| base_url.contains(marker))
            || self
                .auto_model_markers
                .iter()
                .any(|marker| model.contains(marker))
            || self
                .auto_model_tail_prefixes
                .iter()
                .any(|prefix| tail.starts_with(prefix))
    }
}

fn generic_profile_spec() -> &'static ProviderProfileSpec {
    ProviderProfileSpec::resolve_profile_id("generic")
        .expect("generic OpenAI-compatible profile spec must exist")
}

fn moonshot_kimi_profile_spec() -> &'static ProviderProfileSpec {
    ProviderProfileSpec::resolve_profile_id("moonshot-kimi")
        .expect("moonshot-kimi OpenAI-compatible profile spec must exist")
}

fn apply_kimi_fields(
    body: &mut serde_json::Map<String, serde_json::Value>,
    options: &ModelRequestOptions,
) {
    if options.reasoning.enabled == Some(false) {
        body.insert("thinking".into(), serde_json::json!({"type": "disabled"}));
        body.remove("reasoning_effort");
        return;
    }

    match options.reasoning.effort {
        Some(ReasoningEffort::Low | ReasoningEffort::Medium | ReasoningEffort::High) => {
            body.insert(
                "reasoning_effort".into(),
                serde_json::Value::String(
                    options
                        .reasoning
                        .effort
                        .expect("checked above")
                        .as_wire_value()
                        .into(),
                ),
            );
            body.remove("thinking");
        }
        _ => {
            body.insert("thinking".into(), serde_json::json!({"type": "enabled"}));
            body.remove("reasoning_effort");
        }
    }
}

fn infer_generic_context_window(model: &str) -> u32 {
    let model = model.trim().to_ascii_lowercase();
    if model.contains("128k") || model.contains("128-k") {
        return 128_000;
    }
    if model.contains("64k") || model.contains("64-k") {
        return 64_000;
    }
    if model.contains("32k") || model.contains("32-k") {
        return 32_000;
    }
    if model.contains("16k") || model.contains("16-k") {
        return 16_000;
    }
    if model.contains("8k") || model.contains("8-k") {
        return 8_000;
    }
    128_000
}

fn apply_deepseek_fields(
    body: &mut serde_json::Map<String, serde_json::Value>,
    model: &str,
    options: &ModelRequestOptions,
) {
    if !deepseek_supports_thinking(model) {
        return;
    }
    let enabled = options.reasoning.enabled != Some(false);
    body.insert(
        "thinking".into(),
        serde_json::json!({"type": if enabled { "enabled" } else { "disabled" }}),
    );
    if !enabled {
        body.remove("reasoning_effort");
        return;
    }
    let effort = match options.reasoning.effort {
        Some(ReasoningEffort::XHigh | ReasoningEffort::Max) => Some("max"),
        Some(ReasoningEffort::Low) => Some("low"),
        Some(ReasoningEffort::Medium) => Some("medium"),
        Some(ReasoningEffort::High) => Some("high"),
        _ => None,
    };
    if let Some(effort) = effort {
        body.insert(
            "reasoning_effort".into(),
            serde_json::Value::String(effort.into()),
        );
    }
}

fn apply_gemini_openai_fields(
    body: &mut serde_json::Map<String, serde_json::Value>,
    model: &str,
    base_url: &str,
    options: &ModelRequestOptions,
) {
    let Some(thinking_config) = gemini_thinking_config(model, options) else {
        return;
    };
    if base_url
        .trim()
        .trim_end_matches('/')
        .to_ascii_lowercase()
        .ends_with("/openai")
    {
        merge_gemini_openai_thinking_config(body, thinking_config);
    } else {
        body.insert("thinking_config".into(), thinking_config);
    }
}

fn merge_gemini_openai_thinking_config(
    body: &mut serde_json::Map<String, serde_json::Value>,
    thinking_config: serde_json::Value,
) {
    let extra_body = body
        .entry("extra_body")
        .or_insert_with(|| serde_json::json!({}));
    if !extra_body.is_object() {
        *extra_body = serde_json::json!({});
    }
    let extra_body = extra_body
        .as_object_mut()
        .expect("extra_body was normalized to object");
    let google = extra_body
        .entry("google")
        .or_insert_with(|| serde_json::json!({}));
    if !google.is_object() {
        *google = serde_json::json!({});
    }
    google
        .as_object_mut()
        .expect("google was normalized to object")
        .insert("thinking_config".into(), thinking_config);
}

fn apply_openrouter_fields(
    body: &mut serde_json::Map<String, serde_json::Value>,
    model: &str,
    options: &ModelRequestOptions,
) {
    if !openrouter_anthropic_reasoning_is_mandatory(model) {
        return;
    }
    body.remove("reasoning");
    if options.reasoning.enabled == Some(false) {
        return;
    }
    if let Some(effort) = options.reasoning.effort {
        if !matches!(effort, ReasoningEffort::None) {
            body.insert(
                "verbosity".into(),
                serde_json::Value::String(effort.as_wire_value().into()),
            );
        }
    }
}

fn apply_qwen_fields(body: &mut serde_json::Map<String, serde_json::Value>) {
    body.insert(
        "vl_high_resolution_images".into(),
        serde_json::Value::Bool(true),
    );
}

fn prepare_qwen_messages(messages: &mut serde_json::Value) {
    let serde_json::Value::Array(items) = messages else {
        return;
    };
    let mut system_cache_control_added = false;
    for message in items {
        let serde_json::Value::Object(message) = message else {
            continue;
        };
        normalize_qwen_content(message);
        if system_cache_control_added {
            continue;
        }
        if message.get("role").and_then(|role| role.as_str()) == Some("system") {
            if let Some(content) = message.get_mut("content") {
                add_qwen_system_cache_control(content);
                system_cache_control_added = true;
            }
        }
    }
}

fn normalize_qwen_content(message: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(content) = message.get_mut("content") else {
        return;
    };
    match content {
        serde_json::Value::String(text) => {
            let text = std::mem::take(text);
            *content = serde_json::Value::Array(vec![serde_json::json!({
                "type": "text",
                "text": text,
            })]);
        }
        serde_json::Value::Array(parts) => {
            for part in parts {
                if let serde_json::Value::String(text) = part {
                    let text = std::mem::take(text);
                    *part = serde_json::json!({
                        "type": "text",
                        "text": text,
                    });
                }
            }
        }
        _ => {}
    }
}

fn add_qwen_system_cache_control(content: &mut serde_json::Value) {
    let serde_json::Value::Array(parts) = content else {
        return;
    };
    if let Some(serde_json::Value::Object(last)) = parts.last_mut() {
        last.insert(
            "cache_control".into(),
            serde_json::json!({"type": "ephemeral"}),
        );
    }
}

fn gemini_thinking_config(model: &str, options: &ModelRequestOptions) -> Option<serde_json::Value> {
    let model_lower = model.to_ascii_lowercase();
    let normalized = model_tail(&model_lower);
    if !normalized.starts_with("gemini-") {
        return None;
    }
    if options.reasoning.enabled == Some(false)
        || matches!(options.reasoning.effort, Some(ReasoningEffort::None))
    {
        return Some(serde_json::json!({"include_thoughts": false}));
    }
    let mut config =
        serde_json::Map::from_iter([("include_thoughts".into(), serde_json::Value::Bool(true))]);
    let effort = options.reasoning.effort.unwrap_or(ReasoningEffort::Medium);
    if normalized.starts_with("gemini-3") {
        let level = if normalized.contains("flash") {
            match effort {
                ReasoningEffort::Minimal | ReasoningEffort::Low => Some("low"),
                ReasoningEffort::High | ReasoningEffort::XHigh | ReasoningEffort::Max => {
                    Some("high")
                }
                ReasoningEffort::Medium => Some("medium"),
                ReasoningEffort::None => None,
            }
        } else if normalized.contains("pro") {
            Some(
                if matches!(
                    effort,
                    ReasoningEffort::High | ReasoningEffort::XHigh | ReasoningEffort::Max
                ) {
                    "high"
                } else {
                    "low"
                },
            )
        } else {
            None
        };
        if let Some(level) = level {
            config.insert(
                "thinking_level".into(),
                serde_json::Value::String(level.into()),
            );
        }
    }
    Some(serde_json::Value::Object(config))
}

fn deepseek_supports_thinking(model: &str) -> bool {
    let model_lower = model.to_ascii_lowercase();
    let tail = model_tail(&model_lower);
    (tail.starts_with("deepseek-v") && !tail.starts_with("deepseek-v3"))
        || tail == "deepseek-reasoner"
}

fn openrouter_anthropic_reasoning_is_mandatory(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    if !(model.starts_with("anthropic/") || model.starts_with("claude") || model.contains("claude"))
    {
        return false;
    }
    ![
        "claude-3",
        "claude-opus-4-0",
        "claude-opus-4.0",
        "claude-opus-4-1",
        "claude-opus-4.1",
        "claude-sonnet-4-0",
        "claude-sonnet-4.0",
        "claude-opus-4-2025",
        "claude-sonnet-4-2025",
        "claude-opus-4-5",
        "claude-opus-4.5",
        "claude-sonnet-4-5",
        "claude-sonnet-4.5",
        "claude-haiku-4-5",
        "claude-haiku-4.5",
    ]
    .iter()
    .any(|needle| model.contains(needle))
}

fn model_tail(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}
