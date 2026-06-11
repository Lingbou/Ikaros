// SPDX-License-Identifier: GPL-3.0-only

mod dispatch;
mod model_turn;
mod prompt;
mod report;
mod runtime;
mod stream;
mod tool_parse;
mod tool_repair;
mod types;

pub use prompt::agent_loop_tool_definitions;
pub use runtime::{AgentRuntime, HarnessAgentRuntime, run_agent_loop};
pub use types::{
    AgentLoopInput, AgentLoopOptions, AgentLoopReport, AgentLoopStopReason, AgentLoopToolCall,
    AgentLoopToolCallDiagnostic, AgentLoopToolCallParseStrategy, AgentLoopToolDefinition,
    AgentLoopToolResult,
};

#[cfg(test)]
mod tests;
