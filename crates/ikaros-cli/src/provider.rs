// SPDX-License-Identifier: GPL-3.0-only

use crate::resolve_agent_instance;
use anyhow::Result;
use clap::Subcommand;
use ikaros_core::{
    IkarosConfig, IkarosPaths, ModelConfig, ModelCostConfig, RemoteProviderConfig, redact_secrets,
    resolve_config_secret, resolve_config_value,
};
use ikaros_harness::{ExecutionEnv, NetworkEgressRequest};
use ikaros_models::{
    ModelProviderDescriptor, ModelRequest, ModelUsageLedger, ModelUsageRecord,
    ProviderHealthLedger, ProviderRegistry, governed_provider_from_config_with_http_client,
};
use ikaros_rag::{LocalRagStore, RagQuery};
use ikaros_runtime::{EgressModelHttpClient, runtime_execution_env};
use ikaros_voice::{
    AsrProvider, AsrRequest, AudioFormat, OpenAiCompatibleVoiceProvider, TtsProvider, TtsRequest,
    VoiceHttpBody, VoiceHttpClient, VoiceHttpRequest, VoiceHttpResponse, asr_provider_from_config,
    tts_provider_from_config,
};
use std::{path::Path, sync::Arc};

#[derive(Debug, Subcommand)]
pub(crate) enum ProviderCommand {
    Inspect,
    Health {
        #[arg(long)]
        live: bool,
    },
    Matrix {
        #[arg(long)]
        live: bool,
        #[arg(long)]
        json: bool,
    },
    Profiles,
}

pub(crate) async fn provider_command(
    command: ProviderCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        ProviderCommand::Inspect => inspect_provider(paths, workspace, agent_override),
        ProviderCommand::Health { live } => {
            provider_health(paths, workspace, agent_override, live).await
        }
        ProviderCommand::Matrix { live, json } => {
            provider_matrix(paths, workspace, agent_override, live, json).await
        }
        ProviderCommand::Profiles => provider_profiles(),
    }
}

fn inspect_provider(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let (model, provider_settings) =
        resolved_agent_model_config(&config, paths, workspace, agent_override)?;
    let mut descriptor = ProviderRegistry.descriptor_with_profile(
        &model.provider,
        &provider_settings.base_url,
        &model.model,
        &model.compat_profile,
    )?;
    apply_configured_model_cost(&mut descriptor, &model.cost);

    println!("provider: {}", descriptor.provider);
    println!("model: {}", redact_secrets(&descriptor.model));
    println!(
        "configured_profile: {}",
        redact_secrets(&model.compat_profile)
    );
    println!("profile: {}", descriptor.profile);
    println!(
        "profile_source: {}",
        provider_profile_source(
            &model.provider,
            Some(&model.compat_profile),
            Some(&descriptor)
        )
    );
    println!(
        "temperature_policy: {}",
        descriptor.profile_policy.temperature
    );
    println!("reasoning_policy: {}", descriptor.profile_policy.reasoning);
    println!("message_policy: {}", descriptor.profile_policy.message);
    println!(
        "tool_schema_policy: {}",
        descriptor.profile_policy.tool_schema
    );
    println!(
        "request_body_policy: {}",
        descriptor.profile_policy.request_body
    );
    println!(
        "retry_without_parameters: {}",
        format_retry_without_parameters(&descriptor.profile_policy.retry_without_parameters)
    );
    print_fallback_inspect_rows(&model)?;
    println!("context_window: {}", descriptor.context.context_window);
    println!(
        "default_output_tokens: {}",
        descriptor.context.default_output_tokens
    );
    println!("tokenizer: {:?}", descriptor.context.tokenizer);
    println!("streaming: {}", descriptor.capabilities.streaming);
    println!("tool_calls: {}", descriptor.capabilities.tool_calls);
    println!("reasoning: {}", descriptor.capabilities.reasoning);
    println!("json_mode: {}", descriptor.capabilities.json_mode);
    println!("network: {}", descriptor.capabilities.network);
    println!("image_input: {}", descriptor.capabilities.image_input);
    println!("audio_input: {}", descriptor.capabilities.audio_input);
    println!("file_input: {}", descriptor.capabilities.file_input);
    println!("health: {:?}", descriptor.health.status);
    if let Some(input) = descriptor.cost.input_per_million {
        println!(
            "cost_input_per_million: {} {}",
            input, descriptor.cost.currency
        );
    } else {
        println!("cost_input_per_million: unknown");
    }
    if let Some(output) = descriptor.cost.output_per_million {
        println!(
            "cost_output_per_million: {} {}",
            output, descriptor.cost.currency
        );
    } else {
        println!("cost_output_per_million: unknown");
    }
    if let Some(cache_read) = descriptor.cost.cache_read_per_million {
        println!(
            "cost_cache_read_per_million: {} {}",
            cache_read, descriptor.cost.currency
        );
    } else {
        println!("cost_cache_read_per_million: unknown");
    }
    if let Some(cache_write) = descriptor.cost.cache_write_per_million {
        println!(
            "cost_cache_write_per_million: {} {}",
            cache_write, descriptor.cost.currency
        );
    } else {
        println!("cost_cache_write_per_million: unknown");
    }
    Ok(())
}

