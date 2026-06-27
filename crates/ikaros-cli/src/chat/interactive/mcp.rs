// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use crate::chat::workbench::{mcp_status_human_lines, print_mcp_status};
use crate::mcp::run_mcp_http_call;
use anyhow::{Context, Result, anyhow};
use ikaros_core::{redact_json, redact_secrets};
use ikaros_terminal::terminal_inline;
use serde_json::json;

use super::{InteractiveCommandContext, append_workbench_evidence, print_default_inline_lines};

struct InteractiveMcpHttpCall {
    url: String,
    tool: String,
    arguments_json: String,
    max_response_bytes: usize,
}

struct InteractiveMcpStdioCall {
    command: String,
    tool: String,
    arguments_json: String,
    args: Vec<String>,
    timeout_ms: Option<u64>,
    max_output_bytes: Option<usize>,
}

pub(super) async fn handle_mcp_command(
    args: Vec<&str>,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    match args.as_slice() {
        [] | ["status"] => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(mcp_status_human_lines(ctx.config))?;
            } else {
                print_mcp_status(ctx.config);
            }
            append_workbench_evidence(runtime, "mcp", json!({"args": args}))?;
        }
        ["call-stdio", rest @ ..] => {
            let call = parse_mcp_call_stdio_args(rest)?;
            let tool = call.tool.clone();
            let mut input = json!({
                "command": call.command,
                "args": call.args,
                "tool": tool,
                "arguments": serde_json::from_str::<serde_json::Value>(&call.arguments_json)
                    .with_context(|| "invalid --arguments-json")?,
            });
            if let Some(timeout_ms) = call.timeout_ms {
                input["timeout_ms"] = json!(timeout_ms);
            }
            if let Some(max_output_bytes) = call.max_output_bytes {
                input["max_output_bytes"] = json!(max_output_bytes);
            }
            let result = runtime
                .session
                .execute_skill(ctx.registry, "mcp_stdio_call", input)
                .await?;
            println!(
                "mcp_stdio_call_json: {}",
                serde_json::to_string(&redact_json(result.output))?
            );
            append_workbench_evidence(runtime, "mcp_stdio_call", json!({"tool": tool}))?;
        }
        ["call-http", rest @ ..] => {
            let call = parse_mcp_call_http_args(rest)?;
            let report = run_mcp_http_call(
                &runtime.session,
                &call.url,
                &call.tool,
                &call.arguments_json,
                call.max_response_bytes,
            )
            .await?;
            println!("mcp_http_call_json: {}", serde_json::to_string(&report)?);
            append_workbench_evidence(
                runtime,
                "mcp_http_call",
                json!({
                    "url": redact_secrets(&call.url),
                    "tool": redact_secrets(&call.tool),
                    "max_response_bytes": call.max_response_bytes,
                    "network_egress": true,
                }),
            )?;
        }
        ["help"] | ["--help"] => emit_mcp_usage(runtime)?,
        _ => emit_mcp_usage(runtime)?,
    }
    Ok(())
}

fn parse_mcp_call_http_args(args: &[&str]) -> Result<InteractiveMcpHttpCall> {
    if args.len() < 2 {
        return Err(anyhow!(
            "usage: /mcp call-http <url> <tool> [--arguments-json JSON] [--max-response-bytes N]"
        ));
    }
    let mut call = InteractiveMcpHttpCall {
        url: args[0].to_owned(),
        tool: args[1].to_owned(),
        arguments_json: "{}".into(),
        max_response_bytes: 64 * 1024,
    };
    let mut index = 2;
    while index < args.len() {
        match args[index] {
            "--arguments-json" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-http <url> <tool> --arguments-json JSON")
                })?;
                call.arguments_json = (*value).to_owned();
                index += 2;
            }
            "--max-response-bytes" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-http <url> <tool> --max-response-bytes N")
                })?;
                call.max_response_bytes = value
                    .parse::<usize>()
                    .with_context(|| "--max-response-bytes must be a positive integer")?;
                index += 2;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /mcp call-http <url> <tool> [--arguments-json JSON] [--max-response-bytes N]"
                ));
            }
            value => {
                return Err(anyhow!(
                    "unknown /mcp call-http argument '{}'; expected --arguments-json or --max-response-bytes",
                    terminal_inline(value)
                ));
            }
        }
    }
    Ok(call)
}

