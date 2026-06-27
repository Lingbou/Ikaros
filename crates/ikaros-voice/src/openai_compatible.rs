// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AsrProvider, AsrRequest, AudioFormat, AudioOutput, Transcript, TtsProvider, TtsRequest,
};
use async_trait::async_trait;
use ikaros_core::{
    IkarosError, RemoteProviderConfig, Result, VoiceProviderConfig, redact_secrets,
    resolve_config_secret, resolve_config_value,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

#[derive(Clone)]
pub struct OpenAiCompatibleVoiceProvider {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    default_voice: Option<String>,
    max_retries: u8,
    http: Arc<dyn VoiceHttpClient>,
}

impl std::fmt::Debug for OpenAiCompatibleVoiceProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiCompatibleVoiceProvider")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &"<redacted>")
            .field("default_voice", &self.default_voice)
            .field("max_retries", &self.max_retries)
            .field("http", &"<voice-http-client>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: VoiceHttpBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceHttpBody {
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait VoiceHttpClient: Send + Sync {
    async fn send(&self, request: VoiceHttpRequest) -> Result<VoiceHttpResponse>;
}

#[derive(Clone)]
pub struct ReqwestVoiceHttpClient {
    client: Client,
}

impl ReqwestVoiceHttpClient {
    pub fn new(timeout: Duration) -> Result<Self> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|source| {
                IkarosError::Message(format!("failed to build voice client: {source}"))
            })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl VoiceHttpClient for ReqwestVoiceHttpClient {
    async fn send(&self, request: VoiceHttpRequest) -> Result<VoiceHttpResponse> {
        let method = request
            .method
            .parse::<reqwest::Method>()
            .map_err(|source| {
                IkarosError::Message(format!("invalid voice HTTP method: {source}"))
            })?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        builder = match request.body {
            VoiceHttpBody::Text(body) => builder.body(body),
            VoiceHttpBody::Bytes(body) => builder.body(body),
        };
        let response = builder
            .send()
            .await
            .map_err(|source| IkarosError::Message(format!("voice request failed: {source}")))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
            })
            .collect();
        let body = response.bytes().await.map_err(|source| {
            IkarosError::Message(format!("failed to read voice response: {source}"))
        })?;
        Ok(VoiceHttpResponse {
            status,
            headers,
            body: body.to_vec(),
        })
    }
}

impl OpenAiCompatibleVoiceProvider {
    pub fn from_config(
        provider_name: impl Into<String>,
        config: &VoiceProviderConfig,
        provider_settings: &RemoteProviderConfig,
    ) -> Result<Self> {
        Self::from_config_with_http_client(
            provider_name,
            config,
            provider_settings,
            Arc::new(ReqwestVoiceHttpClient::new(Duration::from_millis(
                config.timeout_ms,
            ))?),
        )
    }

    pub fn from_config_with_http_client(
        provider_name: impl Into<String>,
        config: &VoiceProviderConfig,
        provider_settings: &RemoteProviderConfig,
        http: Arc<dyn VoiceHttpClient>,
    ) -> Result<Self> {
        Ok(Self {
            name: provider_name.into(),
            base_url: resolve_config_value(
                &provider_settings.base_url,
                "providers.tts/asr.base_url",
            )?
            .trim_end_matches('/')
            .into(),
            model: resolve_config_value(&config.model, "voice.tts/asr.model")?,
            api_key: provider_settings.api_key.clone(),
            default_voice: config.voice.clone(),
            max_retries: config.max_retries,
            http,
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
        let body = serde_json::to_string(&body).map_err(|source| {
            IkarosError::Message(format!("failed to serialize TTS request JSON: {source}"))
        })?;
        let headers = voice_json_headers(&key);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result = self
                .http
                .send(VoiceHttpRequest {
                    method: "POST".into(),
                    url: url.clone(),
                    headers: headers.clone(),
                    body: VoiceHttpBody::Text(body.clone()),
                })
                .await;
            match result {
                Ok(response) => {
                    let status = response.status;
                    if !(200..=299).contains(&status) {
                        let text = String::from_utf8_lossy(&response.body);
                        last_error = Some(format!(
                            "voice TTS provider returned HTTP {status}: {}",
                            redact_secrets(&text)
                        ));
                        continue;
                    }
                    return Ok(AudioOutput {
                        path: None,
                        format: request.format.clone(),
                        bytes: response.body,
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
        let url = format!("{}/audio/transcriptions", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let file_name = request.file_name.clone().unwrap_or_else(|| {
                request
                    .format
                    .as_ref()
                    .map(|format| format!("audio.{}", format.as_str()))
                    .unwrap_or_else(|| "audio".into())
            });
            let (content_type, body) = asr_multipart_body(&self.model, &file_name, &request);
            let headers = voice_multipart_headers(&key, &content_type);
            let result = self
                .http
                .send(VoiceHttpRequest {
                    method: "POST".into(),
                    url: url.clone(),
                    headers,
                    body: VoiceHttpBody::Bytes(body),
                })
                .await;
            match result {
                Ok(response) => {
                    let status = response.status;
                    let text = String::from_utf8_lossy(&response.body).into_owned();
                    if !(200..=299).contains(&status) {
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

fn voice_json_headers(key: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("accept".into(), "audio/*".into()),
        ("accept-encoding".into(), "identity".into()),
        ("authorization".into(), format!("Bearer {key}")),
        ("content-type".into(), "application/json".into()),
    ])
}

fn voice_multipart_headers(key: &str, content_type: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("authorization".into(), format!("Bearer {key}")),
        ("content-type".into(), content_type.into()),
    ])
}

fn asr_multipart_body(model: &str, file_name: &str, request: &AsrRequest) -> (String, Vec<u8>) {
    let boundary = "ikaros-openai-compatible-asr-boundary";
    let mut body = Vec::new();
    push_multipart_text(&mut body, boundary, "model", model);
    if let Some(language) = &request.language {
        push_multipart_text(&mut body, boundary, "language", language);
    }
    push_multipart_file(&mut body, boundary, "file", file_name, request);
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn push_multipart_text(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{}\"\r\n\r\n",
            sanitize_multipart_token(name)
        )
        .as_bytes(),
    );
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

fn push_multipart_file(
    body: &mut Vec<u8>,
    boundary: &str,
    name: &str,
    file_name: &str,
    request: &AsrRequest,
) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
            sanitize_multipart_token(name),
            sanitize_multipart_token(file_name)
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "Content-Type: {}\r\n\r\n",
            request
                .format
                .as_ref()
                .map(audio_content_type)
                .unwrap_or("application/octet-stream")
        )
        .as_bytes(),
    );
    body.extend_from_slice(&request.audio);
    body.extend_from_slice(b"\r\n");
}

fn sanitize_multipart_token(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '"' | '\r' | '\n' => '_',
            _ => ch,
        })
        .collect()
}

fn audio_content_type(format: &AudioFormat) -> &'static str {
    match format {
        AudioFormat::Wav => "audio/wav",
        AudioFormat::Mp3 => "audio/mpeg",
        AudioFormat::Ogg => "audio/ogg",
    }
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
