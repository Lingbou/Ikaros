// SPDX-License-Identifier: GPL-3.0-only

use super::{diagnostics, egress, policy};
use crate::{
    types::{
        ModelContextProfile, ModelProvider, ModelProviderCapabilities, ModelRequest, ModelResponse,
        ModelStream, ModelStreamEventSink,
    },
    usage::{ModelUsageLedger, ProviderHealthLedger},
};
use async_trait::async_trait;
use ikaros_core::{IkarosError, Result, redact_secrets};
use std::time::Duration;
use tokio::time::sleep;

use policy::{ModelRuntimeLimits, ProviderCooldownPolicy, ProviderRetryPolicy};

pub struct GovernedModelProvider {
    inner: Box<dyn ModelProvider>,
    ledger: ModelUsageLedger,
    health_ledger: ProviderHealthLedger,
    limits: ModelRuntimeLimits,
    retry_policy: ProviderRetryPolicy,
    cooldown_policy: ProviderCooldownPolicy,
}

pub struct FallbackModelProvider {
    providers: Vec<Box<dyn ModelProvider>>,
}

impl FallbackModelProvider {
    pub fn new(providers: Vec<Box<dyn ModelProvider>>) -> Result<Self> {
        if providers.is_empty() {
            return Err(IkarosError::Message(
                "fallback provider chain must contain at least one provider".into(),
            ));
        }
        Ok(Self { providers })
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    async fn stream_with_optional_events(
        &self,
        request: ModelRequest,
        mut event_sink: Option<&mut dyn ModelStreamEventSink>,
    ) -> Result<ModelStream> {
        let mut last_error = None;
        let mut diagnostics = Vec::new();
        for provider in &self.providers {
            if let Some(error) =
                diagnostics::unsupported_content_blocks_error(provider.as_ref(), &request)
            {
                tracing::warn!(
                    target: "ikaros.models",
                    event = "fallback_provider_skipped",
                    mode = "stream",
                    provider = %redact_secrets(provider.name()),
                    model = %redact_secrets(policy::provider_model_id(provider.as_ref())),
                    error = %redact_secrets(&error.to_string()),
                );
                diagnostics.push(diagnostics::fallback_provider_skipped_diagnostic(
                    provider.as_ref(),
                    &error,
                ));
                last_error = Some(error);
                continue;
            }
            let result = if let Some(sink) = event_sink.as_deref_mut() {
                provider.stream_with_events(request.clone(), sink).await
            } else {
                provider.stream(request.clone()).await
            };
            match result {
                Ok(mut stream) => {
                    let stream_diagnostics = diagnostics::sanitize_model_request_diagnostics(
                        std::mem::take(&mut stream.diagnostics),
                    );
                    if !diagnostics.is_empty() {
                        let failed_count = diagnostics.len();
                        tracing::info!(
                            target: "ikaros.models",
                            event = "fallback_provider_selected",
                            mode = "stream",
                            provider = %redact_secrets(&stream.provider),
                            model = %redact_secrets(&stream.model),
                            failed_count = failed_count,
                        );
                        diagnostics.push(diagnostics::fallback_provider_selected_diagnostic(
                            &stream.provider,
                            &stream.model,
                            failed_count,
                        ));
                        diagnostics.extend(stream_diagnostics);
                        stream.diagnostics = diagnostics;
                    } else {
                        stream.diagnostics = stream_diagnostics;
                    }
                    return Ok(stream);
                }
                Err(error) => {
                    let kind = policy::classify_provider_error(&error);
                    if !kind.retryable() {
                        tracing::warn!(
                            target: "ikaros.models",
                            event = "fallback_provider_failed",
                            mode = "stream",
                            provider = %redact_secrets(provider.name()),
                            model = %redact_secrets(policy::provider_model_id(provider.as_ref())),
                            error_kind = policy::provider_error_kind_label(kind),
                            retryable = false,
                            error = %redact_secrets(&error.to_string()),
                        );
                        return Err(error);
                    }
                    tracing::warn!(
                        target: "ikaros.models",
                        event = "fallback_provider_failed",
                        mode = "stream",
                        provider = %redact_secrets(provider.name()),
                        model = %redact_secrets(policy::provider_model_id(provider.as_ref())),
                        error_kind = policy::provider_error_kind_label(kind),
                        retryable = true,
                        error = %redact_secrets(&error.to_string()),
                    );
                    diagnostics.push(diagnostics::fallback_provider_failed_diagnostic(
                        provider.as_ref(),
                        kind,
                        &error,
                    ));
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            IkarosError::Message("fallback provider chain failed without an error".into())
        }))
    }
}

impl GovernedModelProvider {
    pub fn new(
        inner: Box<dyn ModelProvider>,
        ledger: ModelUsageLedger,
        limits: ModelRuntimeLimits,
    ) -> Self {
        Self::new_with_retry_policy(inner, ledger, limits, ProviderRetryPolicy::default())
    }

