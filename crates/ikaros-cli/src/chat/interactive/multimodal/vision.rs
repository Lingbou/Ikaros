// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::notice::WorkbenchNotice;
use crate::vision::VisionDescribeArgs;
use anyhow::{Result, anyhow};
use ikaros_terminal::{
    render_terminal_markdown_for_current_width as render_terminal_markdown, terminal_inline,
};
use serde_json::json;

use super::super::{InteractiveChatRuntime, InteractiveCommandContext, print_default_inline_lines};
use super::result::{output_str, redacted_output_json, redacted_value_json};

pub(in crate::chat::interactive) async fn handle_vision_command(
    args: Vec<&str>,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    match args.as_slice() {
        ["describe", rest @ ..] => {
            let describe = parse_vision_describe_args(rest)?;
            let mut input = json!({
                "image": describe.image,
                "prompt": describe.prompt,
            });
            if let Some(detail) = describe.detail {
                input["detail"] = json!(detail);
            }
            let result = runtime
                .session
                .execute_skill(ctx.registry, "vision_describe", input)
                .await?;
            emit_interactive_vision_result(&result, runtime)?;
        }
        ["help"] | ["--help"] | [] => emit_vision_usage(runtime)?,
        _ => emit_vision_usage(runtime)?,
    }
    Ok(())
}

fn parse_vision_describe_args(args: &[&str]) -> Result<VisionDescribeArgs> {
    let mut image: Option<String> = None;
    let mut prompt =
        "Describe this image. Mention visible text, UI state, objects, and anything relevant to debugging or understanding the scene."
            .to_owned();
    let mut detail = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--prompt" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /vision describe <image> --prompt TEXT"))?;
                prompt = (*value).to_owned();
                index += 2;
            }
            "--detail" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /vision describe <image> --detail low|high|auto")
                })?;
                detail = Some((*value).to_owned());
                index += 2;
            }
            value if image.is_none() => {
                image = Some(value.to_owned());
                index += 1;
            }
            value => return Err(anyhow!("unexpected /vision argument: {value}")),
        }
    }
    Ok(VisionDescribeArgs {
        image: image.ok_or_else(|| anyhow!("usage: /vision describe <image>"))?,
        prompt,
        detail,
    })
}

fn print_vision_usage() {
    println!(
        "vision_usage: /vision describe <image-path|url|data-url> [--prompt TEXT] [--detail low|high|auto]"
    );
}

fn emit_vision_usage(runtime: &mut InteractiveChatRuntime) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        runtime.push_notice(WorkbenchNotice::info(
            "vision",
            "usage is available in the fullscreen vision cell",
        ));
    } else if runtime.default_inline_stdout() {
        print_default_inline_lines(vision_usage_human_lines())?;
    } else {
        print_vision_usage();
    }
    Ok(())
}

fn vision_usage_human_lines() -> Vec<String> {
    vec![
        "* Vision".to_owned(),
        "  usage: /vision describe <image-path|url|data-url>".to_owned(),
        "  options: --prompt TEXT, --detail low|high|auto".to_owned(),
    ]
}

fn emit_interactive_vision_result(
    result: &ikaros_core::ToolResult,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        let model = output_str(result, "model").unwrap_or("unknown");
        let content = output_str(result, "content").unwrap_or("");
        runtime.push_notice(WorkbenchNotice::info(
            "vision result",
            &format!(
                "ok={} model={} summary={} content={}",
                result.ok,
                terminal_inline(model),
                terminal_inline(&result.summary),
                terminal_inline(content)
            ),
        ));
        return Ok(());
    }
    if runtime.default_inline_stdout() {
        let mut lines = vec![
            "* Vision".to_owned(),
            format!("  ok: {}", result.ok),
            format!("  summary: {}", terminal_inline(&result.summary)),
        ];
        if let Some(model) = output_str(result, "model") {
            lines.push(format!("  model: {}", terminal_inline(model)));
        }
        if let Some(content) = output_str(result, "content") {
            lines.push("  content:".to_owned());
            for line in render_terminal_markdown(content).lines() {
                lines.push(format!("    {line}"));
            }
        }
        print_default_inline_lines(lines)?;
        return Ok(());
    }
    println!(
        "vision_result: ok={} summary={}",
        result.ok,
        terminal_inline(&result.summary)
    );
    if let Some(model) = output_str(result, "model") {
        println!("vision_model: {}", terminal_inline(model));
    }
    if let Some(content) = output_str(result, "content") {
        println!("vision_content: {}", render_terminal_markdown(content));
    }
    if let Some(usage) = result.output.get("usage") {
        println!("vision_usage: {}", redacted_value_json(usage)?);
    }
    println!("vision_json: {}", redacted_output_json(result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_vision_describe_args;

    #[test]
    fn parses_vision_describe_args_with_prompt_and_detail() {
        let args = parse_vision_describe_args(&[
            "screen.png",
            "--prompt",
            "Read the UI",
            "--detail",
            "high",
        ])
        .expect("vision describe args");

        assert_eq!(args.image, "screen.png");
        assert_eq!(args.prompt, "Read the UI");
        assert_eq!(args.detail.as_deref(), Some("high"));
    }
}
