// SPDX-License-Identifier: GPL-3.0-only

use crate::{AsrProvider, AsrRequest, AudioOutput, Transcript, TtsProvider, TtsRequest};
use async_trait::async_trait;
use ikaros_core::{Result, redact_secrets};

#[derive(Debug, Clone, Default)]
pub struct MockTtsProvider;

#[async_trait]
impl TtsProvider for MockTtsProvider {
    fn name(&self) -> &str {
        "mock-tts"
    }

    async fn synthesize(&self, request: TtsRequest) -> Result<AudioOutput> {
        let redacted_text_preview = redact_secrets(&request.text);
        Ok(AudioOutput {
            path: None,
            format: request.format.clone(),
            bytes: mock_audio_bytes(&request, &redacted_text_preview),
            redacted_text_preview,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct MockAsrProvider;

#[async_trait]
impl AsrProvider for MockAsrProvider {
    fn name(&self) -> &str {
        "mock-asr"
    }

    async fn transcribe(&self, request: AsrRequest) -> Result<Transcript> {
        Ok(Transcript {
            text: "mock transcript".into(),
            language: request.language,
            confidence: Some(100),
        })
    }
}

fn mock_audio_bytes(request: &TtsRequest, redacted_text: &str) -> Vec<u8> {
    format!(
        "IKAROS_MOCK_TTS\nformat={}\nsample_rate_hz={}\nlanguage={}\nvoice={}\ntext={}\n",
        request.format.as_str(),
        request
            .sample_rate_hz
            .map(|value| value.to_string())
            .unwrap_or_else(|| "default".into()),
        request.language.as_deref().unwrap_or("default"),
        request.voice.as_deref().unwrap_or("default"),
        redacted_text,
    )
    .into_bytes()
}
