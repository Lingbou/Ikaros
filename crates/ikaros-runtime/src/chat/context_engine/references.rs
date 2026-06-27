// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::types::ChatContext;
use ikaros_context::{
    ContextReference, ContextReferenceKind, parse_context_references, resolve_context_reference,
};
use ikaros_core::{IkarosError, Result, redact_secrets};
use ikaros_harness::{ExecutionSession, NetworkEgressRequest};

pub(super) async fn assemble_reference_context(
    context: &mut ChatContext,
    input: &str,
    session: &ExecutionSession,
) -> Result<Vec<ContextReference>> {
    let references = parse_context_references(input);
    let mut resolved = Vec::with_capacity(references.len());
    for reference in &references {
        let reference_text = match &reference.kind {
            ContextReferenceKind::Url { url } => resolve_url_reference(url, session).await?,
            _ => resolve_context_reference(reference, &session.sandbox.workspace_root)
                .map_err(|error| IkarosError::Message(error.to_string()))?,
        };
        resolved.push(reference_text);
    }
    context.references = resolved;
    Ok(references)
}

async fn resolve_url_reference(url: &str, session: &ExecutionSession) -> Result<String> {
    const MAX_URL_REFERENCE_BODY_BYTES: usize = 64 * 1024;

    let response = session
        .env
        .send_network_request(NetworkEgressRequest {
            method: "GET".into(),
            url: url.into(),
            headers: Default::default(),
            body: None,
            body_bytes: None,
        })
        .await?;
    if let Some(content_type) = url_reference_content_type(&response) {
        if !url_reference_content_type_allowed(content_type) {
            return Ok(redact_secrets(&format!(
                "[reference/url] {url} status={} skipped: unsupported content-type {}",
                response.status,
                normalize_content_type(content_type)
            )));
        }
    }
    if response.body.len() > MAX_URL_REFERENCE_BODY_BYTES {
        return Ok(redact_secrets(&format!(
            "[reference/url] {url} status={} skipped: response body too large bytes={} max_bytes={MAX_URL_REFERENCE_BODY_BYTES}",
            response.status,
            response.body.len()
        )));
    }
    let body = truncate_url_reference_body(&redact_secrets(&response.body));
    Ok(redact_secrets(&format!(
        "[reference/url] {url} status={}\n{}",
        response.status, body
    )))
}

fn url_reference_content_type(response: &ikaros_harness::NetworkEgressResponse) -> Option<&str> {
    response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
}

fn url_reference_content_type_allowed(content_type: &str) -> bool {
    let content_type = normalize_content_type(content_type);
    matches!(
        content_type.as_str(),
        "application/json"
            | "application/ld+json"
            | "application/xml"
            | "application/yaml"
            | "application/x-yaml"
            | "text/markdown"
            | "text/plain"
            | "text/x-markdown"
            | "text/xml"
    )
}

fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase()
}

fn truncate_url_reference_body(body: &str) -> String {
    const MAX_CHARS: usize = 16 * 1024;
    let mut chars = body.chars();
    let mut truncated = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        truncated.push_str("\n[reference/url] truncated");
    }
    truncated
}
