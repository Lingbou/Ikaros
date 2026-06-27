// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, anyhow};

use crate::chat::attachments::{
    content_block_from_parts_resolving_path, content_block_kind, content_block_summary,
};
use std::path::Path;

use super::{InteractiveChatRuntime, terminal_inline};

pub(super) fn handle_attach_command(
    args: Vec<&str>,
    runtime: &mut InteractiveChatRuntime,
    workspace: &Path,
) -> Result<()> {
    match args.as_slice() {
        [] | ["status"] | ["list"] => {
            print_attachment_status(runtime);
        }
        ["clear"] => {
            let cleared = runtime.pending_content_blocks.len();
            runtime.pending_content_blocks.clear();
            println!("attachments_cleared: {cleared}");
            println!("{}", attachments_json_line(runtime));
        }
        ["remove", index] => {
            let index = index
                .parse::<usize>()
                .map_err(|_| anyhow!("attachment index must be a positive number"))?;
            if index == 0 || index > runtime.pending_content_blocks.len() {
                println!(
                    "attachment_remove_error: index={} reason=not_found pending={}",
                    index,
                    runtime.pending_content_blocks.len()
                );
                println!("{}", attachments_json_line(runtime));
            } else {
                let removed = runtime.pending_content_blocks.remove(index - 1);
                println!(
                    "attachment_removed: index={} kind={} remaining={}",
                    index,
                    content_block_kind(&removed),
                    runtime.pending_content_blocks.len()
                );
                println!("{}", attachments_json_line(runtime));
            }
        }
        ["image", rest @ ..] => {
            if rest.is_empty() {
                return Err(anyhow!(
                    "usage: /attach image <url-or-path> [--detail low|high|auto]"
                ));
            }
            let (value, detail) = parse_image_attachment_args(rest)?;
            let mut block = content_block_from_parts_resolving_path("image", &value, workspace)?;
            if let Some(detail) = detail {
                if let ikaros_models::ModelContentBlock::Image {
                    detail: image_detail,
                    ..
                } = &mut block
                {
                    *image_detail = Some(detail);
                }
            }
            let kind = content_block_kind(&block);
            runtime.pending_content_blocks.push(block);
            println!(
                "attachment_queued: kind={} pending={} attachments_force_single_call=true",
                kind,
                runtime.pending_content_blocks.len()
            );
            println!("{}", attachments_json_line(runtime));
        }
        [kind @ ("audio" | "file"), rest @ ..] => {
            if rest.is_empty() {
                return Err(anyhow!("usage: /attach {kind} <url-or-path>"));
            }
            let value = rest.join(" ");
            let block = content_block_from_parts_resolving_path(kind, &value, workspace)?;
            let kind = content_block_kind(&block);
            runtime.pending_content_blocks.push(block);
            println!(
                "attachment_queued: kind={} pending={} attachments_force_single_call=true",
                kind,
                runtime.pending_content_blocks.len()
            );
            println!("{}", attachments_json_line(runtime));
        }
        ["help"] | ["--help"] => {
            print_attachment_usage();
        }
        _ => {
            print_attachment_usage();
        }
    }
    Ok(())
}

fn print_attachment_status(runtime: &InteractiveChatRuntime) {
    println!(
        "attachments_pending: {} attachments_force_single_call={}",
        runtime.pending_content_blocks.len(),
        !runtime.pending_content_blocks.is_empty()
    );
    for (index, block) in runtime.pending_content_blocks.iter().enumerate() {
        println!(
            "attachment {}: {}",
            index + 1,
            terminal_inline(&content_block_summary(block))
        );
    }
    println!("{}", attachments_json_line(runtime));
}

fn print_attachment_usage() {
    println!("usage: /attach image <url-or-path> [--detail low|high|auto]");
    println!("usage: /attach <audio|file> <url-or-path>");
    println!("usage: /attach list");
    println!("usage: /attach remove <index>");
    println!("usage: /attach clear");
}

fn parse_image_attachment_args(args: &[&str]) -> Result<(String, Option<String>)> {
    let mut value = Vec::new();
    let mut detail = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--detail" => {
                let selected = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /attach image <url-or-path> --detail low|high|auto")
                })?;
                let normalized = selected.trim().to_ascii_lowercase();
                if !matches!(normalized.as_str(), "low" | "high" | "auto") {
                    return Err(anyhow!(
                        "image attachment detail must be low, high, or auto"
                    ));
                }
                detail = Some(normalized);
                index += 2;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /attach image <url-or-path> [--detail low|high|auto]"
                ));
            }
            other if other.starts_with("--") => {
                return Err(anyhow!("unknown /attach image argument: {other}"));
            }
            other => {
                value.push(other);
                index += 1;
            }
        }
    }
    let value = value.join(" ");
    if value.trim().is_empty() {
        return Err(anyhow!(
            "usage: /attach image <url-or-path> [--detail low|high|auto]"
        ));
    }
    Ok((value, detail))
}

fn attachments_json_line(runtime: &InteractiveChatRuntime) -> String {
    let attachments = runtime
        .pending_content_blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            serde_json::json!({
                "index": index + 1,
                "kind": content_block_kind(block),
                "summary": terminal_inline(&content_block_summary(block)),
                "remove_command": format!("/attach remove {}", index + 1),
            })
        })
        .collect::<Vec<_>>();
    format!(
        "attachments_json: {}",
        serde_json::json!({
            "schema": "ikaros-workbench-attachments-v1",
            "version": 1,
            "pending": runtime.pending_content_blocks.len(),
            "force_single_call": !runtime.pending_content_blocks.is_empty(),
            "attachments": attachments,
            "actions": {
                "list": "/attach list",
                "clear": "/attach clear",
            },
        })
    )
}
