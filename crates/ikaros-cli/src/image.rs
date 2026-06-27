// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use ikaros_core::{IkarosPaths, redact_json, redact_secrets};
use ikaros_host::{GeneratedImage, HostImageGenerationRequest};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub(crate) enum ImageCommand {
    Generate(ImageGenerateArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ImageGenerateArgs {
    pub(crate) prompt: String,
    #[arg(long)]
    pub(crate) model: Option<String>,
    #[arg(long, default_value = "1024x1024")]
    pub(crate) size: String,
    #[arg(long, default_value_t = 1)]
    pub(crate) n: u32,
    #[arg(long, value_enum, default_value = "url")]
    pub(crate) response_format: ImageResponseFormat,
    #[arg(long)]
    pub(crate) quality: Option<String>,
    #[arg(long)]
    pub(crate) style: Option<String>,
    #[arg(long = "output-dir")]
    pub(crate) output_dir: Option<PathBuf>,
    #[arg(long, default_value = "png")]
    pub(crate) output_format: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ImageResponseFormat {
    Url,
    B64Json,
}

impl ImageResponseFormat {
    fn as_openai_value(self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::B64Json => "b64_json",
        }
    }
}

pub(crate) async fn image_command(
    command: ImageCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        ImageCommand::Generate(args) => {
            generate_image(args, paths, workspace, agent_override).await
        }
    }
}

async fn generate_image(
    args: ImageGenerateArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let response_format = args.response_format.as_openai_value().to_owned();
    let result = ikaros_host::generate_image(
        paths,
        workspace,
        agent_override,
        HostImageGenerationRequest {
            prompt: args.prompt,
            model: args.model,
            size: args.size,
            n: args.n,
            response_format,
            quality: args.quality,
            style: args.style,
            output_dir: args.output_dir,
            output_format: args.output_format,
        },
    )
    .await?;
    print_image_generation_report(&result.model, &result.body, &result.saved)?;
    Ok(())
}

fn print_image_generation_report(
    model: &str,
    body: &Value,
    saved: &[GeneratedImage],
) -> Result<()> {
    println!("image_model: {}", redact_secrets(model));
    if let Some(created) = body.get("created").and_then(Value::as_i64) {
        println!("image_created: {created}");
    }
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    println!("image_count: {}", items.len());
    for (index, item) in items.iter().enumerate() {
        let url = item
            .get("url")
            .and_then(Value::as_str)
            .map(redact_secrets)
            .unwrap_or_else(|| "none".into());
        let b64_bytes = item
            .get("b64_json")
            .and_then(Value::as_str)
            .map(estimated_base64_decoded_len)
            .unwrap_or_default();
        let revised_prompt = item
            .get("revised_prompt")
            .and_then(Value::as_str)
            .map(redact_secrets)
            .unwrap_or_else(|| "none".into());
        println!(
            "image_item: index={} url={} b64_bytes={} revised_prompt={}",
            index, url, b64_bytes, revised_prompt
        );
    }
    for image in saved {
        println!(
            "image_saved: index={} path={} bytes={}",
            image.index,
            image.path.display(),
            image.bytes
        );
    }
    println!(
        "image_json: {}",
        serde_json::to_string(&redact_image_generation_response(body))?
    );
    Ok(())
}

fn redact_image_generation_response(value: &Value) -> Value {
    let mut redacted = redact_json(value.clone());
    if let Some(items) = redacted.get_mut("data").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(encoded) = item.get("b64_json").and_then(Value::as_str) {
                let bytes = estimated_base64_decoded_len(encoded);
                item["b64_json"] = json!({
                    "redacted": true,
                    "bytes_estimate": bytes,
                });
            }
        }
    }
    redacted
}

fn estimated_base64_decoded_len(value: &str) -> usize {
    let trimmed = value.trim_end_matches('=');
    trimmed.len().saturating_mul(3) / 4
}
