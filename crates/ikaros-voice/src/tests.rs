// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::{RemoteProviderConfig, VoiceProviderConfig};
use std::path::PathBuf;

#[tokio::test]
async fn mock_tts_redacts_text_preview() {
    let output = MockTtsProvider
        .synthesize(TtsRequest {
            text: "say sk-not-real".into(),
            voice: None,
            format: AudioFormat::Wav,
            sample_rate_hz: Some(24_000),
            language: Some("en".into()),
        })
        .await
        .expect("tts");
    assert!(!output.redacted_text_preview.contains("sk-not-real"));
    assert!(!String::from_utf8_lossy(&output.bytes).contains("sk-not-real"));
}

#[tokio::test]
async fn mock_asr_does_not_echo_audio_path() {
    let transcript = MockAsrProvider
        .transcribe(AsrRequest {
            audio_path: PathBuf::from("secret-audio.wav"),
            format: Some(AudioFormat::Wav),
            sample_rate_hz: Some(16_000),
            language: Some("en".into()),
        })
        .await
        .expect("asr");
    assert_eq!(transcript.text, "mock transcript");
    assert!(!transcript.text.contains("secret-audio.wav"));
}

#[test]
fn provider_factory_supports_canonical_mock_and_openai_compatible() {
    let empty_provider = RemoteProviderConfig::default();
    let tts = tts_provider_from_config(&VoiceProviderConfig::mock_tts(), &empty_provider)
        .expect("mock tts");
    assert_eq!(tts.name(), "mock-tts");
    let asr = asr_provider_from_config(&VoiceProviderConfig::mock_asr(), &empty_provider)
        .expect("mock asr");
    assert_eq!(asr.name(), "mock-asr");

    let config = VoiceProviderConfig {
        provider: "openai-compatible".into(),
        model: "tts-model".into(),
        timeout_ms: 1000,
        max_retries: 0,
        voice: Some("alloy".into()),
    };
    let provider_settings = RemoteProviderConfig {
        api_key: "test-voice-key".into(),
        base_url: "https://example.invalid/v1/".into(),
    };
    let provider = tts_provider_from_config(&config, &provider_settings).expect("compatible tts");
    assert_eq!(provider.name(), "openai-compatible");
}

#[test]
fn openai_compatible_tts_body_redacts_input() {
    let config = VoiceProviderConfig {
        provider: "openai-compatible".into(),
        model: "tts-model".into(),
        timeout_ms: 1000,
        max_retries: 0,
        voice: Some("nova".into()),
    };
    let body = openai_compatible::test_tts_speech_body(
        &config,
        &TtsRequest {
            text: "speak token=abc123".into(),
            voice: None,
            format: AudioFormat::Mp3,
            sample_rate_hz: None,
            language: None,
        },
    );
    assert_eq!(body["model"], "tts-model");
    assert_eq!(body["voice"], "nova");
    assert_eq!(body["response_format"], "mp3");
    assert!(
        body["input"]
            .as_str()
            .unwrap_or_default()
            .contains("[REDACTED_SECRET]")
    );
    assert!(!body.to_string().contains("abc123"));
}
