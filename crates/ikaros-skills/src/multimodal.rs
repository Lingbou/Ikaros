// SPDX-License-Identifier: GPL-3.0-only

use crate::support::input_string;
use base64::Engine;
use ikaros_core::{
    IkarosError, ModelConfig, RemoteProviderConfig, Result, RiskLevel, redact_json, redact_secrets,
};
use ikaros_models::{
    ModelContentBlock, ModelHttpClient, ModelHttpRequest, ModelHttpResponse, ModelMessage,
    ModelProvider, ModelRequest, ModelRequestOptions,
    governed_provider_from_config_with_http_client,
};
use ikaros_tools::{
    ExecutionEnv, NetworkEgressRequest, PolicyRequest, Skill, SkillContext, SkillOutput,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

const MAX_VISION_IMAGE_BYTES: usize = 12 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct VisionDescribeSkill {
    model: ModelConfig,
    provider: RemoteProviderConfig,
}

impl VisionDescribeSkill {
    pub fn new(model: ModelConfig, provider: RemoteProviderConfig) -> Self {
        Self { model, provider }
    }
}

#[derive(Debug, Clone)]
pub struct ImageGenerateSkill {
    model: ModelConfig,
    provider: RemoteProviderConfig,
}

impl ImageGenerateSkill {
    pub fn new(model: ModelConfig, provider: RemoteProviderConfig) -> Self {
        Self { model, provider }
    }
}

#[async_trait::async_trait]
impl Skill for VisionDescribeSkill {
    fn name(&self) -> &'static str {
        "vision_describe"
    }

    fn description(&self) -> &'static str {
        "Describe an image with the configured model provider through the harness execution boundary."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["image"],
            "properties": {
                "image": {
                    "type": "string",
                    "description": "HTTP(S), data:image URL, file:// URL, absolute workspace path, or workspace-relative path."
                },
                "prompt": {
                    "type": "string",
                    "description": "Instruction sent with the image."
                },
                "detail": {
                    "type": "string",
                    "description": "Optional provider image detail hint such as low, high, or auto."
                }
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Network
    }

    fn policy_request(&self, input: &Value, workspace_root: &Path) -> PolicyRequest {
        let image = input
            .get("image")
            .and_then(Value::as_str)
            .map(redact_secrets)
            .unwrap_or_else(|| "missing-image".into());
        let path = local_reference_path(input.get("image").and_then(Value::as_str), workspace_root);
        PolicyRequest {
            action: self.name().into(),
            risk: RiskLevel::Network,
            path,
            command: Some(format!("vision_describe {}", redact_secrets(&image))),
            is_write: false,
        }
    }

    fn approval_context(&self, input: &Value, workspace_root: &Path) -> Option<Value> {
        Some(json!({
            "kind": "vision_describe",
            "image": input.get("image").and_then(Value::as_str).map(redact_secrets),
            "path": local_reference_path(input.get("image").and_then(Value::as_str), workspace_root)
                .map(|path| redact_secrets(&path.display().to_string())),
            "provider": redact_secrets(&self.model.provider),
            "model": redact_secrets(&self.model.model),
            "network_egress": true,
        }))
    }

    async fn execute(&self, input: Value, ctx: SkillContext) -> Result<SkillOutput> {
        let image = input_string(&input, "image")?;
        let prompt = input
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("Describe this image. Mention visible text, UI state, objects, and anything relevant to debugging or understanding the scene.")
            .to_owned();
        let detail = input
            .get("detail")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let image_url = image_reference_to_url(&image, &ctx).await?;
        let mut image_block = ModelContentBlock::image_url(image_url);
        if let ModelContentBlock::Image {
            detail: block_detail,
            ..
        } = &mut image_block
        {
            *block_detail = detail;
        }
        let provider = model_provider_for_skill(&self.model, &self.provider, &ctx)?;
        if !provider.capabilities().image_input {
            return Err(IkarosError::Message(format!(
                "provider {} model {} does not support image content blocks",
                redact_secrets(provider.name()),
                redact_secrets(provider.model_id())
            )));
        }
        let request = ModelRequest {
            messages: vec![ModelMessage::user_with_content_blocks(vec![
                ModelContentBlock::text(prompt),
                image_block,
            ])],
            options: ModelRequestOptions::default(),
            tools: Vec::new(),
        };
        let response = provider.generate(request).await?;
        Ok(SkillOutput {
            summary: format!(
                "vision_describe model={} chars={}",
                redact_secrets(&response.model),
                response.content.chars().count()
            ),
            output: json!({
                "provider": redact_secrets(&response.provider),
                "model": redact_secrets(&response.model),
                "content": redact_secrets(&response.content),
                "usage": response.usage,
            }),
        })
    }
}

