// SPDX-License-Identifier: GPL-3.0-only

use crate::builder::{
    chat_model_services_for_session, host_agent_context_shape_checked, runtime_execution_env,
    services_for_instance,
};
use crate::send_model_json_request_for_instance;
use base64::Engine;
use ikaros_core::{IkarosError, IkarosPaths, Result, redact_secrets};
use ikaros_execution::harness::ExecutionEnv;
use ikaros_providers::model::{
    ModelContentBlock, ModelMessage, ModelRequest, ModelRequestOptions, TokenUsage,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HostImageGenerationRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub size: String,
    pub n: u32,
    pub response_format: String,
    pub quality: Option<String>,
    pub style: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub output_format: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    pub index: usize,
    pub path: PathBuf,
    pub bytes: usize,
}

#[derive(Debug, Clone)]
pub struct ImageGenerationResult {
    pub model: String,
    pub body: Value,
    pub saved: Vec<GeneratedImage>,
}

#[derive(Debug, Clone)]
pub struct VisionDescribeRequest {
    pub image: String,
    pub prompt: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VisionDescribeResult {
    pub model: String,
    pub content: String,
    pub usage: TokenUsage,
}

pub async fn generate_image(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    request: HostImageGenerationRequest,
) -> Result<ImageGenerationResult> {
    let host = host_agent_context_shape_checked(paths, workspace, agent_override)?;
    let config = host.config;
    let agent = host.agent_instance;
    let model_config = agent.model_config(&config.model.default).clone();
    let provider = agent
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let model = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or(model_config.model);
    if model.trim().is_empty() {
        return Err(IkarosError::Message(
            "image generation model must not be empty".into(),
        ));
    }
    let body = image_generation_body(&model, &request);
    let response = send_model_json_request_for_instance(
        paths,
        &config,
        &agent,
        &provider,
        "/images/generations",
        &body,
        "providers.model.base_url is required for image generation",
    )
    .await?;
    if !(200..300).contains(&response.status) {
        return Err(IkarosError::Message(format!(
            "image generation provider returned HTTP {}: {}",
            response.status,
            redact_secrets(&response.body)
        )));
    }
    let response_body: Value = serde_json::from_str(&response.body)?;
    let env = runtime_execution_env(&config, &agent.workspace)?;
    let saved = save_generated_images(
        &response_body,
        request.output_dir.as_deref(),
        &request.output_format,
        workspace,
        env.as_ref(),
    )
    .await?;
    Ok(ImageGenerationResult {
        model,
        body: response_body,
        saved,
    })
}

pub async fn describe_image(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    request: VisionDescribeRequest,
) -> Result<VisionDescribeResult> {
    let host = host_agent_context_shape_checked(paths, workspace, agent_override)?;
    let config = host.config;
    let agent = host.agent_instance;
    let services = services_for_instance(paths, &config, &agent)?;
    let model = chat_model_services_for_session(paths, &config, &agent, &services.session)?;
    let image_url =
        image_reference_to_url(&request.image, workspace, services.session.env.as_ref()).await?;
    let mut image = ModelContentBlock::image_url(image_url);
    if let ModelContentBlock::Image { detail, .. } = &mut image {
        *detail = request.detail;
    }
    let model_request = ModelRequest {
        messages: vec![ModelMessage::user_with_content_blocks(vec![
            ModelContentBlock::text(request.prompt),
            image,
        ])],
        options: ModelRequestOptions::default(),
        tools: Vec::new(),
    };
    let response = model.provider.generate(model_request).await?;
    Ok(VisionDescribeResult {
        model: response.model,
        content: response.content,
        usage: response.usage,
    })
}

fn image_generation_body(model: &str, request: &HostImageGenerationRequest) -> Value {
    let mut body = json!({
        "model": model,
        "prompt": &request.prompt,
        "n": request.n,
        "size": &request.size,
        "response_format": &request.response_format,
    });
    if let Some(quality) = request
        .quality
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body["quality"] = json!(quality);
    }
    if let Some(style) = request
        .style
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body["style"] = json!(style);
    }
    body
}

