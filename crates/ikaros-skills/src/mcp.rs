// SPDX-License-Identifier: GPL-3.0-only

use async_trait::async_trait;
use ikaros_core::{IkarosError, Result, RiskLevel, redact_json, redact_secrets};
use ikaros_tools::{PolicyRequest, ProcessRequest, Skill, SkillContext, SkillOutput};
use serde_json::{Value, json};
use std::path::Path;

const DEFAULT_MCP_STDIO_PROBE_TIMEOUT_MS: u64 = 5_000;
const MAX_MCP_STDIO_PROBE_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MCP_STDIO_PROBE_MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_MCP_STDIO_PROBE_MAX_OUTPUT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct McpStdioProbeSkill;

#[derive(Debug, Clone)]
pub struct McpStdioCallSkill;

#[async_trait]
impl Skill for McpStdioProbeSkill {
    fn name(&self) -> &'static str {
        "mcp_stdio_probe"
    }

    fn description(&self) -> &'static str {
        "Probe a stdio MCP server through the harness-managed process boundary."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": {"type": "string"},
                "args": {"type": "array", "items": {"type": "string"}},
                "include_tools": {"type": "array", "items": {"type": "string"}},
                "exclude_tools": {"type": "array", "items": {"type": "string"}},
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_MCP_STDIO_PROBE_TIMEOUT_MS
                },
                "max_output_bytes": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_MCP_STDIO_PROBE_MAX_OUTPUT_BYTES
                }
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::ShellWrite
    }

    fn policy_request(&self, input: &Value, _workspace_root: &Path) -> PolicyRequest {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let args = parse_args(input).unwrap_or_default();
        PolicyRequest {
            action: self.name().into(),
            risk: self.risk_level(),
            path: None,
            command: Some(command_display(command, &args)),
            is_write: true,
        }
    }

    async fn execute(&self, input: Value, ctx: SkillContext) -> Result<SkillOutput> {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| IkarosError::Message("command is required".into()))?;
        validate_program(&command)?;
        let args = parse_args(&input)?;
        let timeout_ms = bounded_u64(
            &input,
            "timeout_ms",
            DEFAULT_MCP_STDIO_PROBE_TIMEOUT_MS,
            MAX_MCP_STDIO_PROBE_TIMEOUT_MS,
        )?;
        let max_output_bytes = bounded_usize(
            &input,
            "max_output_bytes",
            DEFAULT_MCP_STDIO_PROBE_MAX_OUTPUT_BYTES,
            MAX_MCP_STDIO_PROBE_MAX_OUTPUT_BYTES,
        )?;
        let stdin = ikaros_mcp::mcp_stdio_probe_input()?;
        let output = ctx
            .session
            .env
            .run_process(
                ProcessRequest::program(
                    command.clone(),
                    args.clone(),
                    &ctx.session.sandbox.workspace_root,
                )
                .with_stdin(stdin)
                .with_timeout_ms(timeout_ms)
                .with_max_output_bytes(max_output_bytes),
            )
            .await?;
        let include_tools = parse_optional_string_array(&input, "include_tools")?;
        let exclude_tools = parse_optional_string_array(&input, "exclude_tools")?;
        let mut report = ikaros_mcp::parse_mcp_stdio_probe_output(&output.stdout);
        filter_probe_tools(&mut report.tools, &include_tools, &exclude_tools);
        let ok = output.status == 0 && report.errors.is_empty() && report.server_info.is_some();
        let tool_count = report.tools.len();
        Ok(SkillOutput::new(
            if ok {
                format!("MCP stdio probe completed: {tool_count} tools")
            } else {
                format!("MCP stdio probe finished with issues: {tool_count} tools")
            },
            json!({
                "ok": ok,
                "command": redact_secrets(&command_display(&command, &args)),
                "status": output.status,
                "timeout_ms": timeout_ms,
                "max_output_bytes": max_output_bytes,
                "tool_filter": {
                    "include_tools": include_tools,
                    "exclude_tools": exclude_tools,
                    "visible_tools": tool_count,
                },
                "probe": report,
                "stderr": redact_secrets(&output.stderr),
            }),
        ))
    }
}

