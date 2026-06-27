// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

use super::VoiceProviderKind;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VoiceConfig {
    #[serde(default = "VoiceProviderConfig::mock_tts")]
    pub tts: VoiceProviderConfig,
    #[serde(default = "VoiceProviderConfig::mock_asr")]
    pub asr: VoiceProviderConfig,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            tts: VoiceProviderConfig::mock_tts(),
            asr: VoiceProviderConfig::mock_asr(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VoiceProviderConfig {
    pub provider: VoiceProviderKind,
    pub model: String,
    pub timeout_ms: u64,
    pub max_retries: u8,
    pub voice: Option<String>,
}

impl VoiceProviderConfig {
    pub fn remote_tts() -> Self {
        Self {
            provider: VoiceProviderKind::OpenaiCompatible,
            model: String::new(),
            timeout_ms: 30_000,
            max_retries: 0,
            voice: Some("default".into()),
        }
    }

    pub fn remote_asr() -> Self {
        Self {
            provider: VoiceProviderKind::OpenaiCompatible,
            model: String::new(),
            timeout_ms: 30_000,
            max_retries: 0,
            voice: None,
        }
    }

    pub fn mock_tts() -> Self {
        Self {
            provider: VoiceProviderKind::Mock,
            model: "mock-tts".into(),
            timeout_ms: 30_000,
            max_retries: 0,
            voice: Some("default".into()),
        }
    }

    pub fn mock_asr() -> Self {
        Self {
            provider: VoiceProviderKind::Mock,
            model: "mock-asr".into(),
            timeout_ms: 30_000,
            max_retries: 0,
            voice: None,
        }
    }
}

impl Default for VoiceProviderConfig {
    fn default() -> Self {
        Self::mock_asr()
    }
}
