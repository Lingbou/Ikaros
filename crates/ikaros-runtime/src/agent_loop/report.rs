// SPDX-License-Identifier: GPL-3.0-only

use super::types::{AgentLoopFinish, AgentLoopReport};
use ikaros_core::{Result, redact_secrets};
use ikaros_harness::{AuditEvent, ExecutionSession};
use serde_json::json;

pub(super) fn finish_agent_loop(
    session: &ExecutionSession,
    task_id: Option<String>,
    finish: AgentLoopFinish,
) -> Result<AgentLoopReport> {
    let report = AgentLoopReport {
        stop_reason: finish.stop_reason,
        final_content: redact_secrets(&finish.final_content),
        provider: redact_secrets(&finish.provider),
        model: redact_secrets(&finish.model),
        usage: finish.usage,
        streamed: finish.streamed,
        stream_chunks: finish
            .stream_chunks
            .into_iter()
            .map(|chunk| redact_secrets(&chunk))
            .collect(),
        iterations: finish.iterations,
        tool_call_diagnostics: finish.tool_call_diagnostics,
        tool_results: finish.tool_results,
        events: finish.events,
    };
    session
        .audit
        .append(session.correlate_audit_event(AuditEvent::new(
            "agent_loop_end",
            None,
            format!("agent loop ended: {:?}", report.stop_reason),
            json!({
                "correlation_id": session.correlation_id(),
                "task_id": task_id,
                "stop_reason": &report.stop_reason,
                "provider": &report.provider,
                "model": &report.model,
                "streamed": report.streamed,
                "stream_chunk_count": report.stream_chunks.len(),
                "iterations": report.iterations,
                "tool_call_diagnostics": &report.tool_call_diagnostics,
                "tool_result_count": report.tool_results.len(),
                "event_count": report.events.len(),
                "final_content_chars": report.final_content.chars().count(),
            }),
        )?))?;
    Ok(report)
}
