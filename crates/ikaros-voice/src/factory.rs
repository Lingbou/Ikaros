// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AsrProvider, MockAsrProvider, MockTtsProvider, OpenAiCompatibleVoiceProvider, TtsProvider,
    VoiceProviderConfig,
};
use ikaros_core::{IkarosError, RemoteProviderConfig, Result};

pub fn tts_provider_from_config(
    config: &VoiceProviderConfig,
    provider_settings: &RemoteProviderConfig,
) -> Result<Box<dyn TtsProvider>> {
    match config.provider.to_ascii_lowercase().as_str() {
        "mock" => Ok(Box::new(MockTtsProvider)),
        "openai-compatible" => Ok(Box::new(OpenAiCompatibleVoiceProvider::from_config(
            config.provider.to_string(),
            config,
            provider_settings,
        )?)),
        other => Err(IkarosError::Message(format!(
            "unsupported TTS provider: {other}"
        ))),
    }
}

pub fn asr_provider_from_config(
    config: &VoiceProviderConfig,
    provider_settings: &RemoteProviderConfig,
) -> Result<Box<dyn AsrProvider>> {
    match config.provider.to_ascii_lowercase().as_str() {
        "mock" => Ok(Box::new(MockAsrProvider)),
        "openai-compatible" => Ok(Box::new(OpenAiCompatibleVoiceProvider::from_config(
            config.provider.to_string(),
            config,
            provider_settings,
        )?)),
        other => Err(IkarosError::Message(format!(
            "unsupported ASR provider: {other}"
        ))),
    }
}
