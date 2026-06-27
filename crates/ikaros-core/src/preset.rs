// SPDX-License-Identifier: GPL-3.0-only
//! Static lookup table for model provider presets.

use crate::{IkarosError, Result};

/// Static description of a single model/provider preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelPresetSpec {
    pub id: &'static str,
    pub provider: &'static str,
    pub transport: &'static str,
    pub compat_profile: &'static str,
    pub recommended_base_url: Option<&'static str>,
}

pub const PRESETS: &[ModelPresetSpec] = &[
    ModelPresetSpec {
        id: "auto",
        provider: "openai-compatible",
        transport: "openai-compatible-chat-completions",
        compat_profile: "auto",
        recommended_base_url: None,
    },
    ModelPresetSpec {
        id: "openai",
        provider: "openai-compatible",
        transport: "openai-compatible-chat-completions",
        compat_profile: "generic",
        recommended_base_url: Some("https://api.openai.com/v1"),
    },
    ModelPresetSpec {
        id: "kimi",
        provider: "openai-compatible",
        transport: "openai-compatible-chat-completions",
        compat_profile: "moonshot-kimi",
        recommended_base_url: Some("https://api.moonshot.cn/v1"),
    },
    ModelPresetSpec {
        id: "deepseek",
        provider: "openai-compatible",
        transport: "openai-compatible-chat-completions",
        compat_profile: "deepseek",
        recommended_base_url: Some("https://api.deepseek.com"),
    },
    ModelPresetSpec {
        id: "gemini",
        provider: "openai-compatible",
        transport: "openai-compatible-chat-completions",
        compat_profile: "gemini-openai",
        recommended_base_url: Some("https://generativelanguage.googleapis.com/v1beta/openai"),
    },
    ModelPresetSpec {
        id: "openrouter",
        provider: "openai-compatible",
        transport: "openai-compatible-chat-completions",
        compat_profile: "openrouter",
        recommended_base_url: Some("https://openrouter.ai/api/v1"),
    },
    ModelPresetSpec {
        id: "qwen",
        provider: "openai-compatible",
        transport: "openai-compatible-chat-completions",
        compat_profile: "qwen",
        recommended_base_url: Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
    },
    ModelPresetSpec {
        id: "local-openai",
        provider: "openai-compatible",
        transport: "openai-compatible-chat-completions",
        compat_profile: "local-openai-compatible",
        recommended_base_url: Some("http://127.0.0.1:8080/v1"),
    },
    ModelPresetSpec {
        id: "ollama",
        provider: "ollama",
        transport: "ollama-chat",
        compat_profile: "ollama-native",
        recommended_base_url: Some("http://127.0.0.1:11434"),
    },
    ModelPresetSpec {
        id: "anthropic",
        provider: "anthropic",
        transport: "anthropic-messages",
        compat_profile: "anthropic-native",
        recommended_base_url: Some("https://api.anthropic.com"),
    },
    ModelPresetSpec {
        id: "mock",
        provider: "mock",
        transport: "mock",
        compat_profile: "mock",
        recommended_base_url: None,
    },
];

pub fn preset_catalog() -> &'static [ModelPresetSpec] {
    PRESETS
}

pub fn resolve_preset(id: &str) -> Result<&'static ModelPresetSpec> {
    PRESETS
        .iter()
        .find(|preset| preset.id == id)
        .ok_or_else(|| {
            let available = preset_ids().join(", ");
            IkarosError::Message(format!(
                "unknown model preset: {id}; available presets: {available}"
            ))
        })
}

pub fn preset_ids() -> Vec<&'static str> {
    PRESETS.iter().map(|preset| preset.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_core_providers() {
        let ids: Vec<&str> = preset_catalog().iter().map(|spec| spec.id).collect();
        for required in ["auto", "openai", "kimi", "ollama", "anthropic", "mock"] {
            assert!(
                ids.contains(&required),
                "preset catalog missing required id: {required}"
            );
        }
    }

    #[test]
    fn resolves_kimi_preset() {
        let spec = resolve_preset("kimi").expect("kimi preset should resolve");
        assert_eq!(spec.id, "kimi");
        assert_eq!(spec.provider, "openai-compatible");
        assert_eq!(spec.transport, "openai-compatible-chat-completions");
        assert_eq!(spec.compat_profile, "moonshot-kimi");
        assert_eq!(
            spec.recommended_base_url,
            Some("https://api.moonshot.cn/v1")
        );
    }

    #[test]
    fn rejects_unknown_preset() {
        let err = resolve_preset("does-not-exist").expect_err("unknown preset should error");
        let message = err.to_string();
        assert!(
            message.contains("does-not-exist"),
            "error should name the unknown id: {message}"
        );
        for available in ["auto", "openai", "kimi", "ollama", "anthropic", "mock"] {
            assert!(
                message.contains(available),
                "error should list available preset {available}: {message}"
            );
        }
    }
}
