// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::types::{
    AgentLoopFinish, AgentLoopStopReason, AgentLoopToolCallDiagnostic, AgentLoopToolResult,
};
use ikaros_models::TokenUsage;

#[derive(Debug, Clone)]
pub(super) struct AgentLoopTurnState {
    pub(super) last_content: String,
    pub(super) last_provider: String,
    pub(super) last_model: String,
    pub(super) total_usage: TokenUsage,
    pub(super) final_streamed: bool,
    pub(super) final_stream_chunks: Vec<String>,
    pub(super) tool_call_diagnostics: Vec<AgentLoopToolCallDiagnostic>,
    pub(super) tool_results: Vec<AgentLoopToolResult>,
}

impl AgentLoopTurnState {
    pub(super) fn new(provider: impl Into<String>) -> Self {
        Self {
            last_content: String::new(),
            last_provider: provider.into(),
            last_model: String::new(),
            total_usage: TokenUsage::default(),
            final_streamed: false,
            final_stream_chunks: Vec::new(),
            tool_call_diagnostics: Vec::new(),
            tool_results: Vec::new(),
        }
    }

    pub(super) fn finish(
        self,
        stop_reason: AgentLoopStopReason,
        iterations: u32,
    ) -> AgentLoopFinish {
        AgentLoopFinish {
            stop_reason,
            final_content: self.last_content,
            provider: self.last_provider,
            model: self.last_model,
            usage: self.total_usage,
            streamed: self.final_streamed,
            stream_chunks: self.final_stream_chunks,
            iterations,
            tool_call_diagnostics: self.tool_call_diagnostics,
            tool_results: self.tool_results,
            events: Vec::new(),
        }
    }
}