#[async_trait]
impl Skill for McpStdioCallSkill {
    fn name(&self) -> &'static str {
        "mcp_stdio_call"
    }

    fn description(&self) -> &'static str {
        "Call a tool on a stdio MCP server through the harness-managed process boundary."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["command", "tool"],
            "properties": {
                "command": {"type": "string"},
                "args": {"type": "array", "items": {"type": "string"}},
                "tool": {"type": "string"},
                "arguments": {"type": "object"},
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_MCP_STDIO_PROBE_TIMEOUT_MS
                },
                "max_output_bytes": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_MCP_STDIO_PROBE_MAX_OUTPUT_BYTES
                }
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::ShellWrite
    }

    fn policy_request(&self, input: &Value, _workspace_root: &Path) -> PolicyRequest {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let args = parse_args(input).unwrap_or_default();
        let tool = input
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        PolicyRequest {
            action: self.name().into(),
            risk: self.risk_level(),
            path: None,
            command: Some(format!(
                "{} tool={}",
                command_display(command, &args),
                redact_secrets(tool)
            )),
            is_write: true,
        }
    }

    async fn execute(&self, input: Value, ctx: SkillContext) -> Result<SkillOutput> {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| IkarosError::Message("command is required".into()))?;
        validate_program(&command)?;
        let args = parse_args(&input)?;
        let tool = input
            .get("tool")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| IkarosError::Message("tool is required".into()))?;
        let arguments = input.get("arguments").cloned().unwrap_or_else(|| json!({}));
        if !arguments.is_object() {
            return Err(IkarosError::Message(
                "mcp_stdio_call arguments must be an object".into(),
            ));
        }
        let timeout_ms = bounded_u64(
            &input,
            "timeout_ms",
            DEFAULT_MCP_STDIO_PROBE_TIMEOUT_MS,
            MAX_MCP_STDIO_PROBE_TIMEOUT_MS,
        )?;
        let max_output_bytes = bounded_usize(
            &input,
            "max_output_bytes",
            DEFAULT_MCP_STDIO_PROBE_MAX_OUTPUT_BYTES,
            MAX_MCP_STDIO_PROBE_MAX_OUTPUT_BYTES,
        )?;
        let stdin = mcp_stdio_call_input(&tool, arguments)?;
        let output = ctx
            .session
            .env
            .run_process(
                ProcessRequest::program(
                    command.clone(),
                    args.clone(),
                    &ctx.session.sandbox.workspace_root,
                )
                .with_stdin(stdin)
                .with_timeout_ms(timeout_ms)
                .with_max_output_bytes(max_output_bytes),
            )
            .await?;
        let call = parse_mcp_stdio_call_output(&output.stdout);
        let ok = output.status == 0 && call.get("error").is_none_or(|error| error.is_null());
        Ok(SkillOutput::new(
            if ok {
                format!("MCP stdio tool call completed: {}", redact_secrets(&tool))
            } else {
                format!(
                    "MCP stdio tool call returned an error: {}",
                    redact_secrets(&tool)
                )
            },
            json!({
                "ok": ok,
                "command": redact_secrets(&command_display(&command, &args)),
                "tool": redact_secrets(&tool),
                "status": output.status,
                "timeout_ms": timeout_ms,
                "max_output_bytes": max_output_bytes,
                "response": call,
                "stderr": redact_secrets(&output.stderr),
            }),
        ))
    }
}

fn validate_program(command: &str) -> Result<()> {
    if command
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '|' | '&' | ';' | '<' | '>'))
    {
        return Err(IkarosError::Message(
            "command must be a program name/path, not a shell expression".into(),
        ));
    }
    Ok(())
}

fn mcp_stdio_call_input(tool: &str, arguments: Value) -> Result<String> {
    let requests = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "ikaros",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": tool,
                "arguments": arguments,
            }
        }),
    ];
    let mut input = String::new();
    for request in requests {
        input.push_str(&serde_json::to_string(&request)?);
        input.push('\n');
    }
    Ok(input)
}

fn parse_mcp_stdio_call_output(stdout: &str) -> Value {
    let mut last = json!({
        "error": {
            "code": -32603,
            "message": "MCP server returned no tools/call response"
        }
    });
    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let value = match serde_json::from_str::<Value>(line) {
            Ok(value) => redact_json(value),
            Err(error) => {
                last = json!({
                    "error": {
                        "code": -32700,
                        "message": redact_secrets(&format!("invalid JSON-RPC response: {error}")),
                    }
                });
                continue;
            }
        };
        if value.get("id").and_then(Value::as_i64) == Some(2) {
            return value;
        }
        last = value;
    }
    last
}

fn parse_args(input: &Value) -> Result<Vec<String>> {
    let Some(args) = input.get("args") else {
        return Ok(Vec::new());
    };
    let args = args
        .as_array()
        .ok_or_else(|| IkarosError::Message("args must be an array of strings".into()))?;
    args.iter()
        .map(|arg| {
            arg.as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| IkarosError::Message("args must be an array of strings".into()))
        })
        .collect()
}

fn parse_optional_string_array(input: &Value, key: &str) -> Result<Vec<String>> {
    let Some(values) = input.get(key) else {
        return Ok(Vec::new());
    };
    let values = values
        .as_array()
        .ok_or_else(|| IkarosError::Message(format!("{key} must be an array of strings")))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| IkarosError::Message(format!("{key} must be an array of strings")))
        })
        .collect()
}

fn filter_probe_tools(
    tools: &mut Vec<ikaros_mcp::McpTool>,
    include_tools: &[String],
    exclude_tools: &[String],
) {
    if !include_tools.is_empty() {
        tools.retain(|tool| include_tools.iter().any(|include| include == &tool.name));
    }
    if !exclude_tools.is_empty() {
        tools.retain(|tool| !exclude_tools.iter().any(|exclude| exclude == &tool.name));
    }
}

fn bounded_u64(input: &Value, key: &str, default: u64, max: u64) -> Result<u64> {
    let Some(value) = input.get(key) else {
        return Ok(default);
    };
    let value = value
        .as_u64()
        .ok_or_else(|| IkarosError::Message(format!("{key} must be a positive integer")))?;
    if value == 0 || value > max {
        return Err(IkarosError::Message(format!(
            "{key} must be between 1 and {max}"
        )));
    }
    Ok(value)
}

fn bounded_usize(input: &Value, key: &str, default: usize, max: usize) -> Result<usize> {
    let value = bounded_u64(input, key, default as u64, max as u64)?;
    usize::try_from(value).map_err(|_| IkarosError::Message(format!("{key} is too large")))
}

fn command_display(command: &str, args: &[String]) -> String {
    std::iter::once(command.to_owned())
        .chain(args.iter().cloned())
        .map(|part| redact_secrets(&part))
        .collect::<Vec<_>>()
        .join(" ")
}
