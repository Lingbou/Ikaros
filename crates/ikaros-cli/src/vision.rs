// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::render_terminal_markdown;
use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_core::{IkarosPaths, redact_secrets};
use ikaros_host::VisionDescribeRequest;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum VisionCommand {
    Describe(VisionDescribeArgs),
}

#[derive(Debug, Args)]
pub(crate) struct VisionDescribeArgs {
    pub(crate) image: String,
    #[arg(
        long,
        default_value = "Describe this image. Mention visible text, UI state, objects, and anything relevant to debugging or understanding the scene."
    )]
    pub(crate) prompt: String,
    #[arg(long)]
    pub(crate) detail: Option<String>,
}

pub(crate) async fn vision_command(
    command: VisionCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        VisionCommand::Describe(args) => {
            describe_image(args, paths, workspace, agent_override).await
        }
    }
}

async fn describe_image(
    args: VisionDescribeArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let response = ikaros_host::describe_image(
        paths,
        workspace,
        agent_override,
        VisionDescribeRequest {
            image: args.image,
            prompt: args.prompt,
            detail: args.detail,
        },
    )
    .await?;
    println!("vision_model: {}", redact_secrets(&response.model));
    println!(
        "vision_content: {}",
        render_terminal_markdown(&response.content)
    );
    println!(
        "vision_usage: prompt_tokens={} completion_tokens={} total_tokens={}",
        response.usage.prompt_tokens.unwrap_or_default(),
        response.usage.completion_tokens.unwrap_or_default(),
        response.usage.total_tokens.unwrap_or_default()
    );
    Ok(())
}