    pub fn new_with_retry_policy(
        inner: Box<dyn ModelProvider>,
        ledger: ModelUsageLedger,
        limits: ModelRuntimeLimits,
        retry_policy: ProviderRetryPolicy,
    ) -> Self {
        let health_ledger = ProviderHealthLedger::for_usage_ledger(&ledger);
        Self {
            inner,
            ledger,
            health_ledger,
            limits,
            retry_policy,
            cooldown_policy: ProviderCooldownPolicy::default(),
        }
    }

    pub fn with_cooldown_policy(mut self, cooldown_policy: ProviderCooldownPolicy) -> Self {
        self.cooldown_policy = cooldown_policy;
        self
    }

    pub fn ledger(&self) -> &ModelUsageLedger {
        &self.ledger
    }

    pub fn retry_policy(&self) -> ProviderRetryPolicy {
        self.retry_policy
    }

    pub fn health_ledger(&self) -> &ProviderHealthLedger {
        &self.health_ledger
    }

    fn enforce_preflight(&self, request: &ModelRequest) -> Result<String> {
        egress::enforce_preflight(
            self.inner.as_ref(),
            &self.ledger,
            &self.health_ledger,
            &self.limits,
            &self.cooldown_policy,
            request,
        )
    }

    fn record_usage(
        &self,
        requested_at: String,
        estimate: u32,
        provider: &str,
        model: &str,
        usage: &crate::types::TokenUsage,
    ) -> Result<()> {
        egress::record_usage(&self.ledger, requested_at, estimate, provider, model, usage)
    }

    fn record_provider_success(&self, at: &str, provider: &str, model: &str) -> Result<()> {
        egress::record_provider_success(&self.health_ledger, at, provider, model)
    }

    fn record_provider_failure(
        &self,
        at: &str,
        kind: crate::types::ProviderErrorKind,
        summary: &str,
    ) -> Result<()> {
        egress::record_provider_failure(
            self.inner.as_ref(),
            &self.health_ledger,
            &self.cooldown_policy,
            at,
            kind,
            summary,
        )
    }

