// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_agent::chat::{WorkbenchDiffStatus, collect_workbench_diff_status};
use std::path::Path;

use super::super::{render_terminal_markdown, terminal_inline};

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

    println!("* Diff");
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

async fn collect_diff_status(
    runtime: &InteractiveChatRuntime,
    workspace: &Path,
) -> Result<WorkbenchDiffStatus> {
    collect_workbench_diff_status(&runtime.session, workspace).await
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
