// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, anyhow};
use ikaros_core::redact_json;
use ikaros_terminal::terminal_inline;
use serde_json::json;

use crate::chat::notice::WorkbenchNotice;

use super::{InteractiveChatRuntime, InteractiveCommandContext, print_default_inline_lines};

pub(super) async fn handle_web_command(
    args: Vec<&str>,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    let Some(command) = args.first().copied() else {
        emit_web_usage(runtime)?;
        return Ok(());
    };
    let (skill, input) = match command {
        "search" => ("web_search", parse_web_search_input(&args[1..])?),
        "extract" => ("web_extract", parse_web_extract_input(&args[1..])?),
        "help" | "--help" => {
            emit_web_usage(runtime)?;
            return Ok(());
        }
        value => {
            if runtime.fullscreen_stdout_quiet() {
                runtime.push_notice(WorkbenchNotice::error(
                    "web command",
                    &format!("unsupported command={}", terminal_inline(value)),
                ));
            } else if runtime.default_inline_stdout() {
                print_default_inline_lines(vec![
                    "* Web".to_owned(),
                    format!("  unsupported command: {}", terminal_inline(value)),
                    "  usage: /web search <query> | /web extract <url>".to_owned(),
                ])?;
            } else {
                println!(
                    "web_usage_error: unsupported command={}",
                    terminal_inline(value)
                );
                print_web_usage();
            }
            return Ok(());
        }
    };
    let result = runtime
        .session
        .execute_skill(ctx.registry, skill, input)
        .await?;
    if runtime.fullscreen_stdout_quiet() {
        runtime.push_notice(WorkbenchNotice::info(
            "web result",
            &format!(
                "skill={} ok={} summary={}",
                skill,
                result.ok,
                terminal_inline(&result.summary)
            ),
        ));
        return Ok(());
    }
    if runtime.default_inline_stdout() {
        print_default_inline_lines(vec![
            "* Web".to_owned(),
            format!("  ok: {}", result.ok),
            format!("  summary: {}", terminal_inline(&result.summary)),
        ])?;
        return Ok(());
    }
    println!(
        "web_result: ok={} summary={}",
        result.ok,
        terminal_inline(&result.summary)
    );
    println!(
        "web_json: {}",
        serde_json::to_string(&redact_json(result.output.clone()))?
    );
    Ok(())
}

fn parse_web_search_input(args: &[&str]) -> Result<serde_json::Value> {
    let mut query = Vec::new();
    let mut max_results: Option<usize> = None;
    let mut endpoint: Option<String> = None;
    let mut provider: Option<String> = None;
    let mut api_key: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--max-results" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web search <query> --max-results N"))?;
                max_results = Some(
                    value
                        .parse::<usize>()
                        .with_context(|| "--max-results must be a positive integer")?,
                );
                index += 2;
            }
            "--endpoint" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web search <query> --endpoint URL"))?;
                endpoint = Some((*value).to_owned());
                index += 2;
            }
            "--provider" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web search <query> --provider PROVIDER"))?;
                provider = Some((*value).to_owned());
                index += 2;
            }
            "--api-key" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web search <query> --api-key KEY"))?;
                api_key = Some((*value).to_owned());
                index += 2;
            }
            "help" | "--help" => {
                print_web_usage();
                index += 1;
            }
            value => {
                query.push(value);
                index += 1;
            }
        }
    }
    if query.is_empty() {
        return Err(anyhow!("usage: /web search <query> [--max-results N]"));
    }
    let mut input = json!({ "query": query.join(" ") });
    if let Some(max_results) = max_results {
        input["max_results"] = json!(max_results);
    }
    if let Some(endpoint) = endpoint {
        input["endpoint"] = json!(endpoint);
    }
    if let Some(provider) = provider {
        input["provider"] = json!(provider);
    }
    if let Some(api_key) = api_key {
        input["api_key"] = json!(api_key);
    }
    Ok(input)
}

fn parse_web_extract_input(args: &[&str]) -> Result<serde_json::Value> {
    let mut url: Option<String> = None;
    let mut max_bytes: Option<usize> = None;
    let mut max_chars: Option<usize> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--max-bytes" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web extract <url> --max-bytes N"))?;
                max_bytes = Some(
                    value
                        .parse::<usize>()
                        .with_context(|| "--max-bytes must be a positive integer")?,
                );
                index += 2;
            }
            "--max-chars" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web extract <url> --max-chars N"))?;
                max_chars = Some(
                    value
                        .parse::<usize>()
                        .with_context(|| "--max-chars must be a positive integer")?,
                );
                index += 2;
            }
            "help" | "--help" => {
                print_web_usage();
                index += 1;
            }
            value if url.is_none() => {
                url = Some(value.to_owned());
                index += 1;
            }
            value => {
                return Err(anyhow!("unexpected /web extract argument: {value}"));
            }
        }
    }
    let url = url.ok_or_else(|| anyhow!("usage: /web extract <url>"))?;
    let mut input = json!({ "url": url });
    if let Some(max_bytes) = max_bytes {
        input["max_bytes"] = json!(max_bytes);
    }
    if let Some(max_chars) = max_chars {
        input["max_chars"] = json!(max_chars);
    }
    Ok(input)
}

pub(super) fn print_web_usage() {
    println!(
        "web_usage: /web search <query> [--provider duckduckgo-html|brave|bing|serpapi|tavily] [--max-results N] [--endpoint URL] [--api-key KEY] | /web extract <url> [--max-bytes N] [--max-chars N]"
    );
}

fn emit_web_usage(runtime: &mut InteractiveChatRuntime) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        runtime.push_notice(WorkbenchNotice::info(
            "web",
            "usage is available in the fullscreen web cell",
        ));
    } else if runtime.default_inline_stdout() {
        print_default_inline_lines(web_usage_human_lines())?;
    } else {
        print_web_usage();
    }
    Ok(())
}

fn web_usage_human_lines() -> Vec<String> {
    vec![
        "* Web".to_owned(),
        "  usage: /web search <query> [--max-results N]".to_owned(),
        "  usage: /web extract <url> [--max-bytes N] [--max-chars N]".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::{parse_web_extract_input, parse_web_search_input};

    #[test]
    fn parses_web_search_input_with_provider_controls() {
        let input = parse_web_search_input(&[
            "rust",
            "agent",
            "--max-results",
            "3",
            "--provider",
            "brave",
            "--endpoint",
            "https://search.example",
            "--api-key",
            "test-key",
        ])
        .expect("web search input");

        assert_eq!(input["query"], "rust agent");
        assert_eq!(input["max_results"], 3);
        assert_eq!(input["provider"], "brave");
        assert_eq!(input["endpoint"], "https://search.example");
        assert_eq!(input["api_key"], "test-key");
    }

    #[test]
    fn parses_web_extract_input_with_limits() {
        let input = parse_web_extract_input(&[
            "https://example.com",
            "--max-bytes",
            "4096",
            "--max-chars",
            "512",
        ])
        .expect("web extract input");

        assert_eq!(input["url"], "https://example.com");
        assert_eq!(input["max_bytes"], 4096);
        assert_eq!(input["max_chars"], 512);
    }
}