    async fn stream_with_optional_events(
        &self,
        request: ModelRequest,
        mut event_sink: Option<&mut dyn ModelStreamEventSink>,
    ) -> Result<ModelStream> {
        let request = request.redacted();
        let requested_at = self.enforce_preflight(&request)?;
        let estimate = self.inner.estimate_request_tokens(&request);
        let provider_name = redact_secrets(self.inner.name());
        let provider_model = redact_secrets(policy::provider_model_id(self.inner.as_ref()));
        tracing::info!(
            target: "ikaros.models",
            event = "model_request_start",
            mode = "stream",
            provider = %provider_name,
            model = %provider_model,
            estimated_tokens = estimate,
        );
        let mut attempt = 0;
        let mut diagnostics = Vec::new();
        loop {
            let result = if let Some(sink) = event_sink.as_deref_mut() {
                self.inner.stream_with_events(request.clone(), sink).await
            } else {
                self.inner.stream(request.clone()).await
            };
            match result {
                Ok(mut stream) => {
                    self.record_provider_success(&requested_at, &stream.provider, &stream.model)?;
                    self.record_usage(
                        requested_at,
                        estimate,
                        &stream.provider,
                        &stream.model,
                        &stream.usage,
                    )?;
                    let stream_diagnostics = diagnostics::sanitize_model_request_diagnostics(
                        std::mem::take(&mut stream.diagnostics),
                    );
                    if !diagnostics.is_empty() {
                        diagnostics.push(diagnostics::provider_retry_succeeded_diagnostic(
                            &stream.provider,
                            &stream.model,
                            attempt,
                        ));
                        diagnostics.extend(stream_diagnostics);
                        stream.diagnostics = diagnostics;
                    } else {
                        stream.diagnostics = stream_diagnostics;
                    }
                    tracing::info!(
                        target: "ikaros.models",
                        event = "model_request_complete",
                        mode = "stream",
                        provider = %redact_secrets(&stream.provider),
                        model = %redact_secrets(&stream.model),
                        attempt_count = attempt,
                        diagnostic_count = stream.diagnostics.len(),
                    );
                    return Ok(stream);
                }
                Err(error) => {
                    let kind = policy::classify_provider_error(&error);
                    if attempt >= self.retry_policy.max_retries || !kind.retryable() {
                        self.record_provider_failure(&requested_at, kind, &error.to_string())?;
                        tracing::warn!(
                            target: "ikaros.models",
                            event = "model_request_failed",
                            mode = "stream",
                            provider = %provider_name,
                            model = %provider_model,
                            attempt_count = attempt,
                            error_kind = policy::provider_error_kind_label(kind),
                            retryable = kind.retryable(),
                            error = %redact_secrets(&error.to_string()),
                        );
                        return Err(error);
                    }
                    attempt += 1;
                    let retry_delay =
                        policy::retry_delay_for_error(&self.retry_policy, attempt, &error);
                    tracing::warn!(
                        target: "ikaros.models",
                        event = "provider_retry_failed",
                        mode = "stream",
                        provider = %provider_name,
                        model = %provider_model,
                        attempt = attempt,
                        error_kind = policy::provider_error_kind_label(kind),
                        retry_delay_ms = retry_delay.delay_ms,
                        base_retry_delay_ms = retry_delay.base_delay_ms,
                        jitter_ms = retry_delay.jitter_ms,
                        retry_after_ms = retry_delay.retry_after_ms,
                        error = %redact_secrets(&error.to_string()),
                    );
                    diagnostics.push(diagnostics::provider_retry_failed_diagnostic(
                        self.inner.as_ref(),
                        attempt,
                        kind,
                        &error,
                        retry_delay,
                    ));
                    if retry_delay.delay_ms > 0 {
                        sleep(Duration::from_millis(retry_delay.delay_ms)).await;
                    }
                }
            }
        }
    }
}

#[async_trait]
impl ModelProvider for GovernedModelProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn estimate_request_tokens(&self, request: &ModelRequest) -> u32 {
        self.inner.estimate_request_tokens(request)
    }

    fn context_profile(&self) -> ModelContextProfile {
        self.inner.context_profile()
    }

