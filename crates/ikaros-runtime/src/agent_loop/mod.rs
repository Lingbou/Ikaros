// SPDX-License-Identifier: GPL-3.0-only

mod dispatch;
mod model_turn;
mod prompt;
mod report;
mod runtime;
mod stream;
mod tool_parse;
mod types;

pub use prompt::agent_loop_tool_definitions;
pub use runtime::{
    AgentRuntime, HarnessAgentRuntime, RecordingAgentRuntime, run_agent_loop,
    run_agent_loop_with_events,
};
pub use types::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopHookEvent,
    AgentLoopHooks, AgentLoopInput, AgentLoopOptions, AgentLoopReport, AgentLoopStopReason,
    AgentLoopToolCall, AgentLoopToolCallDiagnostic, AgentLoopToolCallParseStrategy,
    AgentLoopToolDefinition, AgentLoopToolResult, noop_agent_event_sink,
};

#[cfg(test)]
mod tests;