fn print_fallback_inspect_rows(model: &ModelConfig) -> Result<()> {
    let registry = ProviderRegistry;
    println!("fallback_count: {}", model.fallbacks.len());
    for (index, fallback) in model.fallbacks.iter().enumerate() {
        let fallback_model = fallback.model_config();
        let fallback_provider = fallback.provider_config();
        let descriptor = registry.descriptor_with_profile(
            &fallback_model.provider,
            &fallback_provider.base_url,
            &fallback_model.model,
            &fallback_model.compat_profile,
        )?;
        let base_url_configured = !fallback_provider.base_url.trim().is_empty();
        let api_key_configured = !fallback_provider.api_key.trim().is_empty();
        println!(
            "fallback_row: index={} provider={} model={} configured_profile={} profile={} live_smoke={} streaming={} tool_calls={} reasoning={} network={} image_input={} audio_input={} file_input={} context_window={} default_output_tokens={}",
            index,
            redact_secrets(&descriptor.provider),
            redact_secrets(&descriptor.model),
            redact_secrets(&fallback_model.compat_profile),
            redact_secrets(&descriptor.profile),
            live_smoke_state(
                &fallback_model.provider,
                &fallback_model.model,
                base_url_configured,
                api_key_configured,
            ),
            descriptor.capabilities.streaming,
            descriptor.capabilities.tool_calls,
            descriptor.capabilities.reasoning,
            descriptor.capabilities.network,
            descriptor.capabilities.image_input,
            descriptor.capabilities.audio_input,
            descriptor.capabilities.file_input,
            descriptor.context.context_window,
            descriptor.context.default_output_tokens,
        );
    }
    Ok(())
}

fn resolved_agent_model_config(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<(ModelConfig, RemoteProviderConfig)> {
    let agent = resolve_agent_instance(config, agent_override, workspace, &paths.home)?;
    Ok((
        agent.model_config(&config.model.default).clone(),
        agent
            .effective_model_provider_config(&config.model.default, &config.providers.model)
            .clone(),
    ))
}

fn provider_profiles() -> Result<()> {
    let registry = ProviderRegistry;
    let profiles = registry.openai_compatible_profile_catalog();
    println!("provider_profiles: openai-compatible");
    println!("profile_count: {}", profiles.len());
    for profile in profiles {
        println!(
            "profile_row: provider={} profile={} auto_base_url_markers={} auto_model_markers={} auto_model_tail_prefixes={} temperature_policy={} reasoning_policy={} message_policy={} tool_schema_policy={} request_body_policy={} retry_without_parameters={} context_window={} default_output_tokens={} tokenizer={:?} streaming={} tool_calls={} reasoning={} json_mode={} network={} image_input={} audio_input={} file_input={}",
            redact_secrets(&profile.provider),
            redact_secrets(&profile.profile),
            format_marker_list(&profile.auto_base_url_markers),
            format_marker_list(&profile.auto_model_markers),
            format_marker_list(&profile.auto_model_tail_prefixes),
            redact_secrets(&profile.profile_policy.temperature),
            redact_secrets(&profile.profile_policy.reasoning),
            redact_secrets(&profile.profile_policy.message),
            redact_secrets(&profile.profile_policy.tool_schema),
            redact_secrets(&profile.profile_policy.request_body),
            format_retry_without_parameters(&profile.profile_policy.retry_without_parameters),
            profile.context.context_window,
            profile.context.default_output_tokens,
            profile.context.tokenizer,
            profile.capabilities.streaming,
            profile.capabilities.tool_calls,
            profile.capabilities.reasoning,
            profile.capabilities.json_mode,
            profile.capabilities.network,
            profile.capabilities.image_input,
            profile.capabilities.audio_input,
            profile.capabilities.file_input,
        );
    }
    Ok(())
}

async fn provider_matrix(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    live: bool,
    json: bool,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let (model_config, model_provider) =
        resolved_agent_model_config(&config, paths, workspace, agent_override)?;
    let registry = ProviderRegistry;
    let model_live_probe = if live {
        provider_live_probe(paths, workspace, &config, &model_config, &model_provider).await
    } else {
        LiveProbe::not_run()
    };
    let embedding_live_probe = if live {
        embedding_live_probe(paths, workspace, &config).await
    } else {
        LiveProbe::not_run()
    };
    let tts_live_probe = if live {
        tts_live_probe(workspace, &config).await
    } else {
        LiveProbe::not_run()
    };
    let asr_live_probe = if live {
        asr_live_probe(workspace, &config).await
    } else {
        LiveProbe::not_run()
    };
    let health = ProviderHealthLedger::new(&paths.audit_dir);
    let usage = ModelUsageLedger::new(&paths.audit_dir);
    let rows = vec![
        MatrixRow {
            registry: &registry,
            health: &health,
            usage: &usage,
            kind: "model",
            provider: &model_config.provider,
            model: &model_config.model,
            base_url: &model_provider.base_url,
            api_key: &model_provider.api_key,
            compat_profile: Some(&model_config.compat_profile),
            live_probe: &model_live_probe,
            fallback_models: fallback_model_names(&model_config),
            configured_cost: Some(&model_config.cost),
        },
        MatrixRow {
            registry: &registry,
            health: &health,
            usage: &usage,
            kind: "embedding",
            provider: &config.rag.embedding_provider,
            model: &config.rag.embedding_model,
            base_url: &config.providers.embedding.base_url,
            api_key: &config.providers.embedding.api_key,
            compat_profile: None,
            live_probe: &embedding_live_probe,
            fallback_models: Vec::new(),
            configured_cost: None,
        },
        MatrixRow {
            registry: &registry,
            health: &health,
            usage: &usage,
            kind: "tts",
            provider: &config.voice.tts.provider,
            model: &config.voice.tts.model,
            base_url: &config.providers.tts.base_url,
            api_key: &config.providers.tts.api_key,
            compat_profile: None,
            live_probe: &tts_live_probe,
            fallback_models: Vec::new(),
            configured_cost: None,
        },
        MatrixRow {
            registry: &registry,
            health: &health,
            usage: &usage,
            kind: "asr",
            provider: &config.voice.asr.provider,
            model: &config.voice.asr.model,
            base_url: &config.providers.asr.base_url,
            api_key: &config.providers.asr.api_key,
            compat_profile: None,
            live_probe: &asr_live_probe,
            fallback_models: Vec::new(),
            configured_cost: None,
        },
    ];
    if json {
        let rows = rows
            .into_iter()
            .map(matrix_row_json)
            .collect::<Result<Vec<_>>>()?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema": "ikaros-provider-matrix-v1",
                "version": 1,
                "live": live,
                "rows": rows,
            }))?
        );
        return Ok(());
    }
    println!("provider_matrix: live={live}");
    for row in rows {
        print_matrix_row(row)?;
    }
    Ok(())
}

