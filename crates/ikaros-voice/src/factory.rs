// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AsrProvider, MockAsrProvider, MockTtsProvider, OpenAiCompatibleVoiceProvider, TtsProvider,
    VoiceProviderConfig,
};
use ikaros_core::{IkarosError, Result};

pub fn tts_provider_from_config(config: &VoiceProviderConfig) -> Result<Box<dyn TtsProvider>> {
    match config.provider.to_ascii_lowercase().as_str() {
        "mock" | "mock-tts" => Ok(Box::new(MockTtsProvider)),
        "openai" | "openai-compatible" | "moonshot" | "siliconflow" => Ok(Box::new(
            OpenAiCompatibleVoiceProvider::from_config(config.provider.clone(), config)?,
        )),
        other => Err(IkarosError::Message(format!(
            "unsupported TTS provider: {other}"
        ))),
    }
}

pub fn asr_provider_from_config(config: &VoiceProviderConfig) -> Result<Box<dyn AsrProvider>> {
    match config.provider.to_ascii_lowercase().as_str() {
        "mock" | "mock-asr" => Ok(Box::new(MockAsrProvider)),
        "openai" | "openai-compatible" | "moonshot" | "siliconflow" => Ok(Box::new(
            OpenAiCompatibleVoiceProvider::from_config(config.provider.clone(), config)?,
        )),
        other => Err(IkarosError::Message(format!(
            "unsupported ASR provider: {other}"
        ))),
    }
}