    fn capabilities(&self) -> ModelProviderCapabilities {
        self.inner.capabilities()
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let request = request.redacted();
        let requested_at = self.enforce_preflight(&request)?;
        let estimate = self.inner.estimate_request_tokens(&request);
        let provider_name = redact_secrets(self.inner.name());
        let provider_model = redact_secrets(policy::provider_model_id(self.inner.as_ref()));
        tracing::info!(
            target: "ikaros.models",
            event = "model_request_start",
            mode = "generate",
            provider = %provider_name,
            model = %provider_model,
            estimated_tokens = estimate,
        );
        let mut attempt = 0;
        let mut diagnostics = Vec::new();
        loop {
            match self.inner.generate(request.clone()).await {
                Ok(mut response) => {
                    self.record_provider_success(
                        &requested_at,
                        &response.provider,
                        &response.model,
                    )?;
                    self.record_usage(
                        requested_at,
                        estimate,
                        &response.provider,
                        &response.model,
                        &response.usage,
                    )?;
                    let response_diagnostics = diagnostics::sanitize_model_request_diagnostics(
                        std::mem::take(&mut response.diagnostics),
                    );
                    if !diagnostics.is_empty() {
                        diagnostics.push(diagnostics::provider_retry_succeeded_diagnostic(
                            &response.provider,
                            &response.model,
                            attempt,
                        ));
                        diagnostics.extend(response_diagnostics);
                        response.diagnostics = diagnostics;
                    } else {
                        response.diagnostics = response_diagnostics;
                    }
                    tracing::info!(
                        target: "ikaros.models",
                        event = "model_request_complete",
                        mode = "generate",
                        provider = %redact_secrets(&response.provider),
                        model = %redact_secrets(&response.model),
                        attempt_count = attempt,
                        diagnostic_count = response.diagnostics.len(),
                    );
                    return Ok(response);
                }
                Err(error) => {
                    let kind = policy::classify_provider_error(&error);
                    if attempt >= self.retry_policy.max_retries || !kind.retryable() {
                        self.record_provider_failure(&requested_at, kind, &error.to_string())?;
                        tracing::warn!(
                            target: "ikaros.models",
                            event = "model_request_failed",
                            mode = "generate",
                            provider = %provider_name,
                            model = %provider_model,
                            attempt_count = attempt,
                            error_kind = policy::provider_error_kind_label(kind),
                            retryable = kind.retryable(),
                            error = %redact_secrets(&error.to_string()),
                        );
                        return Err(error);
                    }
                    attempt += 1;
                    let retry_delay =
                        policy::retry_delay_for_error(&self.retry_policy, attempt, &error);
                    tracing::warn!(
                        target: "ikaros.models",
                        event = "provider_retry_failed",
                        mode = "generate",
                        provider = %provider_name,
                        model = %provider_model,
                        attempt = attempt,
                        error_kind = policy::provider_error_kind_label(kind),
                        retry_delay_ms = retry_delay.delay_ms,
                        base_retry_delay_ms = retry_delay.base_delay_ms,
                        jitter_ms = retry_delay.jitter_ms,
                        retry_after_ms = retry_delay.retry_after_ms,
                        error = %redact_secrets(&error.to_string()),
                    );
                    diagnostics.push(diagnostics::provider_retry_failed_diagnostic(
                        self.inner.as_ref(),
                        attempt,
                        kind,
                        &error,
                        retry_delay,
                    ));
                    if retry_delay.delay_ms > 0 {
                        sleep(Duration::from_millis(retry_delay.delay_ms)).await;
                    }
                }
            }
        }
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        self.stream_with_optional_events(request, None).await
    }

    async fn stream_with_events(
        &self,
        request: ModelRequest,
        event_sink: &mut dyn ModelStreamEventSink,
    ) -> Result<ModelStream> {
        self.stream_with_optional_events(request, Some(event_sink))
            .await
    }
}

#[async_trait]
impl ModelProvider for FallbackModelProvider {
    fn name(&self) -> &str {
        "fallback-chain"
    }

    fn model_id(&self) -> &str {
        self.providers
            .first()
            .map(|provider| provider.model_id())
            .unwrap_or_default()
    }

    fn estimate_request_tokens(&self, request: &ModelRequest) -> u32 {
        self.providers
            .first()
            .map(|provider| provider.estimate_request_tokens(request))
            .unwrap_or_else(|| request.estimated_tokens())
    }

    fn context_profile(&self) -> ModelContextProfile {
        self.providers
            .first()
            .map(|provider| provider.context_profile())
            .unwrap_or_default()
    }

