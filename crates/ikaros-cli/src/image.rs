// SPDX-License-Identifier: GPL-3.0-only

use crate::{resolve_agent_instance, session_and_registry_for_instance};
use anyhow::{Context, Result, bail};
use base64::Engine;
use clap::{Args, Subcommand, ValueEnum};
use ikaros_core::{IkarosConfig, IkarosPaths, RemoteProviderConfig, redact_json, redact_secrets};
use ikaros_harness::ExecutionEnv;
use ikaros_models::{ModelHttpClient, ModelHttpRequest};
use ikaros_runtime::EgressModelHttpClient;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

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
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let model_config = agent.model_config(&config.model.default).clone();
    let provider = agent
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let image_model = args.model.clone().unwrap_or(model_config.model);
    let (session, _) = session_and_registry_for_instance(paths, &config, &agent)?;
    let client = EgressModelHttpClient::new(session.env.clone());
    let request = image_generation_request(&provider, &image_model, &args)?;
    let response = client.send(request).await?;
    if !(200..300).contains(&response.status) {
        bail!(
            "image generation provider returned HTTP {}: {}",
            response.status,
            redact_secrets(&response.body)
        );
    }
    let body: Value = serde_json::from_str(&response.body)
        .with_context(|| "failed to parse image generation response JSON")?;
    let saved = save_generated_images(
        &body,
        args.output_dir.as_deref(),
        &args.output_format,
        workspace,
        session.env.as_ref(),
    )
    .await?;
    print_image_generation_report(&image_model, &body, &saved)?;
    Ok(())
}

fn image_generation_request(
    provider: &RemoteProviderConfig,
    model: &str,
    args: &ImageGenerateArgs,
) -> Result<ModelHttpRequest> {
    let base_url = provider.base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        bail!("providers.model.base_url is required for image generation");
    }
    if model.trim().is_empty() {
        bail!("image generation model must not be empty");
    }
    let mut body = json!({
        "model": model,
        "prompt": &args.prompt,
        "n": args.n,
        "size": &args.size,
        "response_format": args.response_format.as_openai_value(),
    });
    if let Some(quality) = args
        .quality
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body["quality"] = json!(quality);
    }
    if let Some(style) = args
        .style
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body["style"] = json!(style);
    }
    let mut headers = BTreeMap::new();
    headers.insert("content-type".into(), "application/json".into());
    if !provider.api_key.trim().is_empty() {
        headers.insert(
            "authorization".into(),
            format!("Bearer {}", provider.api_key.trim()),
        );
    }
    Ok(ModelHttpRequest {
        method: "POST".into(),
        url: format!("{base_url}/images/generations"),
        headers,
        body: serde_json::to_string(&body)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SavedImage {
    index: usize,
    path: PathBuf,
    bytes: usize,
}

async fn save_generated_images(
    response: &Value,
    output_dir: Option<&Path>,
    output_format: &str,
    workspace: &Path,
    env: &dyn ExecutionEnv,
) -> Result<Vec<SavedImage>> {
    let Some(output_dir) = output_dir
        .map(|path| workspace_scoped_output_dir(path, workspace))
        .transpose()?
    else {
        return Ok(Vec::new());
    };
    env.create_dir_all(&output_dir)
        .await
        .with_context(|| format!("failed to create image output dir {}", output_dir.display()))?;
    let extension = clean_image_extension(output_format);
    let mut saved = Vec::new();
    for (index, item) in response
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(encoded) = item.get("b64_json").and_then(Value::as_str) else {
            continue;
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .with_context(|| format!("image generation item {index} contains invalid base64"))?;
        let path = output_dir.join(format!("image-{}.{}", index + 1, extension));
        let byte_count = bytes.len();
        env.write_bytes(&path, bytes)
            .await
            .with_context(|| format!("failed to write generated image {}", path.display()))?;
        saved.push(SavedImage {
            index,
            path,
            bytes: byte_count,
        });
    }
    Ok(saved)
}

fn workspace_scoped_output_dir(output_dir: &Path, workspace: &Path) -> Result<PathBuf> {
    let canonical_workspace = workspace
        .canonicalize()
        .with_context(|| format!("failed to canonicalize workspace {}", workspace.display()))?;
    let candidate = if output_dir.is_absolute() {
        output_dir.to_path_buf()
    } else {
        canonical_workspace.join(output_dir)
    };
    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!(
            "image output dir {} must not contain parent-directory components",
            candidate.display()
        );
    }
    if candidate.exists() {
        let canonical_output = candidate.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize image output dir {}",
                candidate.display()
            )
        })?;
        if !canonical_output.starts_with(&canonical_workspace) {
            bail!(
                "image output dir {} is outside workspace {}",
                canonical_output.display(),
                canonical_workspace.display()
            );
        }
        return Ok(canonical_output);
    }
    let parent = nearest_existing_parent(&candidate).ok_or_else(|| {
        anyhow::anyhow!(
            "image output dir has no existing parent: {}",
            candidate.display()
        )
    })?;
    let canonical_parent = parent.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize image output parent {}",
            parent.display()
        )
    })?;
    if !canonical_parent.starts_with(&canonical_workspace) {
        bail!(
            "image output dir parent {} is outside workspace {}",
            canonical_parent.display(),
            canonical_workspace.display()
        );
    }
    Ok(candidate)
}

fn nearest_existing_parent(path: &Path) -> Option<&Path> {
    let mut current = path.parent();
    while let Some(parent) = current {
        if parent.exists() {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

fn clean_image_extension(value: &str) -> String {
    let extension = value
        .trim()
        .trim_start_matches('.')
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    if extension.is_empty() {
        "png".into()
    } else {
        extension
    }
}

fn print_image_generation_report(model: &str, body: &Value, saved: &[SavedImage]) -> Result<()> {
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