async fn save_generated_images(
    response: &Value,
    output_dir: Option<&Path>,
    output_format: &str,
    workspace: &Path,
    env: &dyn ExecutionEnv,
) -> Result<Vec<GeneratedImage>> {
    let Some(output_dir) = output_dir
        .map(|path| workspace_scoped_output_dir(path, workspace))
        .transpose()?
    else {
        return Ok(Vec::new());
    };
    env.create_dir_all(&output_dir).await.map_err(|error| {
        IkarosError::Message(format!(
            "failed to create image output dir {}: {error}",
            output_dir.display()
        ))
    })?;
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
            .map_err(|error| {
                IkarosError::Message(format!(
                    "image generation item {index} contains invalid base64: {error}"
                ))
            })?;
        let path = output_dir.join(format!("image-{}.{}", index + 1, extension));
        let byte_count = bytes.len();
        env.write_bytes(&path, bytes).await.map_err(|error| {
            IkarosError::Message(format!(
                "failed to write generated image {}: {error}",
                path.display()
            ))
        })?;
        saved.push(GeneratedImage {
            index,
            path,
            bytes: byte_count,
        });
    }
    Ok(saved)
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
    let bytes = env.read_bytes(&path).await.map_err(|error| {
        IkarosError::Message(format!("failed to read image {}: {error}", path.display()))
    })?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(IkarosError::Message(format!(
            "image is too large: {} bytes; max {} bytes",
            bytes.len(),
            MAX_IMAGE_BYTES
        )));
    }
    let mime = image_mime_type(&path);
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

fn workspace_scoped_output_dir(output_dir: &Path, workspace: &Path) -> Result<PathBuf> {
    let canonical_workspace = workspace
        .canonicalize()
        .map_err(|source| IkarosError::io(workspace, source))?;
    let candidate = if output_dir.is_absolute() {
        output_dir.to_path_buf()
    } else {
        canonical_workspace.join(output_dir)
    };
    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(IkarosError::Message(format!(
            "image output dir {} must not contain parent-directory components",
            candidate.display()
        )));
    }
    if candidate.exists() {
        let canonical_output = candidate
            .canonicalize()
            .map_err(|source| IkarosError::io(&candidate, source))?;
        if !canonical_output.starts_with(&canonical_workspace) {
            return Err(IkarosError::Message(format!(
                "image output dir {} is outside workspace {}",
                canonical_output.display(),
                canonical_workspace.display()
            )));
        }
        return Ok(canonical_output);
    }
    let parent = nearest_existing_parent(&candidate).ok_or_else(|| {
        IkarosError::Message(format!(
            "image output dir has no existing parent: {}",
            candidate.display()
        ))
    })?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|source| IkarosError::io(parent, source))?;
    if !canonical_parent.starts_with(&canonical_workspace) {
        return Err(IkarosError::Message(format!(
            "image output dir parent {} is outside workspace {}",
            canonical_parent.display(),
            canonical_workspace.display()
        )));
    }
    Ok(candidate)
}

fn workspace_scoped_image_path(value: &str, workspace: &Path) -> Result<PathBuf> {
    let raw = value.trim().strip_prefix("file://").unwrap_or(value.trim());
    if raw.is_empty() {
        return Err(IkarosError::Message("image path must not be empty".into()));
    }
    let path = PathBuf::from(raw);
    let candidate = if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    };
    let canonical_workspace = workspace
        .canonicalize()
        .map_err(|source| IkarosError::io(workspace, source))?;
    let canonical_image = candidate
        .canonicalize()
        .map_err(|source| IkarosError::io(&candidate, source))?;
    if !canonical_image.starts_with(&canonical_workspace) {
        return Err(IkarosError::Message(format!(
            "image {} is outside workspace {}",
            canonical_image.display(),
            canonical_workspace.display()
        )));
    }
    Ok(canonical_image)
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
