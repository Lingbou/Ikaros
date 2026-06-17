// SPDX-License-Identifier: GPL-3.0-only

use crate::support::{input_path, input_string, optional_input_path};
use async_trait::async_trait;
use ikaros_core::{IkarosError, RemoteProviderConfig, Result, RiskLevel, redact_secrets};
use ikaros_harness::{PolicyRequest, Skill, SkillContext, SkillOutput};
use ikaros_voice::{
    AsrRequest, AudioFormat, TtsRequest, VoiceProviderConfig, asr_provider_from_config,
    tts_provider_from_config,
};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct VoiceTtsSkill {
    config: VoiceProviderConfig,
    provider_settings: RemoteProviderConfig,
}

impl VoiceTtsSkill {
    pub fn new(config: VoiceProviderConfig, provider_settings: RemoteProviderConfig) -> Self {
        Self {
            config,
            provider_settings,
        }
    }
}

#[async_trait]
impl Skill for VoiceTtsSkill {
    fn name(&self) -> &'static str {
        "voice_tts"
    }

    fn description(&self) -> &'static str {
        "Synthesize speech with the configured TTS provider after redacting text."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": {"type": "string"},
                "voice": {"type": "string"},
                "format": {"type": "string", "enum": ["wav", "mp3", "ogg"]},
                "sample_rate_hz": {"type": "integer"},
                "language": {"type": "string"},
                "path": {"type": "string", "description": "Optional workspace output path for mock audio bytes."}
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::LocalWrite
    }

    fn policy_request(&self, input: &serde_json::Value, workspace_root: &Path) -> PolicyRequest {
        let path = input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| {
                if Path::new(path).is_absolute() {
                    PathBuf::from(path)
                } else {
                    workspace_root.join(path)
                }
            });
        let writes_file = path.is_some();
        let uses_network = !is_mock_provider(&self.config.provider);
        PolicyRequest {
            action: self.name().into(),
            risk: if uses_network {
                RiskLevel::Network
            } else if writes_file {
                RiskLevel::LocalWrite
            } else {
                RiskLevel::SafeRead
            },
            path,
            command: None,
            is_write: writes_file,
        }
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let text = input_string(&input, "text")?;
        let format = parse_audio_format(
            input
                .get("format")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("wav"),
        )?;
        let voice = input
            .get("voice")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let language = input
            .get("language")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let sample_rate_hz = optional_u32(&input, "sample_rate_hz")?;
        let redacted_text = redact_secrets(&text);
        let provider = tts_provider_from_config(&self.config, &self.provider_settings)?;
        let mut output = provider
            .synthesize(TtsRequest {
                text: redacted_text,
                voice: voice.clone().or_else(|| self.config.voice.clone()),
                format,
                sample_rate_hz,
                language: language.clone(),
            })
            .await?;

        if let Some(path) = optional_input_path(&input, "path", &ctx.session.sandbox.workspace_root)
        {
            ctx.session
                .env
                .write_bytes(&path, output.bytes.clone())
                .await?;
            output.path = Some(path);
        }

        Ok(SkillOutput::new(
            format!("{} TTS synthesized", provider.name()),
            json!({
                "provider": provider.name(),
                "format": audio_format_name(&output.format),
                "sample_rate_hz": sample_rate_hz,
                "language": language,
                "voice": voice,
                "path": output.path,
                "bytes_len": output.bytes.len(),
                "redacted_text_preview": output.redacted_text_preview,
            }),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct VoiceAsrSkill {
    config: VoiceProviderConfig,
    provider_settings: RemoteProviderConfig,
}

impl VoiceAsrSkill {
    pub fn new(config: VoiceProviderConfig, provider_settings: RemoteProviderConfig) -> Self {
        Self {
            config,
            provider_settings,
        }
    }
}

#[async_trait]
impl Skill for VoiceAsrSkill {
    fn name(&self) -> &'static str {
        "voice_asr"
    }

    fn description(&self) -> &'static str {
        "Transcribe a workspace audio path with the configured ASR provider."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {"type": "string"},
                "format": {"type": "string", "enum": ["wav", "mp3", "ogg"]},
                "sample_rate_hz": {"type": "integer"},
                "language": {"type": "string"}
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    fn policy_request(&self, input: &serde_json::Value, workspace_root: &Path) -> PolicyRequest {
        let path = input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| {
                if Path::new(path).is_absolute() {
                    PathBuf::from(path)
                } else {
                    workspace_root.join(path)
                }
            });
        PolicyRequest {
            action: self.name().into(),
            risk: if is_mock_provider(&self.config.provider) {
                RiskLevel::SafeRead
            } else {
                RiskLevel::Network
            },
            path,
            command: None,
            is_write: false,
        }
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let path = input_path(&input, &ctx.session.sandbox.workspace_root)?;
        let audio = ctx.session.env.read_bytes(&path).await?;
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string());
        let language = input
            .get("language")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let format = input
            .get("format")
            .and_then(serde_json::Value::as_str)
            .map(parse_audio_format)
            .transpose()?;
        let sample_rate_hz = optional_u32(&input, "sample_rate_hz")?;
        let provider = asr_provider_from_config(&self.config, &self.provider_settings)?;
        let transcript = provider
            .transcribe(AsrRequest {
                audio,
                file_name,
                format: format.clone(),
                sample_rate_hz,
                language: language.clone(),
            })
            .await?;
        Ok(SkillOutput::new(
            format!("{} ASR transcribed", provider.name()),
            json!({
                "provider": provider.name(),
                "audio": {
                    "format": format.as_ref().map(audio_format_name),
                    "sample_rate_hz": sample_rate_hz,
                    "language": language,
                },
                "transcript": transcript,
            }),
        ))
    }
}

fn is_mock_provider(provider: &str) -> bool {
    matches!(
        provider.to_ascii_lowercase().as_str(),
        "mock" | "mock-tts" | "mock-asr"
    )
}

fn parse_audio_format(value: &str) -> Result<AudioFormat> {
    match value.to_ascii_lowercase().as_str() {
        "wav" => Ok(AudioFormat::Wav),
        "mp3" => Ok(AudioFormat::Mp3),
        "ogg" => Ok(AudioFormat::Ogg),
        other => Err(IkarosError::Message(format!(
            "unsupported audio format: {other}"
        ))),
    }
}

fn optional_u32(input: &serde_json::Value, field: &str) -> Result<Option<u32>> {
    let Some(value) = input.get(field) else {
        return Ok(None);
    };
    let Some(number) = value.as_u64() else {
        return Err(IkarosError::Message(format!(
            "{field} must be a non-negative integer"
        )));
    };
    u32::try_from(number)
        .map(Some)
        .map_err(|_| IkarosError::Message(format!("{field} exceeds u32 range")))
}

fn audio_format_name(format: &AudioFormat) -> &'static str {
    match format {
        AudioFormat::Wav => "wav",
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Ogg => "ogg",
    }
}
