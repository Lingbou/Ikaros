// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_approval_hint, print_skill_result, session_and_registry};
use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_core::IkarosPaths;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub(crate) enum VoiceCommand {
    Tts(VoiceTts),
    Asr(VoiceAsr),
}

#[derive(Debug, Args)]
pub(crate) struct VoiceTts {
    text: String,
    #[arg(long)]
    voice: Option<String>,
    #[arg(long, default_value = "wav")]
    format: String,
    #[arg(long)]
    sample_rate_hz: Option<u32>,
    #[arg(long)]
    language: Option<String>,
    #[arg(long = "output")]
    path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub(crate) struct VoiceAsr {
    path: PathBuf,
    #[arg(long)]
    format: Option<String>,
    #[arg(long)]
    sample_rate_hz: Option<u32>,
    #[arg(long)]
    language: Option<String>,
}

pub(crate) async fn voice_command(
    command: VoiceCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = match command {
        VoiceCommand::Tts(args) => {
            let mut input = json!({
                "text": args.text,
                "format": args.format,
            });
            if let Some(voice) = args.voice {
                input["voice"] = json!(voice);
            }
            if let Some(sample_rate_hz) = args.sample_rate_hz {
                input["sample_rate_hz"] = json!(sample_rate_hz);
            }
            if let Some(language) = args.language {
                input["language"] = json!(language);
            }
            if let Some(path) = args.path {
                input["path"] = json!(path);
            }
            session.execute_skill(&registry, "voice_tts", input).await?
        }
        VoiceCommand::Asr(args) => {
            let mut input = json!({"path": args.path});
            if let Some(format) = args.format {
                input["format"] = json!(format);
            }
            if let Some(sample_rate_hz) = args.sample_rate_hz {
                input["sample_rate_hz"] = json!(sample_rate_hz);
            }
            if let Some(language) = args.language {
                input["language"] = json!(language);
            }
            session.execute_skill(&registry, "voice_asr", input).await?
        }
    };
    print_skill_result(&result)?;
    if !result.ok {
        print_approval_hint(&result);
    }
    println!("audit: {}", session.audit.path().display());
    if let Some(log) = session.approvals.log() {
        println!("approvals: {}", log.path().display());
    }
    Ok(())
}
