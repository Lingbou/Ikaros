// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::types::ChatRunOptions;
use ikaros_core::{IkarosError, Result, redact_secrets};
use ikaros_models::{
    ModelContentBlock, ModelMessage, ModelProvider, ModelRequest, ModelResponse, ModelStream,
    ModelStreamEvent, ModelStreamEventSink,
};

pub(super) fn redacted_chat_error(error: IkarosError) -> IkarosError {
    IkarosError::Message(redact_secrets(&error.to_string()))
}

pub(super) async fn cancellable_provider_stream_with_events(
    provider: &dyn ModelProvider,
    request: ModelRequest,
    options: &ChatRunOptions,
    event_sink: &mut dyn ModelStreamEventSink,
) -> Result<ModelStream> {
    if options.cancellation.is_cancelled() {
        return Err(chat_cancelled_error(
            "provider stream was cancelled before request",
        ));
    }
    tokio::select! {
        _ = options.cancellation.cancelled() => {
            Err(chat_cancelled_error("provider stream was cancelled"))
        }
        result = provider.stream_with_events(request, event_sink) => result,
    }
}

pub(super) fn validate_content_blocks_supported(
    provider: &dyn ModelProvider,
    content_blocks: &[ModelContentBlock],
) -> Result<()> {
    if content_blocks.is_empty() {
        return Ok(());
    }
    let capabilities = provider.capabilities();
    let mut unsupported = Vec::new();
    if content_blocks
        .iter()
        .any(|block| matches!(block, ModelContentBlock::Image { .. }))
        && !capabilities.image_input
    {
        unsupported.push("image");
    }
    if content_blocks
        .iter()
        .any(|block| matches!(block, ModelContentBlock::Audio { .. }))
        && !capabilities.audio_input
    {
        unsupported.push("audio");
    }
    if content_blocks
        .iter()
        .any(|block| matches!(block, ModelContentBlock::File { .. }))
        && !capabilities.file_input
    {
        unsupported.push("file");
    }
    if unsupported.is_empty() {
        return Ok(());
    }
    Err(IkarosError::Message(format!(
        "provider {} model {} does not support {} content blocks; choose a multimodal provider/model or remove the pending attachments",
        redact_secrets(provider.name()),
        redact_secrets(provider.model_id()),
        unsupported.join(",")
    )))
}

pub(super) async fn cancellable_provider_generate(
    provider: &dyn ModelProvider,
    request: ModelRequest,
    options: &ChatRunOptions,
) -> Result<ModelResponse> {
    if options.cancellation.is_cancelled() {
        return Err(chat_cancelled_error(
            "provider request was cancelled before request",
        ));
    }
    tokio::select! {
        _ = options.cancellation.cancelled() => {
            Err(chat_cancelled_error("provider request was cancelled"))
        }
        result = provider.generate(request) => result,
    }
}

fn chat_cancelled_error(message: &str) -> IkarosError {
    IkarosError::Message(message.into())
}

#[cfg(test)]
pub(crate) fn model_messages_for_single_call(
    system_prompts: &[String],
    input: &str,
) -> Vec<ModelMessage> {
    model_messages_for_single_call_with_content_blocks(system_prompts, input, &[])
}

pub(crate) fn model_messages_for_single_call_with_content_blocks(
    system_prompts: &[String],
    input: &str,
    content_blocks: &[ModelContentBlock],
) -> Vec<ModelMessage> {
    let mut messages = system_prompts
        .iter()
        .filter(|prompt| !prompt.trim().is_empty())
        .cloned()
        .map(ModelMessage::system)
        .collect::<Vec<_>>();
    if content_blocks.is_empty() {
        messages.push(ModelMessage::user(redact_secrets(input)));
    } else {
        let mut blocks = Vec::with_capacity(content_blocks.len() + 1);
        if !input.trim().is_empty() {
            blocks.push(ModelContentBlock::text(redact_secrets(input)));
        }
        blocks.extend(
            content_blocks
                .iter()
                .cloned()
                .map(ModelContentBlock::redacted),
        );
        messages.push(ModelMessage::user_with_content_blocks(blocks));
    }
    messages
}

pub(super) fn model_response_stream_events(response: &ModelResponse) -> Vec<ModelStreamEvent> {
    let mut events = vec![ModelStreamEvent::Start {
        provider: redact_secrets(&response.provider),
        model: redact_secrets(&response.model),
    }];
    if !response.content.is_empty() {
        events.push(ModelStreamEvent::TextDelta(redact_secrets(
            &response.content,
        )));
    }
    if response.usage.total_or_prompt_completion() > 0 {
        events.push(ModelStreamEvent::Usage(response.usage.clone()));
    }
    events.push(ModelStreamEvent::Done);
    events
}
