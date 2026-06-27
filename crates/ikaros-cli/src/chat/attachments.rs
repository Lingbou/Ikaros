// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ikaros_core::redact_secrets;
use ikaros_models::ModelContentBlock;
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_INLINE_ATTACHMENT_BYTES: u64 = 8 * 1024 * 1024;

pub(in crate::chat) fn content_blocks_from_args(
    images: &[String],
    audios: &[String],
    files: &[String],
) -> Vec<ModelContentBlock> {
    let mut blocks = Vec::with_capacity(images.len() + audios.len() + files.len());
    blocks.extend(
        images
            .iter()
            .cloned()
            .map(|image_url| ModelContentBlock::Image {
                mime_type: guess_attachment_mime_type(&image_url),
                detail: None,
                image_url,
            }),
    );
    blocks.extend(
        audios
            .iter()
            .cloned()
            .map(|audio_url| ModelContentBlock::Audio {
                mime_type: guess_attachment_mime_type(&audio_url),
                audio_url,
            }),
    );
    blocks.extend(
        files
            .iter()
            .cloned()
            .map(|file_url| ModelContentBlock::File {
                mime_type: guess_attachment_mime_type(&file_url),
                name: attachment_display_name(&file_url),
                file_url,
            }),
    );
    blocks
}

pub(in crate::chat) fn content_blocks_from_args_resolving_paths(
    images: &[String],
    audios: &[String],
    files: &[String],
    workspace: &Path,
) -> Result<Vec<ModelContentBlock>> {
    let mut blocks = Vec::with_capacity(images.len() + audios.len() + files.len());
    for image in images {
        blocks.push(content_block_from_parts_resolving_path(
            "image", image, workspace,
        )?);
    }
    for audio in audios {
        blocks.push(content_block_from_parts_resolving_path(
            "audio", audio, workspace,
        )?);
    }
    for file in files {
        blocks.push(content_block_from_parts_resolving_path(
            "file", file, workspace,
        )?);
    }
    Ok(blocks)
}

pub(in crate::chat) fn content_block_from_parts(
    kind: &str,
    value: &str,
) -> Result<ModelContentBlock> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("attachment value is required"));
    }
    match kind {
        "image" => Ok(ModelContentBlock::Image {
            image_url: value.to_owned(),
            mime_type: guess_attachment_mime_type(value),
            detail: None,
        }),
        "audio" => Ok(ModelContentBlock::Audio {
            audio_url: value.to_owned(),
            mime_type: guess_attachment_mime_type(value),
        }),
        "file" => Ok(ModelContentBlock::File {
            file_url: value.to_owned(),
            mime_type: guess_attachment_mime_type(value),
            name: attachment_display_name(value),
        }),
        _ => Err(anyhow!(
            "unknown attachment kind `{}`; use image, audio, or file",
            redact_secrets(kind)
        )),
    }
}

pub(in crate::chat) fn content_block_from_parts_resolving_path(
    kind: &str,
    value: &str,
    workspace: &Path,
) -> Result<ModelContentBlock> {
    let mut block = content_block_from_parts(kind, value)?;
    match &mut block {
        ModelContentBlock::Image {
            image_url,
            mime_type,
            ..
        } => {
            if let Some((data_url, inferred_mime)) =
                local_attachment_data_url(image_url, workspace, mime_type.as_deref())?
            {
                *image_url = data_url;
                if mime_type.is_none() {
                    *mime_type = inferred_mime;
                }
            }
        }
        ModelContentBlock::Audio {
            audio_url,
            mime_type,
        } => {
            if let Some((data_url, inferred_mime)) =
                local_attachment_data_url(audio_url, workspace, mime_type.as_deref())?
            {
                *audio_url = data_url;
                if mime_type.is_none() {
                    *mime_type = inferred_mime;
                }
            }
        }
        ModelContentBlock::File {
            file_url,
            mime_type,
            ..
        } => {
            if let Some((data_url, inferred_mime)) =
                local_attachment_data_url(file_url, workspace, mime_type.as_deref())?
            {
                *file_url = data_url;
                if mime_type.is_none() {
                    *mime_type = inferred_mime;
                }
            }
        }
        ModelContentBlock::Text { .. } | ModelContentBlock::ToolResult { .. } => {}
    }
    Ok(block)
}

pub(in crate::chat) fn content_block_kind(block: &ModelContentBlock) -> &'static str {
    match block {
        ModelContentBlock::Text { .. } => "text",
        ModelContentBlock::Image { .. } => "image",
        ModelContentBlock::Audio { .. } => "audio",
        ModelContentBlock::File { .. } => "file",
        ModelContentBlock::ToolResult { .. } => "tool_result",
    }
}