fn fallback_model_names(model_config: &ModelConfig) -> Vec<String> {
    model_config
        .fallbacks
        .iter()
        .map(|fallback| {
            if fallback.model.trim().is_empty() {
                fallback.provider.to_string()
            } else {
                fallback.model.clone()
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
struct LiveProbe {
    status: String,
    detail: String,
}

impl LiveProbe {
    fn ok(detail: impl Into<String>) -> Self {
        Self {
            status: "ok".into(),
            detail: detail.into(),
        }
    }

    fn failed(detail: impl Into<String>) -> Self {
        Self {
            status: "failed".into(),
            detail: detail.into(),
        }
    }

    fn not_run() -> Self {
        Self {
            status: "not-run".into(),
            detail: "not-run".into(),
        }
    }
}

async fn provider_live_probe(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
    model_config: &ModelConfig,
    model_provider: &RemoteProviderConfig,
) -> LiveProbe {
    let Ok(env) = runtime_execution_env(config, workspace) else {
        return LiveProbe::failed("execution-env");
    };
    let provider = governed_provider_from_config_with_http_client(
        model_config,
        model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(env))),
    );
    let Ok(provider) = provider else {
        return LiveProbe::failed("provider-build");
    };
    match provider
        .generate(ModelRequest::from_user_text(
            "Ikaros provider matrix live probe. Reply with ok.",
        ))
        .await
    {
        Ok(response) => LiveProbe::ok(format!(
            "usage_total={}",
            response.usage.total_or_prompt_completion()
        )),
        Err(error) => LiveProbe::failed(error.to_string()),
    }
}

async fn embedding_live_probe(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
) -> LiveProbe {
    if matches!(
        config.rag.embedding_provider.to_ascii_lowercase().as_str(),
        "openai-compatible" | "ollama"
    ) {
        return remote_embedding_live_probe(workspace, config).await;
    }
    let store = match LocalRagStore::new(&paths.rag_dir, &config.rag.backend) {
        Ok(store) => store,
        Err(error) => return LiveProbe::failed(error.to_string()),
    };
    match store.search_with_embedding_provider(
        RagQuery {
            query: "ikaros live embedding probe".into(),
            top_k: 1,
            scope: None,
        },
        &config.rag.embedding_provider,
    ) {
        Ok(hits) => LiveProbe::ok(format!("hits={}", hits.len())),
        Err(error) => LiveProbe::failed(error.to_string()),
    }
}

async fn remote_embedding_live_probe(workspace: &Path, config: &IkarosConfig) -> LiveProbe {
    let env = match runtime_execution_env(config, workspace) {
        Ok(env) => env,
        Err(error) => return LiveProbe::failed(error.to_string()),
    };
    let provider = config.rag.embedding_provider.to_ascii_lowercase();
    let result = match provider.as_str() {
        "openai-compatible" => {
            let base_url = match resolve_config_value(
                &config.providers.embedding.base_url,
                "providers.embedding.base_url",
            ) {
                Ok(base_url) => base_url.trim_end_matches('/').to_owned(),
                Err(error) => return LiveProbe::failed(error.to_string()),
            };
            let key = match resolve_config_secret(
                &config.providers.embedding.api_key,
                "providers.embedding.api_key",
            ) {
                Ok(key) => key,
                Err(error) => return LiveProbe::failed(error.to_string()),
            };
            let mut headers = std::collections::BTreeMap::new();
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
            .await
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
            let mut headers = std::collections::BTreeMap::new();
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
            .await
        }
        _ => unreachable!("remote embedding provider is prefiltered"),
    };
    match result {
        Ok(response) if (200..=299).contains(&response.status) => {
            let vector_count = embedding_vector_count(&response.body);
            LiveProbe::ok(format!("vectors={vector_count}"))
        }
        Ok(response) => LiveProbe::failed(format!(
            "http_status={} body={}",
            response.status,
            redact_secrets(&response.body)
        )),
        Err(error) => LiveProbe::failed(error.to_string()),
    }
}

fn embedding_vector_count(body: &str) -> usize {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return 0;
    };
    if let Some(data) = value.get("data").and_then(serde_json::Value::as_array) {
        return data
            .iter()
            .filter(|item| item.get("embedding").is_some_and(embedding_value_non_empty))
            .count();
    }
    if let Some(embeddings) = value
        .get("embeddings")
        .and_then(serde_json::Value::as_array)
    {
        return embeddings.len();
    }
    value
        .get("embedding")
        .map(|embedding| usize::from(embedding_value_non_empty(embedding)))
        .unwrap_or(0)
}

fn embedding_value_non_empty(value: &serde_json::Value) -> bool {
    value
        .as_array()
        .is_some_and(|embedding| !embedding.is_empty())
        || value
            .as_str()
            .is_some_and(|embedding| !embedding.is_empty())
}

async fn tts_live_probe(workspace: &Path, config: &IkarosConfig) -> LiveProbe {
    let provider = match tts_provider_for_egress(workspace, config) {
        Ok(provider) => provider,
        Err(error) => return LiveProbe::failed(error.to_string()),
    };
    match provider
        .synthesize(TtsRequest {
            text: "Ikaros provider matrix TTS probe.".into(),
            voice: config.voice.tts.voice.clone(),
            format: AudioFormat::Wav,
            sample_rate_hz: Some(16_000),
            language: Some("en".into()),
        })
        .await
    {
        Ok(audio) => LiveProbe::ok(format!("bytes={}", audio.bytes.len())),
        Err(error) => LiveProbe::failed(error.to_string()),
    }
}

async fn asr_live_probe(workspace: &Path, config: &IkarosConfig) -> LiveProbe {
    let provider = match asr_provider_for_egress(workspace, config) {
        Ok(provider) => provider,
        Err(error) => return LiveProbe::failed(error.to_string()),
    };
    match provider
        .transcribe(AsrRequest {
            audio: asr_probe_wav(),
            file_name: Some("probe.wav".into()),
            format: Some(AudioFormat::Wav),
            sample_rate_hz: Some(16_000),
            language: Some("en".into()),
        })
        .await
    {
        Ok(transcript) => LiveProbe::ok(format!(
            "text_len={} confidence={}",
            transcript.text.len(),
            transcript
                .confidence
                .map(|confidence| confidence.to_string())
                .unwrap_or_else(|| "unknown".into())
        )),
        Err(error) => LiveProbe::failed(error.to_string()),
    }
}

fn tts_provider_for_egress(
    workspace: &Path,
    config: &IkarosConfig,
) -> Result<Box<dyn TtsProvider>> {
    if provider_matrix_voice_is_mock(&config.voice.tts.provider) {
        return Ok(tts_provider_from_config(
            &config.voice.tts,
            &config.providers.tts,
        )?);
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
                Arc::new(ProviderMatrixVoiceHttpClient::new(env)),
            )?,
        ));
    }
    Ok(tts_provider_from_config(
        &config.voice.tts,
        &config.providers.tts,
    )?)
}

