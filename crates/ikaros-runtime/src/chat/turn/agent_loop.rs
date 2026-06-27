// SPDX-License-Identifier: GPL-3.0-only

use super::{result::ChatModelResult, setup::ChatTurnSetup};
use crate::{
    AgentEventSink, AgentHarness, AgentHarnessConfig, AgentLoopOptions, HarnessAgentRuntime,
};
use ikaros_core::Result;
use ikaros_harness::{
    CancellationToken, ExecutionSession, GuardrailConfig, SkillRegistry, ToolsetSelection,
};
use ikaros_models::{ModelProvider, ModelRequestOptions, ModelResponse};
use ikaros_session::SessionId;

pub(super) struct AgentLoopInput<'a> {
    pub(super) input: &'a str,
    pub(super) provider: &'a dyn ModelProvider,
    pub(super) session: &'a ExecutionSession,
    pub(super) registry: &'a SkillRegistry,
    pub(super) event_sink: &'a dyn AgentEventSink,
    pub(super) setup: &'a ChatTurnSetup,
    pub(super) request_options: ModelRequestOptions,
    pub(super) stream: bool,
    pub(super) cancellation: CancellationToken,
    pub(super) system_prompt: String,
    pub(super) system_prompt_messages: Vec<String>,
    pub(super) toolsets: ToolsetSelection,
}

pub(super) async fn run_agent_loop(input: AgentLoopInput<'_>) -> Result<ChatModelResult> {
    let runtime = HarnessAgentRuntime;
    let mut harness = AgentHarness::new(
        AgentHarnessConfig {
            session_id: SessionId::from(input.setup.chat_session_id.clone()),
            turn_id: Some(input.setup.turn_id.clone()),
            task_id: None,
            system_prompt: input.system_prompt,
            options: AgentLoopOptions {
                max_iterations: 4,
                request_options: input.request_options,
                stream: input.stream,
                system_prompt_messages: input.system_prompt_messages,
                guardrails: GuardrailConfig::default(),
                toolsets: input.toolsets,
                cancellation: input.cancellation,
                hooks: None,
            },
        },
        &runtime,
        input.provider,
        input.session,
        input.registry,
        input.event_sink,
    );
    let harness_turn = harness.run_turn(input.input).await?;
    let loop_report = harness_turn.report;
    Ok(ChatModelResult {
        response: ModelResponse {
            provider: loop_report.provider,
            model: loop_report.model,
            content: loop_report.final_content,
            tool_calls: Vec::new(),
            usage: loop_report.usage,
            diagnostics: Vec::new(),
        },
        streamed: loop_report.streamed,
        stream_chunks: loop_report.stream_chunks,
    })
}
