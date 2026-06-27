// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_core::redact_secrets;
use ikaros_execution::harness::{ExecutionSession, ProcessRequest};
use std::path::Path;

const DIFF_PREVIEW_MAX_BYTES: usize = 32 * 1024;
const DIFF_PREVIEW_MAX_LINES: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchDiffStatus {
    pub status: i32,
    pub stat_stdout: String,
    pub stat_stderr: String,
    pub preview_status: i32,
    pub preview: WorkbenchDiffPreview,
    pub preview_stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchDiffPreview {
    pub text: String,
    pub rendered_lines: usize,
    pub truncated: bool,
}

pub async fn collect_workbench_diff_status(
    session: &ExecutionSession,
    workspace: &Path,
) -> Result<WorkbenchDiffStatus> {
    let output = session
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
    let patch_output = session
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
    let preview =
        workbench_diff_preview_text(patch_output.stdout.trim_end(), DIFF_PREVIEW_MAX_LINES);
    Ok(WorkbenchDiffStatus {
        status: output.status,
        stat_stdout: output.stdout,
        stat_stderr: output.stderr,
        preview_status: patch_output.status,
        preview,
        preview_stderr: patch_output.stderr,
    })
}

pub fn workbench_diff_preview_text(stdout: &str, max_lines: usize) -> WorkbenchDiffPreview {
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
    WorkbenchDiffPreview {
        text: redact_secrets(&text),
        rendered_lines,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workbench_diff_preview_truncates_and_redacts() {
        let preview =
            workbench_diff_preview_text("line1\n+api_key = \"sk-secret-value\"\nline3\nline4", 3);

        assert_eq!(preview.rendered_lines, 3);
        assert!(preview.truncated);
        assert!(preview.text.contains("line1"));
        assert!(preview.text.contains("line3"));
        assert!(!preview.text.contains("line4"));
        assert!(!preview.text.contains("sk-secret-value"));
    }
}