#[async_trait::async_trait]
impl Skill for ImageGenerateSkill {
    fn name(&self) -> &'static str {
        "image_generate"
    }

    fn description(&self) -> &'static str {
        "Generate images through an OpenAI-compatible image generation endpoint using governed network egress."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {"type": "string"},
                "model": {"type": "string"},
                "size": {"type": "string", "default": "1024x1024"},
                "n": {"type": "integer", "minimum": 1, "maximum": 8},
                "response_format": {"type": "string", "enum": ["url", "b64_json"]},
                "quality": {"type": "string"},
                "style": {"type": "string"}
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Network
    }

    fn policy_request(&self, input: &Value, _workspace_root: &Path) -> PolicyRequest {
        PolicyRequest {
            action: self.name().into(),
            risk: RiskLevel::Network,
            path: None,
            command: input
                .get("prompt")
                .and_then(Value::as_str)
                .map(|prompt| format!("image_generate {}", redact_secrets(prompt))),
            is_write: false,
        }
    }

    fn approval_context(&self, input: &Value, _workspace_root: &Path) -> Option<Value> {
        Some(json!({
            "kind": "image_generate",
            "prompt": input.get("prompt").and_then(Value::as_str).map(redact_secrets),
            "provider": redact_secrets(&self.model.provider),
            "model": input
                .get("model")
                .and_then(Value::as_str)
                .map(redact_secrets)
                .unwrap_or_else(|| redact_secrets(&self.model.model)),
            "network_egress": true,
        }))
    }

    async fn execute(&self, input: Value, ctx: SkillContext) -> Result<SkillOutput> {
        let prompt = input_string(&input, "prompt")?;
        let model = input
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.model.model);
        let request = image_generation_request(&self.provider, model, &input, &prompt)?;
        let client = SkillModelHttpClient::from_env(ctx.session.env.clone());
        let response = client.send(request).await?;
        if !(200..=299).contains(&response.status) {
            return Err(IkarosError::Message(format!(
                "image generation provider returned HTTP {}: {}",
                response.status,
                redact_secrets(&response.body)
            )));
        }
        let body: Value = serde_json::from_str(&response.body)
            .map_err(|source| IkarosError::Message(format!("invalid image JSON: {source}")))?;
        let redacted = redact_image_response(&body);
        let count = body
            .get("data")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        Ok(SkillOutput {
            summary: format!(
                "image_generate model={} count={count}",
                redact_secrets(model)
            ),
            output: json!({
                "model": redact_secrets(model),
                "count": count,
                "response": redacted,
            }),
        })
    }
}

#[derive(Clone)]
struct SkillModelHttpClient {
    env: Arc<dyn ExecutionEnv>,
}

impl SkillModelHttpClient {
    fn from_env(env: Arc<dyn ExecutionEnv>) -> Arc<dyn ModelHttpClient> {
        Arc::new(Self { env })
    }
}

impl ModelHttpClient for SkillModelHttpClient {
    fn send<'a>(
        &'a self,
        request: ModelHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ModelHttpResponse>> + Send + 'a>> {
        Box::pin(async move {
            let response = self
                .env
                .send_network_request(NetworkEgressRequest {
                    method: request.method,
                    url: request.url,
                    headers: request.headers,
                    body: Some(request.body),
                    body_bytes: None,
                })
                .await?;
            Ok(ModelHttpResponse {
                status: response.status,
                headers: response.headers,
                body: response.body,
            })
        })
    }
}

fn model_provider_for_skill(
    model: &ModelConfig,
    provider: &RemoteProviderConfig,
    ctx: &SkillContext,
) -> Result<Box<dyn ModelProvider>> {
    let audit_dir = ctx
        .session
        .audit
        .path()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    governed_provider_from_config_with_http_client(
        model,
        provider,
        audit_dir,
        Some(SkillModelHttpClient::from_env(ctx.session.env.clone())),
    )
}

async fn image_reference_to_url(value: &str, ctx: &SkillContext) -> Result<String> {
    if value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("data:image/")
    {
        return Ok(value.to_owned());
    }
    let path = workspace_image_path(value, &ctx.session.sandbox.workspace_root)?;
    let bytes = ctx.session.env.read_bytes(&path).await?;
    if bytes.len() > MAX_VISION_IMAGE_BYTES {
        return Err(IkarosError::Message(format!(
            "image is too large: {} bytes; max {} bytes",
            bytes.len(),
            MAX_VISION_IMAGE_BYTES
        )));
    }
    let mime = image_mime_type(&path);
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn workspace_image_path(value: &str, workspace: &Path) -> Result<PathBuf> {
    let raw = value.trim().strip_prefix("file://").unwrap_or(value.trim());
    if raw.is_empty() {
        return Err(IkarosError::Message("image path must not be empty".into()));
    }
    let path = PathBuf::from(raw);
    Ok(if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    })
}

fn local_reference_path(value: Option<&str>, workspace: &Path) -> Option<PathBuf> {
    let value = value?.trim();
    if value.starts_with("http://") || value.starts_with("https://") || value.starts_with("data:") {
        return None;
    }
    workspace_image_path(value, workspace).ok()
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

fn image_generation_request(
    provider: &RemoteProviderConfig,
    model: &str,
    input: &Value,
    prompt: &str,
) -> Result<ModelHttpRequest> {
    let base_url = provider.base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return Err(IkarosError::Message(
            "providers.model.base_url is required for image generation".into(),
        ));
    }
    let mut body = json!({
        "model": model,
        "prompt": redact_secrets(prompt),
        "n": input.get("n").and_then(Value::as_u64).unwrap_or(1),
        "size": input.get("size").and_then(Value::as_str).unwrap_or("1024x1024"),
        "response_format": input
            .get("response_format")
            .and_then(Value::as_str)
            .unwrap_or("url"),
    });
    for key in ["quality", "style"] {
        if let Some(value) = input
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            body[key] = json!(value);
        }
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

fn redact_image_response(value: &Value) -> Value {
    let mut redacted = redact_json(value.clone());
    if let Some(items) = redacted.get_mut("data").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(encoded) = item.get("b64_json").and_then(Value::as_str) {
                item["b64_json"] = json!({
                    "redacted": true,
                    "decoded_bytes_estimate": estimated_base64_decoded_len(encoded),
                });
            }
        }
    }
    redacted
}

fn estimated_base64_decoded_len(value: &str) -> usize {
    let padding = value.chars().rev().take_while(|ch| *ch == '=').count();
    (value.len().saturating_mul(3) / 4).saturating_sub(padding)
}