fn asr_provider_for_egress(
    workspace: &Path,
    config: &IkarosConfig,
) -> Result<Box<dyn AsrProvider>> {
    if provider_matrix_voice_is_mock(&config.voice.asr.provider) {
        return Ok(asr_provider_from_config(
            &config.voice.asr,
            &config.providers.asr,
        )?);
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
                Arc::new(ProviderMatrixVoiceHttpClient::new(env)),
            )?,
        ));
    }
    Ok(asr_provider_from_config(
        &config.voice.asr,
        &config.providers.asr,
    )?)
}

fn provider_matrix_voice_is_mock(provider: &str) -> bool {
    matches!(
        provider.to_ascii_lowercase().as_str(),
        "mock" | "mock-tts" | "mock-asr"
    )
}

#[derive(Clone)]
struct ProviderMatrixVoiceHttpClient {
    env: Arc<dyn ExecutionEnv>,
}

impl ProviderMatrixVoiceHttpClient {
    fn new(env: Arc<dyn ExecutionEnv>) -> Self {
        Self { env }
    }
}

#[async_trait::async_trait]
impl VoiceHttpClient for ProviderMatrixVoiceHttpClient {
    async fn send(&self, request: VoiceHttpRequest) -> ikaros_core::Result<VoiceHttpResponse> {
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

struct MatrixRow<'a> {
    registry: &'a ProviderRegistry,
    health: &'a ProviderHealthLedger,
    usage: &'a ModelUsageLedger,
    kind: &'a str,
    provider: &'a str,
    model: &'a str,
    base_url: &'a str,
    api_key: &'a str,
    compat_profile: Option<&'a str>,
    live_probe: &'a LiveProbe,
    fallback_models: Vec<String>,
    configured_cost: Option<&'a ModelCostConfig>,
}

fn print_matrix_row(row: MatrixRow<'_>) -> Result<()> {
    let mut descriptor = match row.compat_profile {
        Some(compat_profile) => row.registry.descriptor_with_profile(
            row.provider,
            row.base_url,
            row.model,
            compat_profile,
        ),
        None => row
            .registry
            .descriptor(row.provider, row.base_url, row.model),
    }
    .ok();
    if let (Some(descriptor), Some(cost)) = (&mut descriptor, row.configured_cost) {
        apply_configured_model_cost(descriptor, cost);
    }
    let health_record = row.health.latest(row.provider, row.model).ok().flatten();
    let provider = redact_secrets(row.provider);
    let model = redact_secrets(row.model);
    let base_url_configured = !row.base_url.trim().is_empty();
    let api_key_configured = !row.api_key.trim().is_empty();
    let live_smoke = live_smoke_state(&provider, &model, base_url_configured, api_key_configured);
    let usage_today =
        matrix_usage_summary(row.usage, row.provider, row.model, descriptor.as_ref())?;
    println!(
        "matrix_row: kind={} provider={} model={} base_url_configured={} api_key_configured={} live_smoke={} live_probe={} probe_detail={} health_status={} consecutive_failures={} cooldown_until={} configured_profile={} provider_profile={} profile_source={} temperature_policy={} reasoning_policy={} message_policy={} tool_schema_policy={} request_body_policy={} retry_without_parameters={} context_window={} default_output_tokens={} tokenizer={} streaming={} tool_calls={} reasoning={} json_mode={} network={} image_input={} audio_input={} file_input={} cost_input_per_million={} cost_output_per_million={} cost_cache_read_per_million={} cost_cache_write_per_million={} cost_currency={} usage_requests_today={} usage_prompt_tokens_today={} usage_completion_tokens_today={} usage_total_tokens_today={} cache_read_tokens_today={} cache_write_tokens_today={} estimated_cost_today={} cache_accounting={} fallback_role={} fallback_count={} fallback_models={} debug_hint={}",
        redact_secrets(row.kind),
        provider,
        model,
        base_url_configured,
        api_key_configured,
        live_smoke,
        redact_secrets(&row.live_probe.status),
        redact_secrets(&row.live_probe.detail),
        health_record
            .as_ref()
            .map(|record| format!("{:?}", record.status))
            .unwrap_or_else(|| "Unknown".into()),
        health_record
            .as_ref()
            .map(|record| record.consecutive_failures.to_string())
            .unwrap_or_else(|| "0".into()),
        health_record
            .as_ref()
            .and_then(|record| record.cooldown_until.clone())
            .map(|cooldown| cooldown.to_string())
            .unwrap_or_else(|| "none".into()),
        matrix_configured_profile(row.provider, row.compat_profile),
        matrix_profile(&descriptor),
        provider_profile_source(row.provider, row.compat_profile, descriptor.as_ref()),
        matrix_policy(&descriptor, |policy| &policy.temperature),
        matrix_policy(&descriptor, |policy| &policy.reasoning),
        matrix_policy(&descriptor, |policy| &policy.message),
        matrix_policy(&descriptor, |policy| &policy.tool_schema),
        matrix_policy(&descriptor, |policy| &policy.request_body),
        matrix_retry_without_parameters(&descriptor),
        matrix_context_window(&descriptor),
        matrix_default_output_tokens(&descriptor),
        matrix_tokenizer(&descriptor),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.streaming),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.tool_calls),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.reasoning),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.json_mode),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.network),
        matrix_capability(&descriptor, |descriptor| descriptor
            .capabilities
            .image_input),
        matrix_capability(&descriptor, |descriptor| descriptor
            .capabilities
            .audio_input),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.file_input),
        matrix_cost(&descriptor, |descriptor| descriptor.cost.input_per_million),
        matrix_cost(&descriptor, |descriptor| descriptor.cost.output_per_million),
        matrix_cost(&descriptor, |descriptor| descriptor
            .cost
            .cache_read_per_million),
        matrix_cost(&descriptor, |descriptor| descriptor
            .cost
            .cache_write_per_million),
        descriptor
            .as_ref()
            .map(|descriptor| descriptor.cost.currency.as_str())
            .unwrap_or("unknown"),
        usage_today.requests,
        usage_today.prompt_tokens,
        usage_today.completion_tokens,
        usage_today.total_tokens,
        usage_today.cache_read_tokens,
        usage_today.cache_write_tokens,
        usage_today.estimated_cost_today,
        usage_today.cache_accounting,
        matrix_fallback_role(row.kind),
        row.fallback_models.len(),
        format_fallback_models(&row.fallback_models),
        matrix_debug_hint(live_smoke)
    );
    Ok(())
}

