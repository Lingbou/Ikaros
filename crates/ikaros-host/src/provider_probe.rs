// SPDX-License-Identifier: GPL-3.0-only

use crate::builder::runtime_execution_env;
use crate::model_http::EgressModelHttpClient;
use ikaros_core::{
    IkarosConfig, IkarosError, IkarosPaths, ModelConfig, RemoteProviderConfig, Result,
    redact_secrets, resolve_config_secret, resolve_config_value,
};
use ikaros_execution::harness::{ExecutionEnv, NetworkEgressRequest};
use ikaros_providers::model::{ModelRequest, governed_provider_from_config_with_http_client};
use ikaros_providers::voice::{
    AsrProvider, AsrRequest, AudioFormat, OpenAiCompatibleVoiceProvider, TtsProvider, TtsRequest,
    VoiceHttpBody, VoiceHttpClient, VoiceHttpRequest, VoiceHttpResponse, asr_provider_from_config,
    tts_provider_from_config,
};
use ikaros_state::rag::{LocalRagStore, RagQuery};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProviderLiveProbeReport {
    pub provider: String,
    pub model: String,
    pub usage_total: u32,
}

pub async fn model_provider_live_probe(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
    model_config: &ModelConfig,
    model_provider: &RemoteProviderConfig,
    prompt: impl Into<String>,
) -> Result<ModelProviderLiveProbeReport> {
    let env = runtime_execution_env(config, workspace)?;
    let provider = governed_provider_from_config_with_http_client(
        model_config,
        model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(env))),
    )?;
    let response = provider
        .generate(ModelRequest::from_user_text(prompt.into()))
        .await?;
    Ok(ModelProviderLiveProbeReport {
        provider: response.provider,
        model: response.model,
        usage_total: response.usage.total_or_prompt_completion(),
    })
}

pub async fn embedding_provider_live_probe(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
) -> Result<String> {
    if matches!(
        config.rag.embedding_provider.to_ascii_lowercase().as_str(),
        "openai-compatible" | "ollama"
    ) {
        return remote_embedding_live_probe(workspace, config).await;
    }
    let store = LocalRagStore::new(&paths.rag_dir, &config.rag.backend)?;
    let hits = store.search_with_embedding_provider(
        RagQuery {
            query: "ikaros live embedding probe".into(),
            top_k: 1,
            scope: None,
        },
        &config.rag.embedding_provider,
    )?;
    Ok(format!("hits={}", hits.len()))
}

pub async fn tts_provider_live_probe(workspace: &Path, config: &IkarosConfig) -> Result<String> {
    let provider = tts_provider_for_egress(workspace, config)?;
    let audio = provider
        .synthesize(TtsRequest {
            text: "Ikaros provider matrix TTS probe.".into(),
            voice: config.voice.tts.voice.clone(),
            format: AudioFormat::Wav,
            sample_rate_hz: Some(16_000),
            language: Some("en".into()),
        })
        .await?;
    Ok(format!("bytes={}", audio.bytes.len()))
}

pub async fn asr_provider_live_probe(workspace: &Path, config: &IkarosConfig) -> Result<String> {
    let provider = asr_provider_for_egress(workspace, config)?;
    let transcript = provider
        .transcribe(AsrRequest {
            audio: asr_probe_wav(),
            file_name: Some("probe.wav".into()),
            format: Some(AudioFormat::Wav),
            sample_rate_hz: Some(16_000),
            language: Some("en".into()),
        })
        .await?;
    Ok(format!(
        "text_len={} confidence={}",
        transcript.text.len(),
        transcript
            .confidence
            .map(|confidence| confidence.to_string())
            .unwrap_or_else(|| "unknown".into())
    ))
}

async fn remote_embedding_live_probe(workspace: &Path, config: &IkarosConfig) -> Result<String> {
    let env = runtime_execution_env(config, workspace)?;
    let provider = config.rag.embedding_provider.to_ascii_lowercase();
    let response = match provider.as_str() {
        "openai-compatible" => {
            let base_url = resolve_config_value(
                &config.providers.embedding.base_url,
                "providers.embedding.base_url",
            )?
            .trim_end_matches('/')
            .to_owned();
            let key = resolve_config_secret(
                &config.providers.embedding.api_key,
                "providers.embedding.api_key",
            )?;
            let mut headers = BTreeMap::new();
            headers.insert("authorization".into(), format!("Bearer {key}"));
            headers.insert("content-type".into(), "application/json".into());
            env.send_network_request(NetworkEgressRequest {
                method: "POST".into(),
                url: format!("{base_url}/embeddings"),
                headers,
                body: Some(
                    serde_json::json!({
                        "model": &config.rag.embedding_model,
                        "input": "ikaros live embedding probe"
                    })
                    .to_string(),
                ),
                body_bytes: None,
            })
            .await?
        }
        "ollama" => {
            let base_url = if config.providers.embedding.base_url.trim().is_empty() {
                "http://127.0.0.1:11434".to_owned()
            } else {
                config
                    .providers
                    .embedding
                    .base_url
                    .trim_end_matches('/')
                    .to_owned()
            };
            let mut headers = BTreeMap::new();
            headers.insert("content-type".into(), "application/json".into());
            env.send_network_request(NetworkEgressRequest {
                method: "POST".into(),
                url: format!("{base_url}/api/embed"),
                headers,
                body: Some(
                    serde_json::json!({
                        "model": &config.rag.embedding_model,
                        "input": "ikaros live embedding probe"
                    })
                    .to_string(),
                ),
                body_bytes: None,
            })
            .await?
        }
        _ => unreachable!("remote embedding provider is prefiltered"),
    };
    if (200..=299).contains(&response.status) {
        let vector_count = embedding_vector_count(&response.body);
        return Ok(format!("vectors={vector_count}"));
    }
    Err(IkarosError::Message(format!(
        "http_status={} body={}",
        response.status,
        redact_secrets(&response.body)
    )))
}

