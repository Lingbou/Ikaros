// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    chat::render_terminal_markdown, resolve_agent_instance, session_and_registry_for_instance,
};
use anyhow::{Context, Result, anyhow};
use base64::Engine;
use clap::{Args, Subcommand};
use ikaros_core::{IkarosConfig, IkarosPaths, redact_secrets};
use ikaros_harness::ExecutionEnv;
use ikaros_models::{
    ModelContentBlock, ModelMessage, ModelRequest, ModelRequestOptions,
    governed_provider_from_config_with_http_client,
};
use ikaros_runtime::EgressModelHttpClient;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;

#[derive(Debug, Subcommand)]
pub(crate) enum VisionCommand {
    Describe(VisionDescribeArgs),
}

#[derive(Debug, Args)]
pub(crate) struct VisionDescribeArgs {
    pub(crate) image: String,
    #[arg(
        long,
        default_value = "Describe this image. Mention visible text, UI state, objects, and anything relevant to debugging or understanding the scene."
    )]
    pub(crate) prompt: String,
    #[arg(long)]
    pub(crate) detail: Option<String>,
}

pub(crate) async fn vision_command(
    command: VisionCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        VisionCommand::Describe(args) => {
            describe_image(args, paths, workspace, agent_override).await
        }
    }
}

async fn describe_image(
    args: VisionDescribeArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let model_config = agent.model_config(&config.model.default).clone();
    let model_provider = agent
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let (session, _) = session_and_registry_for_instance(paths, &config, &agent)?;
    let provider = governed_provider_from_config_with_http_client(
        &model_config,
        &model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(session.env.clone()))),
    )?;
    let image_url = image_reference_to_url(&args.image, workspace, session.env.as_ref()).await?;
    let mut image = ModelContentBlock::image_url(image_url);
    if let ModelContentBlock::Image { detail, .. } = &mut image {
        *detail = args.detail.clone();
    }
    let request = ModelRequest {
        messages: vec![ModelMessage::user_with_content_blocks(vec![
            ModelContentBlock::text(args.prompt),
            image,
        ])],
        options: ModelRequestOptions::default(),
        tools: Vec::new(),
    };
    let response = provider.generate(request).await?;
    println!("vision_model: {}", redact_secrets(&response.model));
    println!(
        "vision_content: {}",
        render_terminal_markdown(&response.content)
    );
    println!(
        "vision_usage: prompt_tokens={} completion_tokens={} total_tokens={}",
        response.usage.prompt_tokens.unwrap_or_default(),
        response.usage.completion_tokens.unwrap_or_default(),
        response.usage.total_tokens.unwrap_or_default()
    );
    Ok(())
}

async fn image_reference_to_url(
    value: &str,
    workspace: &Path,
    env: &dyn ExecutionEnv,
) -> Result<String> {
    if value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("data:image/")
    {
        return Ok(value.to_owned());
    }
    let path = workspace_scoped_image_path(value, workspace)?;
    let bytes = env
        .read_bytes(&path)
        .await
        .with_context(|| format!("failed to read image {}", path.display()))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        anyhow::bail!(
            "image is too large: {} bytes; max {} bytes",
            bytes.len(),
            MAX_IMAGE_BYTES
        );
    }
    let mime = image_mime_type(&path);
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

fn workspace_scoped_image_path(value: &str, workspace: &Path) -> Result<PathBuf> {
    let raw = value.trim().strip_prefix("file://").unwrap_or(value.trim());
    if raw.is_empty() {
        return Err(anyhow!("image path must not be empty"));
    }
    let path = PathBuf::from(raw);
    let candidate = if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    };
    let canonical_workspace = workspace
        .canonicalize()
        .with_context(|| format!("failed to canonicalize workspace {}", workspace.display()))?;
    let canonical_image = candidate
        .canonicalize()
        .with_context(|| format!("failed to canonicalize image {}", candidate.display()))?;
    if !canonical_image.starts_with(&canonical_workspace) {
        anyhow::bail!(
            "image {} is outside workspace {}",
            canonical_image.display(),
            canonical_workspace.display()
        );
    }
    Ok(canonical_image)
}

fn image_mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    }
}
