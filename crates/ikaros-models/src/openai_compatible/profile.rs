// SPDX-License-Identifier: GPL-3.0-only

use crate::types::{ModelRequestOptions, ReasoningEffort};
use ikaros_core::{IkarosError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiCompatProfile {
    Generic,
    MoonshotKimi,
    DeepSeek,
    GeminiOpenAi,
    OpenRouter,
    Qwen,
    LocalOpenAiCompatible,
}

impl OpenAiCompatProfile {
    pub fn resolve(configured: &str, base_url: &str, model: &str) -> Result<Self> {
        let configured = configured.trim().to_ascii_lowercase();
        if configured.is_empty() || configured == "auto" {
            return Ok(Self::auto(base_url, model));
        }
        match configured.as_str() {
            "generic" => Ok(Self::Generic),
            "moonshot-kimi" => Ok(Self::MoonshotKimi),
            "deepseek" => Ok(Self::DeepSeek),
            "gemini-openai" => Ok(Self::GeminiOpenAi),
            "openrouter" => Ok(Self::OpenRouter),
            "qwen" => Ok(Self::Qwen),
            "local-openai-compatible" => Ok(Self::LocalOpenAiCompatible),
            other => Err(IkarosError::Message(format!(
                "unsupported OpenAI-compatible profile `{other}`"
            ))),
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::MoonshotKimi => "moonshot-kimi",
            Self::DeepSeek => "deepseek",
            Self::GeminiOpenAi => "gemini-openai",
            Self::OpenRouter => "openrouter",
            Self::Qwen => "qwen",
            Self::LocalOpenAiCompatible => "local-openai-compatible",
        }
    }

    pub fn default_max_tokens(self, model: &str) -> Option<u32> {
        match self {
            Self::MoonshotKimi => Some(32_000),
            Self::Qwen => Some(65_536),
            Self::LocalOpenAiCompatible => Some(65_536),
            Self::DeepSeek if deepseek_supports_thinking(model) => None,
            _ => None,
        }
    }

    pub fn omits_temperature(self) -> bool {
        matches!(self, Self::MoonshotKimi)
    }

    pub fn auto(base_url: &str, model: &str) -> Self {
        let base = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
        let model_lower = model.trim().to_ascii_lowercase();
        if is_moonshot_base_url(&base) || is_moonshot_model(&model_lower) {
            return Self::MoonshotKimi;
        }
        if base.contains("deepseek") || model_tail(&model_lower).starts_with("deepseek-") {
            return Self::DeepSeek;
        }
        if base.contains("generativelanguage.googleapis.com") && base.ends_with("/openai")
            || model_tail(&model_lower).starts_with("gemini-")
        {
            return Self::GeminiOpenAi;
        }
        if base.contains("openrouter.ai") {
            return Self::OpenRouter;
        }
        if base.contains("dashscope") || base.contains("portal.qwen.ai") {
            return Self::Qwen;
        }
        if is_local_base_url(&base) {
            return Self::LocalOpenAiCompatible;
        }
        Self::Generic
    }

    pub fn apply_profile_fields(
        self,
        body: &mut serde_json::Map<String, serde_json::Value>,
        model: &str,
        base_url: &str,
        options: &ModelRequestOptions,
    ) {
        match self {
            Self::MoonshotKimi => apply_kimi_fields(body, options),
            Self::DeepSeek => apply_deepseek_fields(body, model, options),
            Self::GeminiOpenAi => apply_gemini_openai_fields(body, model, base_url, options),
            Self::OpenRouter => apply_openrouter_fields(body, model, options),
            Self::Qwen => apply_qwen_fields(body),
            _ => {}
        }
    }

    pub fn prepare_messages(self, messages: &mut serde_json::Value) {
        if self == Self::Qwen {
            prepare_qwen_messages(messages);
        }
    }

    pub fn can_retry_without_parameter(self, parameter: &str) -> bool {
        match parameter {
            "temperature" => !self.omits_temperature(),
            "max_tokens" => !matches!(self, Self::MoonshotKimi | Self::Qwen),
            _ => false,
        }
    }
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

pub fn is_moonshot_model(model: &str) -> bool {
    let bare = model.trim().to_ascii_lowercase();
    let tail = model_tail(&bare);
    tail.starts_with("kimi-")
        || tail == "kimi"
        || bare.contains("moonshot")
        || bare.contains("/kimi")
        || bare.starts_with("kimi")
}

fn is_moonshot_base_url(base_url: &str) -> bool {
    base_url.contains("api.moonshot.ai")
        || base_url.contains("api.moonshot.cn")
        || base_url.contains("api.kimi.com")
}

fn deepseek_supports_thinking(model: &str) -> bool {
    let model_lower = model.to_ascii_lowercase();
    let tail = model_tail(&model_lower);
    (tail.starts_with("deepseek-v") && !tail.starts_with("deepseek-v3"))
        || tail == "deepseek-reasoner"
}

fn openrouter_anthropic_reasoning_is_mandatory(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    if !(model.starts_with("anthropic/") || model.starts_with("claude"))
        && !model.contains("claude")
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

fn is_local_base_url(base_url: &str) -> bool {
    base_url.contains("localhost")
        || base_url.contains("127.0.0.1")
        || base_url.contains("0.0.0.0")
        || base_url.contains("::1")
}

fn model_tail(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}
