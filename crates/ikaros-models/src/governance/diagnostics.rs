// SPDX-License-Identifier: GPL-3.0-only

use super::policy::{ProviderRetryDelay, provider_error_kind_label, provider_model_id};
use crate::types::{
    ModelContentBlock, ModelProvider, ModelRequest, ModelRequestDiagnostic, ProviderErrorKind,
};
use ikaros_core::{IkarosError, redact_secrets};

pub(super) fn provider_retry_failed_diagnostic(
    provider: &dyn ModelProvider,
    attempt: u32,
    kind: ProviderErrorKind,
    error: &IkarosError,
    retry_delay: ProviderRetryDelay,
) -> ModelRequestDiagnostic {
    let retry_after = retry_delay
        .retry_after_ms
        .map(|value| format!(" retry_after_ms={value}"))
        .unwrap_or_default();
    ModelRequestDiagnostic::new(
        "provider_retry_failed",
        format!(
            "provider {}/{} retry attempt {} failed with {} error: {}; base_retry_delay_ms={} jitter_ms={} retry_delay_ms={}{}",
            redact_secrets(provider.name()),
            redact_secrets(provider_model_id(provider)),
            attempt,
            provider_error_kind_label(kind),
            redact_secrets(&error.to_string()),
            retry_delay.base_delay_ms,
            retry_delay.jitter_ms,
            retry_delay.delay_ms,
            retry_after
        ),
        None,
    )
}

pub(super) fn provider_retry_succeeded_diagnostic(
    provider: &str,
    model: &str,
    attempt_count: u32,
) -> ModelRequestDiagnostic {
    ModelRequestDiagnostic::new(
        "provider_retry_succeeded",
        format!(
            "provider {}/{} succeeded after {} retry attempt(s)",
            redact_secrets(provider),
            redact_secrets(model),
            attempt_count
        ),
        None,
    )
}

pub(super) fn fallback_provider_failed_diagnostic(
    provider: &dyn ModelProvider,
    kind: ProviderErrorKind,
    error: &IkarosError,
) -> ModelRequestDiagnostic {
    ModelRequestDiagnostic::new(
        "fallback_provider_failed",
        format!(
            "fallback provider {}/{} failed with {} error; trying next provider: {}",
            redact_secrets(provider.name()),
            redact_secrets(provider_model_id(provider)),
            provider_error_kind_label(kind),
            redact_secrets(&error.to_string())
        ),
        None,
    )
}

pub(super) fn fallback_provider_skipped_diagnostic(
    provider: &dyn ModelProvider,
    error: &IkarosError,
) -> ModelRequestDiagnostic {
    ModelRequestDiagnostic::new(
        "fallback_provider_skipped",
        format!(
            "fallback provider {}/{} skipped before request: {}",
            redact_secrets(provider.name()),
            redact_secrets(provider_model_id(provider)),
            redact_secrets(&error.to_string())
        ),
        None,
    )
}

pub(super) fn unsupported_content_blocks_error(
    provider: &dyn ModelProvider,
    request: &ModelRequest,
) -> Option<IkarosError> {
    let capabilities = provider.capabilities();
    let mut unsupported = Vec::new();
    if request_has_content_block(request, |block| {
        matches!(block, ModelContentBlock::Image { .. })
    }) && !capabilities.image_input
    {
        unsupported.push("image");
    }
    if request_has_content_block(request, |block| {
        matches!(block, ModelContentBlock::Audio { .. })
    }) && !capabilities.audio_input
    {
        unsupported.push("audio");
    }
    if request_has_content_block(request, |block| {
        matches!(block, ModelContentBlock::File { .. })
    }) && !capabilities.file_input
    {
        unsupported.push("file");
    }
    if unsupported.is_empty() {
        return None;
    }
    Some(IkarosError::Message(format!(
        "provider {} model {} does not support {} content blocks",
        redact_secrets(provider.name()),
        redact_secrets(provider_model_id(provider)),
        unsupported.join(",")
    )))
}

fn request_has_content_block(
    request: &ModelRequest,
    matches_block: impl Fn(&ModelContentBlock) -> bool,
) -> bool {
    request
        .messages
        .iter()
        .flat_map(|message| message.content_blocks.iter())
        .any(matches_block)
}

pub(super) fn fallback_provider_selected_diagnostic(
    provider: &str,
    model: &str,
    failed_count: usize,
) -> ModelRequestDiagnostic {
    ModelRequestDiagnostic::new(
        "fallback_provider_selected",
        format!(
            "fallback provider {}/{} selected after {} retryable failure(s)",
            redact_secrets(provider),
            redact_secrets(model),
            failed_count
        ),
        None,
    )
}

pub(super) fn sanitize_model_request_diagnostics(
    diagnostics: Vec<ModelRequestDiagnostic>,
) -> Vec<ModelRequestDiagnostic> {
    diagnostics
        .into_iter()
        .map(ModelRequestDiagnostic::sanitized)
        .collect()
}