    fn capabilities(&self) -> ModelProviderCapabilities {
        let mut capabilities = ModelProviderCapabilities::text_only();
        if self.providers.is_empty() {
            return capabilities;
        }
        capabilities.chat = false;
        for provider in &self.providers {
            let provider_capabilities = provider.capabilities();
            capabilities.chat |= provider_capabilities.chat;
            capabilities.streaming |= provider_capabilities.streaming;
            capabilities.tool_calls |= provider_capabilities.tool_calls;
            capabilities.reasoning |= provider_capabilities.reasoning;
            capabilities.json_mode |= provider_capabilities.json_mode;
            capabilities.network |= provider_capabilities.network;
            capabilities.image_input |= provider_capabilities.image_input;
            capabilities.audio_input |= provider_capabilities.audio_input;
            capabilities.file_input |= provider_capabilities.file_input;
        }
        capabilities
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let mut last_error = None;
        let mut diagnostics = Vec::new();
        for provider in &self.providers {
            if let Some(error) =
                diagnostics::unsupported_content_blocks_error(provider.as_ref(), &request)
            {
                tracing::warn!(
                    target: "ikaros.models",
                    event = "fallback_provider_skipped",
                    mode = "generate",
                    provider = %redact_secrets(provider.name()),
                    model = %redact_secrets(policy::provider_model_id(provider.as_ref())),
                    error = %redact_secrets(&error.to_string()),
                );
                diagnostics.push(diagnostics::fallback_provider_skipped_diagnostic(
                    provider.as_ref(),
                    &error,
                ));
                last_error = Some(error);
                continue;
            }
            match provider.generate(request.clone()).await {
                Ok(mut response) => {
                    let response_diagnostics = diagnostics::sanitize_model_request_diagnostics(
                        std::mem::take(&mut response.diagnostics),
                    );
                    if !diagnostics.is_empty() {
                        let failed_count = diagnostics.len();
                        tracing::info!(
                            target: "ikaros.models",
                            event = "fallback_provider_selected",
                            mode = "generate",
                            provider = %redact_secrets(&response.provider),
                            model = %redact_secrets(&response.model),
                            failed_count = failed_count,
                        );
                        diagnostics.push(diagnostics::fallback_provider_selected_diagnostic(
                            &response.provider,
                            &response.model,
                            failed_count,
                        ));
                        diagnostics.extend(response_diagnostics);
                        response.diagnostics = diagnostics;
                    } else {
                        response.diagnostics = response_diagnostics;
                    }
                    return Ok(response);
                }
                Err(error) => {
                    let kind = policy::classify_provider_error(&error);
                    if !kind.retryable() {
                        tracing::warn!(
                            target: "ikaros.models",
                            event = "fallback_provider_failed",
                            mode = "generate",
                            provider = %redact_secrets(provider.name()),
                            model = %redact_secrets(policy::provider_model_id(provider.as_ref())),
                            error_kind = policy::provider_error_kind_label(kind),
                            retryable = false,
                            error = %redact_secrets(&error.to_string()),
                        );
                        return Err(error);
                    }
                    tracing::warn!(
                        target: "ikaros.models",
                        event = "fallback_provider_failed",
                        mode = "generate",
                        provider = %redact_secrets(provider.name()),
                        model = %redact_secrets(policy::provider_model_id(provider.as_ref())),
                        error_kind = policy::provider_error_kind_label(kind),
                        retryable = true,
                        error = %redact_secrets(&error.to_string()),
                    );
                    diagnostics.push(diagnostics::fallback_provider_failed_diagnostic(
                        provider.as_ref(),
                        kind,
                        &error,
                    ));
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            IkarosError::Message("fallback provider chain failed without an error".into())
        }))
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        self.stream_with_optional_events(request, None).await
    }

    async fn stream_with_events(
        &self,
        request: ModelRequest,
        event_sink: &mut dyn ModelStreamEventSink,
    ) -> Result<ModelStream> {
        self.stream_with_optional_events(request, Some(event_sink))
            .await
    }
}