fn matrix_row_json(row: MatrixRow<'_>) -> Result<serde_json::Value> {
    let mut descriptor = match row.compat_profile {
        Some(compat_profile) => row.registry.descriptor_with_profile(
            row.provider,
            row.base_url,
            row.model,
            compat_profile,
        ),
        None => row
            .registry
            .descriptor(row.provider, row.base_url, row.model),
    }
    .ok();
    if let (Some(descriptor), Some(cost)) = (&mut descriptor, row.configured_cost) {
        apply_configured_model_cost(descriptor, cost);
    }
    let health_record = row.health.latest(row.provider, row.model).ok().flatten();
    let base_url_configured = !row.base_url.trim().is_empty();
    let api_key_configured = !row.api_key.trim().is_empty();
    let live_smoke = live_smoke_state(
        row.provider,
        row.model,
        base_url_configured,
        api_key_configured,
    );
    let usage_today =
        matrix_usage_summary(row.usage, row.provider, row.model, descriptor.as_ref())?;
    Ok(serde_json::json!({
        "kind": redact_secrets(row.kind),
        "provider": redact_secrets(row.provider),
        "model": redact_secrets(row.model),
        "configured": {
            "base_url": base_url_configured,
            "api_key": api_key_configured,
            "profile": matrix_configured_profile(row.provider, row.compat_profile),
        },
        "profile": {
            "resolved": matrix_profile(&descriptor),
            "source": provider_profile_source(row.provider, row.compat_profile, descriptor.as_ref()),
            "temperature_policy": matrix_policy(&descriptor, |policy| &policy.temperature),
            "reasoning_policy": matrix_policy(&descriptor, |policy| &policy.reasoning),
            "message_policy": matrix_policy(&descriptor, |policy| &policy.message),
            "tool_schema_policy": matrix_policy(&descriptor, |policy| &policy.tool_schema),
            "request_body_policy": matrix_policy(&descriptor, |policy| &policy.request_body),
            "retry_without_parameters": matrix_retry_without_parameters(&descriptor),
        },
        "context": {
            "context_window": matrix_context_window(&descriptor),
            "default_output_tokens": matrix_default_output_tokens(&descriptor),
            "tokenizer": matrix_tokenizer(&descriptor),
        },
        "capabilities": {
            "streaming": matrix_capability(&descriptor, |descriptor| descriptor.capabilities.streaming),
            "tool_calls": matrix_capability(&descriptor, |descriptor| descriptor.capabilities.tool_calls),
            "reasoning": matrix_capability(&descriptor, |descriptor| descriptor.capabilities.reasoning),
            "json_mode": matrix_capability(&descriptor, |descriptor| descriptor.capabilities.json_mode),
            "network": matrix_capability(&descriptor, |descriptor| descriptor.capabilities.network),
            "image_input": matrix_capability(&descriptor, |descriptor| descriptor.capabilities.image_input),
            "audio_input": matrix_capability(&descriptor, |descriptor| descriptor.capabilities.audio_input),
            "file_input": matrix_capability(&descriptor, |descriptor| descriptor.capabilities.file_input),
        },
        "health": {
            "status": health_record
                .as_ref()
                .map(|record| format!("{:?}", record.status))
                .unwrap_or_else(|| "Unknown".into()),
            "consecutive_failures": health_record
                .as_ref()
                .map(|record| record.consecutive_failures)
                .unwrap_or_default(),
            "cooldown_until": health_record
                .as_ref()
                .and_then(|record| record.cooldown_until.clone()),
        },
        "live": {
            "local_readiness": live_smoke,
            "probe_status": redact_secrets(&row.live_probe.status),
            "probe_detail": redact_secrets(&row.live_probe.detail),
            "debug_hint": matrix_debug_hint(live_smoke),
        },
        "cost": {
            "input_per_million": matrix_cost(&descriptor, |descriptor| descriptor.cost.input_per_million),
            "output_per_million": matrix_cost(&descriptor, |descriptor| descriptor.cost.output_per_million),
            "cache_read_per_million": matrix_cost(&descriptor, |descriptor| descriptor.cost.cache_read_per_million),
            "cache_write_per_million": matrix_cost(&descriptor, |descriptor| descriptor.cost.cache_write_per_million),
            "currency": descriptor
                .as_ref()
                .map(|descriptor| descriptor.cost.currency.as_str())
                .unwrap_or("unknown"),
        },
        "usage_today": {
            "requests": usage_today.requests,
            "prompt_tokens": usage_today.prompt_tokens,
            "completion_tokens": usage_today.completion_tokens,
            "total_tokens": usage_today.total_tokens,
            "cache_read_tokens": usage_today.cache_read_tokens,
            "cache_write_tokens": usage_today.cache_write_tokens,
            "estimated_cost": usage_today.estimated_cost_today,
            "cache_accounting": usage_today.cache_accounting,
        },
        "fallback": {
            "role": matrix_fallback_role(row.kind),
            "count": row.fallback_models.len(),
            "models": row.fallback_models
                .iter()
                .map(|model| redact_secrets(model))
                .collect::<Vec<_>>(),
        },
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatrixUsageSummary {
    requests: usize,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    estimated_cost_today: String,
    cache_accounting: &'static str,
}

fn matrix_usage_summary(
    usage: &ModelUsageLedger,
    provider: &str,
    model: &str,
    descriptor: Option<&ModelProviderDescriptor>,
) -> Result<MatrixUsageSummary> {
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let records = usage.read_all()?;
    let today_records = records
        .iter()
        .filter(|record| {
            record.at.starts_with(&today) && record.provider == provider && record.model == model
        })
        .collect::<Vec<_>>();
    let prompt_tokens = today_records
        .iter()
        .map(|record| record.prompt_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let completion_tokens = today_records
        .iter()
        .map(|record| record.completion_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let total_tokens = today_records
        .iter()
        .map(|record| record.total_tokens as u64)
        .sum::<u64>();
    let cache_read_tokens = today_records
        .iter()
        .map(|record| record.cache_read_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let cache_write_tokens = today_records
        .iter()
        .map(|record| record.cache_write_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let cost = descriptor.map(|descriptor| &descriptor.cost);
    let estimated_cost_today = cost
        .and_then(|cost| matrix_estimated_cost_today(&today_records, cost))
        .unwrap_or_else(|| "unknown".into());
    let cache_accounting = cost.map(matrix_cache_accounting).unwrap_or("unavailable");
    Ok(MatrixUsageSummary {
        requests: today_records.len(),
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_read_tokens,
        cache_write_tokens,
        estimated_cost_today,
        cache_accounting,
    })
}

fn matrix_estimated_cost_today(
    records: &[&ModelUsageRecord],
    cost: &ikaros_models::ModelProviderCost,
) -> Option<String> {
    let (Some(input), Some(output)) = (cost.input_per_million, cost.output_per_million) else {
        return None;
    };
    let prompt_tokens = records
        .iter()
        .map(|record| record.prompt_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let completion_tokens = records
        .iter()
        .map(|record| record.completion_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let cache_read_tokens = records
        .iter()
        .map(|record| record.cache_read_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let cache_write_tokens = records
        .iter()
        .map(|record| record.cache_write_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let cache_read_price = cost.cache_read_per_million.unwrap_or(input);
    let cache_write_price = cost.cache_write_per_million.unwrap_or(input);
    let regular_input_tokens =
        (prompt_tokens - cache_read_tokens - cache_write_tokens).clamp(0.0, f64::MAX);
    Some(format!(
        "{:.6}",
        ((regular_input_tokens * input)
            + (completion_tokens * output)
            + (cache_read_tokens * cache_read_price)
            + (cache_write_tokens * cache_write_price))
            / 1_000_000.0
    ))
}

fn matrix_cache_accounting(cost: &ikaros_models::ModelProviderCost) -> &'static str {
    if cost.cache_read_per_million.is_some() || cost.cache_write_per_million.is_some() {
        "priced"
    } else if cost.input_per_million.is_some() || cost.output_per_million.is_some() {
        "tracked"
    } else {
        "unavailable"
    }
}

fn apply_configured_model_cost(descriptor: &mut ModelProviderDescriptor, cost: &ModelCostConfig) {
    if !model_cost_is_configured(cost) {
        return;
    }
    descriptor.cost.currency = redact_secrets(&cost.currency);
    if let Some(input) = cost.input_per_million {
        descriptor.cost.input_per_million = Some(input);
    }
    if let Some(output) = cost.output_per_million {
        descriptor.cost.output_per_million = Some(output);
    }
    if let Some(cache_read) = cost.cache_read_per_million {
        descriptor.cost.cache_read_per_million = Some(cache_read);
    }
    if let Some(cache_write) = cost.cache_write_per_million {
        descriptor.cost.cache_write_per_million = Some(cache_write);
    }
}

fn model_cost_is_configured(cost: &ModelCostConfig) -> bool {
    cost.currency.trim() != "USD"
        || cost.input_per_million.is_some()
        || cost.output_per_million.is_some()
        || cost.cache_read_per_million.is_some()
        || cost.cache_write_per_million.is_some()
}

fn format_fallback_models(models: &[String]) -> String {
    if models.is_empty() {
        "none".into()
    } else {
        redact_secrets(&models.join(","))
    }
}

fn matrix_fallback_role(kind: &str) -> &'static str {
    if kind == "model" {
        "primary"
    } else {
        "not-applicable"
    }
}

fn matrix_debug_hint(live_smoke: &str) -> &'static str {
    match live_smoke {
        "ready" => "ready",
        "offline" | "local-ready" => "offline-provider",
        "missing-model" => "configure-model",
        "missing-base-url" => "configure-base-url",
        "missing-api-key" => "configure-api-key",
        _ => "inspect-provider",
    }
}

fn provider_profile_source(
    provider: &str,
    configured_profile: Option<&str>,
    descriptor: Option<&ModelProviderDescriptor>,
) -> &'static str {
    if provider.trim().eq_ignore_ascii_case("openai-compatible") {
        let configured = configured_profile
            .unwrap_or("auto")
            .trim()
            .to_ascii_lowercase();
        if configured.is_empty() || configured == "auto" {
            return match descriptor.map(|descriptor| descriptor.profile.as_str()) {
                Some("generic") => "auto-fallback",
                Some(_) => "auto-detected",
                None => "auto",
            };
        }
        return "explicit";
    }
    "native"
}

fn matrix_configured_profile(provider: &str, configured_profile: Option<&str>) -> String {
    let profile = if provider.trim().eq_ignore_ascii_case("openai-compatible") {
        configured_profile.unwrap_or("auto")
    } else {
        "native"
    };
    redact_secrets(profile)
}

fn format_retry_without_parameters(parameters: &[String]) -> String {
    if parameters.is_empty() {
        "none".into()
    } else {
        redact_secrets(&parameters.join(","))
    }
}

fn format_marker_list(markers: &[String]) -> String {
    if markers.is_empty() {
        "none".into()
    } else {
        redact_secrets(&markers.join(","))
    }
}

fn matrix_profile(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| redact_secrets(&descriptor.profile))
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_policy(
    descriptor: &Option<ModelProviderDescriptor>,
    read: impl FnOnce(&ikaros_models::ModelProviderProfilePolicy) -> &str,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| redact_secrets(read(&descriptor.profile_policy)))
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_retry_without_parameters(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| {
            format_retry_without_parameters(&descriptor.profile_policy.retry_without_parameters)
        })
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_context_window(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| descriptor.context.context_window.to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_default_output_tokens(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| descriptor.context.default_output_tokens.to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_tokenizer(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| format!("{:?}", descriptor.context.tokenizer))
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_capability(
    descriptor: &Option<ModelProviderDescriptor>,
    read: impl FnOnce(&ModelProviderDescriptor) -> bool,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| read(descriptor).to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_cost(
    descriptor: &Option<ModelProviderDescriptor>,
    read: impl FnOnce(&ModelProviderDescriptor) -> Option<f64>,
) -> String {
    descriptor
        .as_ref()
        .and_then(read)
        .map(|cost| cost.to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn live_smoke_state(
    provider: &str,
    model: &str,
    base_url_configured: bool,
    api_key_configured: bool,
) -> &'static str {
    match provider {
        "mock" | "hash" => "offline",
        "ollama" => {
            if model.trim().is_empty() {
                "missing-model"
            } else {
                "local-ready"
            }
        }
        _ if model.trim().is_empty() => "missing-model",
        _ if !base_url_configured => "missing-base-url",
        _ if !api_key_configured => "missing-api-key",
        _ => "ready",
    }
}

async fn provider_health(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    live: bool,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let (model, provider_settings) =
        resolved_agent_model_config(&config, paths, workspace, agent_override)?;
    if live {
        let env = runtime_execution_env(&config, workspace)?;
        let provider = governed_provider_from_config_with_http_client(
            &model,
            &provider_settings,
            &paths.audit_dir,
            Some(Arc::new(EgressModelHttpClient::new(env))),
        )?;
        match provider
            .generate(ModelRequest::from_user_text(
                "Ikaros provider health probe. Reply with a short ok.",
            ))
            .await
        {
            Ok(response) => {
                println!("live: ok");
                println!("provider: {}", response.provider);
                println!("model: {}", redact_secrets(&response.model));
                println!(
                    "usage_total: {}",
                    response.usage.total_or_prompt_completion()
                );
                return Ok(());
            }
            Err(error) => {
                println!("live: failed");
                println!("error: {}", redact_secrets(&error.to_string()));
                return Ok(());
            }
        }
    }

    let ledger = ProviderHealthLedger::new(&paths.audit_dir);
    let latest = ledger.latest(&model.provider, &model.model)?;
    println!("provider: {}", model.provider);
    println!("model: {}", redact_secrets(&model.model));
    if let Some(record) = latest {
        println!("health: {:?}", record.status);
        println!("consecutive_failures: {}", record.consecutive_failures);
        if let Some(kind) = record.last_error_kind {
            println!("last_error_kind: {:?}", kind);
        }
        if !record.last_error_summary.is_empty() {
            println!("last_error: {}", redact_secrets(&record.last_error_summary));
        }
        if let Some(cooldown_until) = record.cooldown_until {
            println!("cooldown_until: {cooldown_until}");
        }
    } else {
        println!("health: Unknown");
    }
    println!("health_log: {}", ledger.path().display());
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn provider_matrix_counts_float_and_base64_embedding_vectors() {
        assert_eq!(
            super::embedding_vector_count(
                r#"{"data":[{"embedding":[0.1,0.2]},{"embedding":"AACAPwAAIMA="}]}"#
            ),
            2
        );
        assert_eq!(
            super::embedding_vector_count(r#"{"embeddings":[[0.1,0.2],"AACAPwAAIMA="]}"#),
            2
        );
        assert_eq!(
            super::embedding_vector_count(r#"{"embedding":"AACAPwAAIMA="}"#),
            1
        );
    }

    #[test]
    fn provider_matrix_asr_probe_audio_is_valid_wav() {
        let wav = super::asr_probe_wav();

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