fn parse_mcp_call_stdio_args(args: &[&str]) -> Result<InteractiveMcpStdioCall> {
    if args.len() < 2 {
        return Err(anyhow!(
            "usage: /mcp call-stdio <command> <tool> [--arguments-json JSON] [--args-json JSON_ARRAY] [--timeout-ms N] [--max-output-bytes N]"
        ));
    }
    let mut call = InteractiveMcpStdioCall {
        command: args[0].to_owned(),
        tool: args[1].to_owned(),
        arguments_json: "{}".into(),
        args: Vec::new(),
        timeout_ms: None,
        max_output_bytes: None,
    };
    let mut index = 2;
    while index < args.len() {
        match args[index] {
            "--arguments-json" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-stdio <command> <tool> --arguments-json JSON")
                })?;
                call.arguments_json = (*value).to_owned();
                index += 2;
            }
            "--args-json" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-stdio <command> <tool> --args-json JSON_ARRAY")
                })?;
                call.args = serde_json::from_str::<Vec<String>>(value)
                    .with_context(|| "--args-json must be a JSON string array")?;
                index += 2;
            }
            "--timeout-ms" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-stdio <command> <tool> --timeout-ms N")
                })?;
                call.timeout_ms = Some(
                    value
                        .parse::<u64>()
                        .with_context(|| "--timeout-ms must be a positive integer")?,
                );
                index += 2;
            }
            "--max-output-bytes" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-stdio <command> <tool> --max-output-bytes N")
                })?;
                call.max_output_bytes = Some(
                    value
                        .parse::<usize>()
                        .with_context(|| "--max-output-bytes must be a positive integer")?,
                );
                index += 2;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /mcp call-stdio <command> <tool> [--arguments-json JSON] [--args-json JSON_ARRAY]"
                ));
            }
            value => {
                return Err(anyhow!(
                    "unknown /mcp call-stdio argument '{}'",
                    terminal_inline(value)
                ));
            }
        }
    }
    Ok(call)
}

fn print_mcp_usage() {
    println!("usage: /mcp status");
    println!(
        "usage: /mcp call-stdio <command> <tool> [--arguments-json JSON] [--args-json JSON_ARRAY] [--timeout-ms N] [--max-output-bytes N]"
    );
    println!("usage: /mcp call-http <url> <tool> [--arguments-json JSON] [--max-response-bytes N]");
    println!(
        "mcp_policy: HTTP calls use NetworkEgress; stdio calls use harness ProcessRunner approval boundary"
    );
}

fn emit_mcp_usage(runtime: &InteractiveChatRuntime) -> Result<()> {
    if runtime.default_inline_stdout() {
        print_default_inline_lines(vec![
            "* MCP".to_owned(),
            "  status: /mcp status".to_owned(),
            "  stdio: /mcp call-stdio <command> <tool>".to_owned(),
            "  http: /mcp call-http <url> <tool>".to_owned(),
        ])?;
    } else {
        print_mcp_usage();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_mcp_call_http_args, parse_mcp_call_stdio_args};

    #[test]
    fn parses_mcp_call_http_arguments() {
        let call = parse_mcp_call_http_args(&[
            "http://127.0.0.1:9000/mcp",
            "search",
            "--arguments-json",
            r#"{"query":"ikaros"}"#,
            "--max-response-bytes",
            "128",
        ])
        .expect("mcp call args");

        assert_eq!(call.url, "http://127.0.0.1:9000/mcp");
        assert_eq!(call.tool, "search");
        assert_eq!(call.arguments_json, r#"{"query":"ikaros"}"#);
        assert_eq!(call.max_response_bytes, 128);
    }

    #[test]
    fn parses_mcp_call_stdio_arguments() {
        let call = parse_mcp_call_stdio_args(&[
            "uvx",
            "search",
            "--arguments-json",
            r#"{"query":"ikaros"}"#,
            "--args-json",
            r#"["tool-server"]"#,
            "--timeout-ms",
            "2500",
            "--max-output-bytes",
            "2048",
        ])
        .expect("mcp stdio call args");

        assert_eq!(call.command, "uvx");
        assert_eq!(call.tool, "search");
        assert_eq!(call.arguments_json, r#"{"query":"ikaros"}"#);
        assert_eq!(call.args, ["tool-server"]);
        assert_eq!(call.timeout_ms, Some(2500));
        assert_eq!(call.max_output_bytes, Some(2048));
    }
}
