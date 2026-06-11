// SPDX-License-Identifier: GPL-3.0-only

use async_trait::async_trait;
use ikaros_core::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    Mp3,
    Ogg,
}

impl AudioFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Ogg => "ogg",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TtsRequest {
    pub text: String,
    pub voice: Option<String>,
    pub format: AudioFormat,
    pub sample_rate_hz: Option<u32>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioOutput {
    pub path: Option<PathBuf>,
    pub format: AudioFormat,
    pub bytes: Vec<u8>,
    pub redacted_text_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AsrRequest {
    pub audio_path: PathBuf,
    pub format: Option<AudioFormat>,
    pub sample_rate_hz: Option<u32>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    pub language: Option<String>,
    pub confidence: Option<u8>,
}

#[async_trait]
pub trait TtsProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn synthesize(&self, request: TtsRequest) -> Result<AudioOutput>;
}

#[async_trait]
pub trait AsrProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn transcribe(&self, request: AsrRequest) -> Result<Transcript>;
}
