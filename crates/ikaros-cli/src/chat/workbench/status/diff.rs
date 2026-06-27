// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::redact_secrets;
use ikaros_harness::ProcessRequest;
use std::path::Path;

use super::super::{render_terminal_markdown, terminal_inline};

const DIFF_PREVIEW_MAX_BYTES: usize = 32 * 1024;
const DIFF_PREVIEW_MAX_LINES: usize = 160;

pub(in crate::chat) async fn print_diff_status(
    runtime: &InteractiveChatRuntime,
    workspace: &Path,
) -> Result<()> {
    let status = collect_diff_status(runtime, workspace).await?;
    println!("diff_status: {}", status.status);
    let stdout = status.stat_stdout.trim();
    let stderr = status.stat_stderr.trim();
    if stdout.is_empty() {
        println!("diff_stat: clean_or_unavailable");
    } else {
        println!("diff_stat:");
        for line in stdout.lines() {
            println!("{}", terminal_inline(line));
        }
    }
    if !stderr.is_empty() {
        println!("diff_error: {}", terminal_inline(stderr));
    }
    let patch_stderr = status.preview_stderr.trim();
    if status.preview.text.is_empty() {
        println!("diff_preview: clean_or_unavailable");
    } else {
        println!("diff_preview:");
        println!(
            "{}",
            render_terminal_markdown(&format!("```diff\n{}\n```", status.preview.text))
        );
        if status.preview.truncated {
            println!("diff_preview_truncated: true");
        }
    }
    if !patch_stderr.is_empty() {
        println!("diff_preview_error: {}", terminal_inline(patch_stderr));
    }
    println!(
        "{}",
        diff_status_json_line(
            status.status,
            stdout,
            stderr,
            status.preview_status,
            status.preview.rendered_lines,
            status.preview.truncated,
            patch_stderr,
        )
    );
    Ok(())
}

pub(in crate::chat) async fn print_diff_status_for_human(
    runtime: &InteractiveChatRuntime,
    workspace: &Path,
) -> Result<()> {
    let status = collect_diff_status(runtime, workspace).await?;
    let stdout = status.stat_stdout.trim();
    let stderr = status.stat_stderr.trim();
    let patch_stderr = status.preview_stderr.trim();

    println!("• Diff");
    if stdout.is_empty() {
        println!("  changes: clean or unavailable");
    } else {
        println!("  stat:");
        for line in stdout.lines().take(20) {
            println!("    {}", terminal_inline(line));
        }
    }

    if !stderr.is_empty() {
        println!("  warning: {}", terminal_inline(stderr));
    }
    if !patch_stderr.is_empty() && patch_stderr != stderr {
        println!("  preview warning: {}", terminal_inline(patch_stderr));
    }

    if !status.preview.text.is_empty() {
        println!("  preview:");
        println!(
            "{}",
            render_terminal_markdown(&format!("```diff\n{}\n```", status.preview.text))
        );
        if status.preview.truncated {
            println!("  preview truncated");
        }
    }
    println!("  actions: /code plan, /code review, /code apply");
    Ok(())
}

struct DiffStatus {
    status: i32,
    stat_stdout: String,
    stat_stderr: String,
    preview_status: i32,
    preview: DiffPreview,
    preview_stderr: String,
}

struct DiffPreview {
    text: String,
    rendered_lines: usize,
    truncated: bool,
}

async fn collect_diff_status(
    runtime: &InteractiveChatRuntime,
    workspace: &Path,
) -> Result<DiffStatus> {
    let output = runtime
        .session
        .env
        .run_process(
            ProcessRequest::program(
                "git",
                vec!["diff".into(), "--stat".into(), "--".into()],
                workspace,
            )
            .with_timeout_ms(2_000)
            .with_max_output_bytes(8 * 1024),
        )
        .await?;
    let patch_output = runtime
        .session
        .env
        .run_process(
            ProcessRequest::program(
                "git",
                vec![
                    "diff".into(),
                    "--no-ext-diff".into(),
                    "--unified=3".into(),
                    "--".into(),
                ],
                workspace,
            )
            .with_timeout_ms(3_000)
            .with_max_output_bytes(DIFF_PREVIEW_MAX_BYTES),
        )
        .await?;
    let preview = diff_preview_text(patch_output.stdout.trim_end(), DIFF_PREVIEW_MAX_LINES);
    Ok(DiffStatus {
        status: output.status,
        stat_stdout: output.stdout,
        stat_stderr: output.stderr,
        preview_status: patch_output.status,
        preview,
        preview_stderr: patch_output.stderr,
    })
}

fn diff_preview_text(stdout: &str, max_lines: usize) -> DiffPreview {
    let mut text = String::new();
    let mut rendered_lines = 0;
    let mut truncated = false;
    for (index, line) in stdout.lines().enumerate() {
        if index >= max_lines {
            truncated = true;
            break;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line);
        rendered_lines += 1;
    }
    DiffPreview {
        text: redact_secrets(&text),
        rendered_lines,
        truncated,
    }
}

fn diff_status_json_line(
    status: i32,
    stdout: &str,
    stderr: &str,
    preview_status: i32,
    preview_line_count: usize,
    preview_truncated: bool,
    preview_stderr: &str,
) -> String {
    let stat_lines = stdout
        .lines()
        .take(20)
        .map(terminal_inline)
        .collect::<Vec<_>>();
    let error_lines = stderr
        .lines()
        .take(10)
        .map(terminal_inline)
        .collect::<Vec<_>>();
    let preview_error_lines = preview_stderr
        .lines()
        .take(10)
        .map(terminal_inline)
        .collect::<Vec<_>>();
    let payload = serde_json::json!({
        "schema": "ikaros-workbench-diff-status-v1",
        "version": 1,
        "status": status,
        "preview_status": preview_status,
        "has_changes": !stat_lines.is_empty(),
        "stat_line_count": stat_lines.len(),
        "error_line_count": error_lines.len(),
        "preview_line_count": preview_line_count,
        "preview_truncated": preview_truncated,
        "preview_error_line_count": preview_error_lines.len(),
        "stat_lines": stat_lines,
        "error_lines": error_lines,
        "preview_error_lines": preview_error_lines,
        "actions": {
            "code_plan": "/code plan",
            "code_apply": "/code apply",
            "code_test": "/code test",
            "code_review": "/code review",
            "code_rollback": "/code rollback",
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-diff-status-v1","version":1,"error":"serialization_failed"}"#
            .to_owned()
    });
    format!("diff_status_json: {encoded}")
}
