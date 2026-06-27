// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::notice::WorkbenchNotice;
use crate::image::{ImageCommand, ImageGenerateArgs, ImageResponseFormat, image_command};
use anyhow::{Context, Result, anyhow};
use ikaros_terminal::terminal_inline;
use serde_json::json;
use std::path::PathBuf;

use super::super::{InteractiveChatRuntime, InteractiveCommandContext, print_default_inline_lines};
use super::result::{output_str, output_u64, redacted_output_json};

pub(in crate::chat::interactive) async fn handle_image_command(
    args: Vec<&str>,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    match args.as_slice() {
        ["generate", rest @ ..] => {
            let generate = parse_image_generate_args(rest)?;
            if generate.output_dir.is_some() {
                image_command(
                    ImageCommand::Generate(generate),
                    ctx.paths,
                    ctx.workspace,
                    Some(&runtime.agent.name),
                )
                .await?;
            } else {
                let input = image_generate_skill_input(&generate);
                let result = runtime
                    .session
                    .execute_skill(ctx.registry, "image_generate", input)
                    .await?;
                emit_interactive_image_result(&result, runtime)?;
            }
        }
        ["help"] | ["--help"] | [] => emit_image_usage(runtime)?,
        _ => emit_image_usage(runtime)?,
    }
    Ok(())
}

fn parse_image_generate_args(args: &[&str]) -> Result<ImageGenerateArgs> {
    let mut prompt_tokens = Vec::new();
    let mut model = None;
    let mut size = "1024x1024".to_owned();
    let mut n = 1;
    let mut response_format = ImageResponseFormat::Url;
    let mut quality = None;
    let mut style = None;
    let mut output_dir = None;
    let mut output_format = "png".to_owned();
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--prompt" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate --prompt TEXT"))?;
                prompt_tokens.push((*value).to_owned());
                index += 2;
            }
            "--model" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --model MODEL"))?;
                model = Some((*value).to_owned());
                index += 2;
            }
            "--size" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --size 1024x1024"))?;
                size = (*value).to_owned();
                index += 2;
            }
            "--n" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --n N"))?;
                n = value
                    .parse::<u32>()
                    .with_context(|| "--n must be a positive integer")?;
                index += 2;
            }
            "--response-format" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /image generate <prompt> --response-format url|b64_json")
                })?;
                response_format = parse_image_response_format(value)?;
                index += 2;
            }
            "--quality" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --quality QUALITY"))?;
                quality = Some((*value).to_owned());
                index += 2;
            }
            "--style" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --style STYLE"))?;
                style = Some((*value).to_owned());
                index += 2;
            }
            "--output-dir" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --output-dir PATH"))?;
                output_dir = Some(PathBuf::from(value));
                index += 2;
            }
            "--output-format" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /image generate <prompt> --output-format png")
                })?;
                output_format = (*value).to_owned();
                index += 2;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /image generate <prompt> [--model MODEL] [--size 1024x1024] [--n N] [--response-format url|b64_json] [--output-dir PATH]"
                ));
            }
            value if value.starts_with("--") => {
                return Err(anyhow!("unknown /image generate argument: {value}"));
            }
            value => {
                prompt_tokens.push(value.to_owned());
                index += 1;
            }
        }
    }
    let prompt = prompt_tokens.join(" ");
    if prompt.trim().is_empty() {
        return Err(anyhow!("usage: /image generate <prompt>"));
    }
    Ok(ImageGenerateArgs {
        prompt,
        model,
        size,
        n,
        response_format,
        quality,
        style,
        output_dir,
        output_format,
    })
}

fn parse_image_response_format(value: &str) -> Result<ImageResponseFormat> {
    match value.trim().to_ascii_lowercase().as_str() {
        "url" => Ok(ImageResponseFormat::Url),
        "b64" | "b64_json" | "b64-json" => Ok(ImageResponseFormat::B64Json),
        _ => Err(anyhow!("--response-format must be url or b64_json")),
    }
}

