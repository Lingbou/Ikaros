// SPDX-License-Identifier: GPL-3.0-only

use crate::{AsrProvider, AsrRequest, AudioOutput, Transcript, TtsProvider, TtsRequest};
use async_trait::async_trait;
use ikaros_core::{
    IkarosError, Result, VoiceProviderConfig, redact_secrets, resolve_config_secret,
    resolve_config_value,
};
use reqwest::{
    Client,
    header::{ACCEPT, ACCEPT_ENCODING},
    multipart,
};
use serde::{Deserialize, Serialize};
use std::{fs, time::Duration};

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleVoiceProvider {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    default_voice: Option<String>,
    max_retries: u8,
    client: Client,
}

impl OpenAiCompatibleVoiceProvider {
    pub fn from_config(
        provider_name: impl Into<String>,
        config: &VoiceProviderConfig,
    ) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|source| {
                IkarosError::Message(format!("failed to build voice client: {source}"))
            })?;
        Ok(Self {
            name: provider_name.into(),
            base_url: resolve_config_value(&config.base_url, "providers.tts/asr.base_url")?
                .trim_end_matches('/')
                .into(),
            model: resolve_config_value(&config.model, "voice.tts/asr.model")?,
            api_key: config.api_key.clone(),
            default_voice: config.voice.clone(),
            max_retries: config.max_retries,
            client,
        })
    }

    fn api_key(&self) -> Result<String> {
        resolve_config_secret(&self.api_key, "providers.tts/asr.api_key")
    }
}

#[async_trait]
impl TtsProvider for OpenAiCompatibleVoiceProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn synthesize(&self, request: TtsRequest) -> Result<AudioOutput> {
        let key = self.api_key()?;
        let redacted_text = redact_secrets(&request.text);
        let body = TtsSpeechRequest {
            model: self.model.clone(),
            input: redacted_text.clone(),
            voice: request
                .voice
                .clone()
                .or_else(|| self.default_voice.clone())
                .unwrap_or_else(|| "alloy".into()),
            response_format: Some(request.format.as_str().into()),
        };
        let url = format!("{}/audio/speech", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result = self
                .client
                .post(&url)
                .header(ACCEPT, "audio/*")
                .header(ACCEPT_ENCODING, "identity")
                .bearer_auth(&key)
                .json(&body)
                .send()
                .await;
            match result {
                Ok(response) => {
                    let status = response.status();
                    if !status.is_success() {
                        let text = response.text().await.unwrap_or_default();
                        last_error = Some(format!(
                            "voice TTS provider returned HTTP {status}: {}",
                            redact_secrets(&text)
                        ));
                        continue;
                    }
                    let bytes = response.bytes().await.map_err(|source| {
                        IkarosError::Message(format!("failed to read TTS response: {source}"))
                    })?;
                    return Ok(AudioOutput {
                        path: None,
                        format: request.format.clone(),
                        bytes: bytes.to_vec(),
                        redacted_text_preview: redacted_text,
                    });
                }
                Err(source) => {
                    last_error = Some(format!(
                        "voice TTS request failed on attempt {attempt}: {source}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(
            last_error.unwrap_or_else(|| "voice TTS request failed".into()),
        ))
    }
}

#[async_trait]
impl AsrProvider for OpenAiCompatibleVoiceProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn transcribe(&self, request: AsrRequest) -> Result<Transcript> {
        let key = self.api_key()?;
        let bytes = fs::read(&request.audio_path)
            .map_err(|source| IkarosError::io(&request.audio_path, source))?;
        let url = format!("{}/audio/transcriptions", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let file_name = request
                .format
                .as_ref()
                .map(|format| format!("audio.{}", format.as_str()))
                .unwrap_or_else(|| "audio".into());
            let mut form = multipart::Form::new()
                .text("model", self.model.clone())
                .part(
                    "file",
                    multipart::Part::bytes(bytes.clone()).file_name(file_name),
                );
            if let Some(language) = &request.language {
                form = form.text("language", language.clone());
            }
            let result = self
                .client
                .post(&url)
                .bearer_auth(&key)
                .multipart(form)
                .send()
                .await;
            match result {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.map_err(|source| {
                        IkarosError::Message(format!("failed to read ASR response: {source}"))
                    })?;
                    if !status.is_success() {
                        last_error = Some(format!(
                            "voice ASR provider returned HTTP {status}: {}",
                            redact_secrets(&text)
                        ));
                        continue;
                    }
                    let parsed: TranscriptionResponse =
                        serde_json::from_str(&text).map_err(|source| {
                            IkarosError::Message(format!(
                                "failed to parse ASR response JSON: {source}"
                            ))
                        })?;
                    return Ok(Transcript {
                        text: redact_secrets(&parsed.text),
                        language: parsed.language.or(request.language.clone()),
                        confidence: None,
                    });
                }
                Err(source) => {
                    last_error = Some(format!(
                        "voice ASR request failed on attempt {attempt}: {source}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(
            last_error.unwrap_or_else(|| "voice ASR request failed".into()),
        ))
    }
}

#[derive(Debug, Clone, Serialize)]
struct TtsSpeechRequest {
    model: String,
    input: String,
    voice: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TranscriptionResponse {
    text: String,
    language: Option<String>,
}

#[cfg(test)]
pub(crate) fn test_tts_speech_body(
    config: &VoiceProviderConfig,
    request: &TtsRequest,
) -> serde_json::Value {
    let body = TtsSpeechRequest {
        model: config.model.clone(),
        input: redact_secrets(&request.text),
        voice: request
            .voice
            .clone()
            .or_else(|| config.voice.clone())
            .unwrap_or_else(|| "alloy".into()),
        response_format: Some(request.format.as_str().into()),
    };
    serde_json::to_value(body).expect("serialize test TTS body")
}
