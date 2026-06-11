// SPDX-License-Identifier: GPL-3.0-only

use super::types::ApprovalRequest;
use ikaros_core::{ToolResult, redact_json, redact_secrets};

pub(super) fn redact_approval_request(mut request: ApprovalRequest) -> ApprovalRequest {
    request.reason = redact_secrets(&request.reason);
    request.call.input = redact_json(request.call.input);
    request
}

pub(super) fn redact_approval_note(note: Option<String>) -> Option<String> {
    note.map(|note| redact_secrets(&note))
}

pub(super) fn redact_tool_result(mut result: ToolResult) -> ToolResult {
    result.summary = redact_secrets(&result.summary);
    result.output = redact_json(result.output);
    result
}