fn print_image_usage() {
    println!(
        "image_usage: /image generate <prompt> [--model MODEL] [--size 1024x1024] [--n N] [--response-format url|b64_json] [--quality VALUE] [--style VALUE] [--output-dir PATH]"
    );
}

fn emit_image_usage(runtime: &mut InteractiveChatRuntime) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        runtime.push_notice(WorkbenchNotice::info(
            "image",
            "usage is available in the fullscreen image cell",
        ));
    } else if runtime.default_inline_stdout() {
        print_default_inline_lines(image_usage_human_lines())?;
    } else {
        print_image_usage();
    }
    Ok(())
}

fn image_usage_human_lines() -> Vec<String> {
    vec![
        "* Image".to_owned(),
        "  usage: /image generate <prompt>".to_owned(),
        "  options: --model MODEL, --size 1024x1024, --n N".to_owned(),
        "  output: add --output-dir PATH to save files directly".to_owned(),
    ]
}

fn image_generate_skill_input(args: &ImageGenerateArgs) -> serde_json::Value {
    let mut input = json!({
        "prompt": &args.prompt,
        "size": &args.size,
        "n": args.n,
        "response_format": match args.response_format {
            ImageResponseFormat::Url => "url",
            ImageResponseFormat::B64Json => "b64_json",
        },
    });
    if let Some(model) = args.model.as_deref() {
        input["model"] = json!(model);
    }
    if let Some(quality) = args.quality.as_deref() {
        input["quality"] = json!(quality);
    }
    if let Some(style) = args.style.as_deref() {
        input["style"] = json!(style);
    }
    input
}

fn emit_interactive_image_result(
    result: &ikaros_core::ToolResult,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        let model = output_str(result, "model").unwrap_or("unknown");
        let count = output_u64(result, "count").unwrap_or_default();
        runtime.push_notice(WorkbenchNotice::info(
            "image result",
            &format!(
                "ok={} model={} count={} summary={}",
                result.ok,
                terminal_inline(model),
                count,
                terminal_inline(&result.summary)
            ),
        ));
        return Ok(());
    }
    if runtime.default_inline_stdout() {
        let mut lines = vec![
            "* Image".to_owned(),
            format!("  ok: {}", result.ok),
            format!("  summary: {}", terminal_inline(&result.summary)),
        ];
        if let Some(model) = output_str(result, "model") {
            lines.push(format!("  model: {}", terminal_inline(model)));
        }
        if let Some(count) = output_u64(result, "count") {
            lines.push(format!("  images: {count}"));
        }
        print_default_inline_lines(lines)?;
        return Ok(());
    }
    println!(
        "image_result: ok={} summary={}",
        result.ok,
        terminal_inline(&result.summary)
    );
    if let Some(model) = output_str(result, "model") {
        println!("image_model: {}", terminal_inline(model));
    }
    if let Some(count) = output_u64(result, "count") {
        println!("image_count: {count}");
    }
    println!("image_json: {}", redacted_output_json(result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{image_generate_skill_input, parse_image_generate_args};
    use crate::image::ImageResponseFormat;

    #[test]
    fn parses_image_generate_args_and_skill_input() {
        let args = parse_image_generate_args(&[
            "futuristic",
            "terminal",
            "--model",
            "image-model",
            "--size",
            "512x512",
            "--n",
            "2",
            "--response-format",
            "b64_json",
            "--quality",
            "hd",
            "--style",
            "vivid",
        ])
        .expect("image generate args");

        assert_eq!(args.prompt, "futuristic terminal");
        assert_eq!(args.model.as_deref(), Some("image-model"));
        assert_eq!(args.size, "512x512");
        assert_eq!(args.n, 2);
        assert!(matches!(args.response_format, ImageResponseFormat::B64Json));
        assert_eq!(args.quality.as_deref(), Some("hd"));
        assert_eq!(args.style.as_deref(), Some("vivid"));

        let input = image_generate_skill_input(&args);
        assert_eq!(input["prompt"], "futuristic terminal");
        assert_eq!(input["response_format"], "b64_json");
        assert_eq!(input["model"], "image-model");
    }
}