fn tts_provider_for_egress(
    workspace: &Path,
    config: &IkarosConfig,
) -> Result<Box<dyn TtsProvider>> {
    if provider_matrix_voice_is_mock(&config.voice.tts.provider) {
        return tts_provider_from_config(&config.voice.tts, &config.providers.tts);
    }
    if config
        .voice
        .tts
        .provider
        .eq_ignore_ascii_case("openai-compatible")
    {
        let env = runtime_execution_env(config, workspace)?;
        return Ok(Box::new(
            OpenAiCompatibleVoiceProvider::from_config_with_http_client(
                config.voice.tts.provider.to_string(),
                &config.voice.tts,
                &config.providers.tts,
                Arc::new(ProviderProbeVoiceHttpClient::new(env)),
            )?,
        ));
    }
    tts_provider_from_config(&config.voice.tts, &config.providers.tts)
}

fn asr_provider_for_egress(
    workspace: &Path,
    config: &IkarosConfig,
) -> Result<Box<dyn AsrProvider>> {
    if provider_matrix_voice_is_mock(&config.voice.asr.provider) {
        return asr_provider_from_config(&config.voice.asr, &config.providers.asr);
    }
    if config
        .voice
        .asr
        .provider
        .eq_ignore_ascii_case("openai-compatible")
    {
        let env = runtime_execution_env(config, workspace)?;
        return Ok(Box::new(
            OpenAiCompatibleVoiceProvider::from_config_with_http_client(
                config.voice.asr.provider.to_string(),
                &config.voice.asr,
                &config.providers.asr,
                Arc::new(ProviderProbeVoiceHttpClient::new(env)),
            )?,
        ));
    }
    asr_provider_from_config(&config.voice.asr, &config.providers.asr)
}

fn provider_matrix_voice_is_mock(provider: &str) -> bool {
    matches!(
        provider.to_ascii_lowercase().as_str(),
        "mock" | "mock-tts" | "mock-asr"
    )
}

#[derive(Clone)]
struct ProviderProbeVoiceHttpClient {
    env: Arc<dyn ExecutionEnv>,
}

impl ProviderProbeVoiceHttpClient {
    fn new(env: Arc<dyn ExecutionEnv>) -> Self {
        Self { env }
    }
}

#[async_trait::async_trait]
impl VoiceHttpClient for ProviderProbeVoiceHttpClient {
    async fn send(&self, request: VoiceHttpRequest) -> Result<VoiceHttpResponse> {
        let (body, body_bytes) = match request.body {
            VoiceHttpBody::Text(body) => (Some(body), None),
            VoiceHttpBody::Bytes(body) => (None, Some(body)),
        };
        let response = self
            .env
            .send_network_request(NetworkEgressRequest {
                method: request.method,
                url: request.url,
                headers: request.headers,
                body,
                body_bytes,
            })
            .await?;
        Ok(VoiceHttpResponse {
            status: response.status,
            headers: response.headers,
            body: response
                .body_bytes
                .unwrap_or_else(|| response.body.into_bytes()),
        })
    }
}

fn embedding_vector_count(body: &str) -> usize {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return 0;
    };
    if let Some(data) = value.get("data").and_then(Value::as_array) {
        return data
            .iter()
            .filter(|item| item.get("embedding").is_some_and(embedding_value_non_empty))
            .count();
    }
    if let Some(embeddings) = value.get("embeddings").and_then(Value::as_array) {
        return embeddings.len();
    }
    value
        .get("embedding")
        .map(|embedding| usize::from(embedding_value_non_empty(embedding)))
        .unwrap_or(0)
}

fn embedding_value_non_empty(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|embedding| !embedding.is_empty())
        || value
            .as_str()
            .is_some_and(|embedding| !embedding.is_empty())
}

fn asr_probe_wav() -> Vec<u8> {
    let sample_rate = 16_000_u32;
    let channels = 1_u16;
    let bits_per_sample = 16_u16;
    let sample_count = sample_rate / 4;
    let bytes_per_sample = u32::from(bits_per_sample / 8);
    let data_len = sample_count * u32::from(channels) * bytes_per_sample;
    let byte_rate = sample_rate * u32::from(channels) * bytes_per_sample;
    let block_align = channels * (bits_per_sample / 8);
    let mut wav = Vec::with_capacity(44 + data_len as usize);

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(44 + data_len as usize, 0);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_matrix_counts_float_and_base64_embedding_vectors() {
        assert_eq!(
            embedding_vector_count(
                r#"{"data":[{"embedding":[0.1,0.2]},{"embedding":"AACAPwAAIMA="}]}"#
            ),
            2
        );
        assert_eq!(
            embedding_vector_count(r#"{"embeddings":[[0.1,0.2],"AACAPwAAIMA="]}"#),
            2
        );
        assert_eq!(embedding_vector_count(r#"{"embedding":"AACAPwAAIMA="}"#), 1);
    }

    #[test]
    fn provider_matrix_asr_probe_audio_is_valid_wav() {
        let wav = asr_probe_wav();

        assert!(wav.starts_with(b"RIFF"));
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert!(wav.windows(4).any(|window| window == b"data"));
        assert!(
            wav.len() > 44,
            "ASR live probe must send audio frames, not just a header"
        );
    }
}