pub(in crate::chat) fn content_block_summary(block: &ModelContentBlock) -> String {
    match block {
        ModelContentBlock::Text { text } => {
            format!("kind=text chars={}", redact_secrets(text).chars().count())
        }
        ModelContentBlock::Image {
            image_url,
            mime_type,
            detail,
        } => format!(
            "kind=image mime={} detail={} source={}",
            mime_type.as_deref().unwrap_or("unknown"),
            detail.as_deref().unwrap_or("auto"),
            attachment_source_preview(image_url)
        ),
        ModelContentBlock::Audio {
            audio_url,
            mime_type,
        } => format!(
            "kind=audio mime={} source={}",
            mime_type.as_deref().unwrap_or("unknown"),
            attachment_source_preview(audio_url)
        ),
        ModelContentBlock::File {
            file_url,
            mime_type,
            name,
        } => format!(
            "kind=file mime={} name={} source={}",
            mime_type.as_deref().unwrap_or("unknown"),
            name.as_deref().unwrap_or("unknown"),
            attachment_source_preview(file_url)
        ),
        ModelContentBlock::ToolResult {
            tool_call_id,
            is_error,
            ..
        } => format!(
            "kind=tool_result id={} is_error={}",
            redact_secrets(tool_call_id),
            is_error
        ),
    }
}

fn attachment_display_name(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with("data:") {
        return None;
    }
    let without_query = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let candidate = without_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(without_query);
    let candidate = candidate.strip_prefix("file://").unwrap_or(candidate);
    if candidate.is_empty() {
        None
    } else {
        Some(redact_secrets(candidate))
    }
}

fn attachment_source_preview(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("data:") {
        let (mime, payload) = rest.split_once(';').unwrap_or((rest, ""));
        return format!(
            "inline-data:{} chars={}",
            redact_secrets(mime),
            payload.chars().count()
        );
    }
    redact_secrets(value)
}

fn guess_attachment_mime_type(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix("data:") {
        return rest
            .split_once(';')
            .map(|(mime, _)| mime)
            .filter(|mime| !mime.trim().is_empty())
            .map(|mime| mime.to_ascii_lowercase());
    }
    let path = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let extension = path
        .rsplit('.')
        .next()
        .filter(|extension| extension.len() <= 12)?
        .to_ascii_lowercase();
    let mime = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "txt" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "pdf" => "application/pdf",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "rs" => "text/rust",
        "py" => "text/x-python",
        "js" | "mjs" | "cjs" => "text/javascript",
        "ts" | "tsx" => "text/typescript",
        "css" => "text/css",
        "xml" => "application/xml",
        _ => return None,
    };
    Some(mime.into())
}

fn local_attachment_data_url(
    value: &str,
    workspace: &Path,
    fallback_mime: Option<&str>,
) -> Result<Option<(String, Option<String>)>> {
    if value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("data:")
        || value.starts_with("file_id:")
    {
        return Ok(None);
    }
    let Some(path) = local_attachment_path(value, workspace)? else {
        return Ok(None);
    };
    let metadata = fs::metadata(&path)
        .with_context(|| format!("failed to inspect attachment `{}`", path.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!(
            "attachment `{}` is not a regular file",
            path.display()
        ));
    }
    if metadata.len() > MAX_INLINE_ATTACHMENT_BYTES {
        return Err(anyhow!(
            "attachment `{}` is too large: {} bytes exceeds {} bytes",
            path.display(),
            metadata.len(),
            MAX_INLINE_ATTACHMENT_BYTES
        ));
    }
    let bytes = fs::read(&path)
        .with_context(|| format!("failed to read attachment `{}`", path.display()))?;
    let mime = fallback_mime
        .map(str::to_owned)
        .or_else(|| guess_attachment_mime_type(&path.display().to_string()))
        .unwrap_or_else(|| "application/octet-stream".into());
    let encoded = STANDARD.encode(bytes);
    Ok(Some((format!("data:{mime};base64,{encoded}"), Some(mime))))
}

fn local_attachment_path(value: &str, workspace: &Path) -> Result<Option<PathBuf>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let raw_path = value.strip_prefix("file://").unwrap_or(value);
    if looks_like_url_with_scheme(raw_path) {
        return Ok(None);
    }
    let path = PathBuf::from(raw_path);
    let path = if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    };
    let canonical_workspace = workspace
        .canonicalize()
        .with_context(|| format!("failed to canonicalize workspace `{}`", workspace.display()))?;
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("attachment path does not exist: `{}`", path.display()))?;
    if !canonical_path.starts_with(&canonical_workspace) {
        return Err(anyhow!(
            "attachment `{}` is outside workspace `{}`",
            canonical_path.display(),
            canonical_workspace.display()
        ));
    }
    Ok(Some(canonical_path))
}

fn looks_like_url_with_scheme(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    !scheme.is_empty()
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}
