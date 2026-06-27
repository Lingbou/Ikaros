// SPDX-License-Identifier: GPL-3.0-only
//! Voice provider abstractions for TTS and ASR.

mod factory;
mod mock;
mod openai_compatible;
mod types;

pub use factory::{asr_provider_from_config, tts_provider_from_config};
pub use ikaros_core::VoiceProviderConfig;
pub use mock::{MockAsrProvider, MockTtsProvider};
pub use openai_compatible::{
    OpenAiCompatibleVoiceProvider, ReqwestVoiceHttpClient, VoiceHttpBody, VoiceHttpClient,
    VoiceHttpRequest, VoiceHttpResponse,
};
pub use types::{
    AsrProvider, AsrRequest, AudioFormat, AudioOutput, Transcript, TtsProvider, TtsRequest,
};

#[cfg(test)]
mod tests;
