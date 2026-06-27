// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_core::{ToolResult, redact_json};

pub(super) fn output_str<'a>(result: &'a ToolResult, key: &str) -> Option<&'a str> {
    result.output.get(key).and_then(|value| value.as_str())
}

pub(super) fn output_u64(result: &ToolResult, key: &str) -> Option<u64> {
    result.output.get(key).and_then(|value| value.as_u64())
}

pub(super) fn redacted_output_json(result: &ToolResult) -> Result<String> {
    Ok(serde_json::to_string(&redact_json(result.output.clone()))?)
}

pub(super) fn redacted_value_json(value: &serde_json::Value) -> Result<String> {
    Ok(serde_json::to_string(&redact_json(value.clone()))?)
}
