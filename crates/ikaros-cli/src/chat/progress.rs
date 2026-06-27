// SPDX-License-Identifier: GPL-3.0-only

use super::notice::{WorkbenchNotice, WorkbenchNoticeKind};
use super::{interactive_chat_turn_error_actions, workbench::terminal_inline};
use ikaros_core::redact_secrets;
use ikaros_runtime::ChatRunOptions;
use serde_json::json;

use super::interactive::InteractiveChatRuntime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat) struct WorkbenchProgressSnapshot {
    pub(in crate::chat) kind: String,
    pub(in crate::chat) status: String,
    pub(in crate::chat) elapsed_ms: Option<u128>,
    pub(in crate::chat) detail: String,
    pub(in crate::chat) error_kind: Option<String>,
}

impl WorkbenchProgressSnapshot {
    fn new(
        kind: &str,
        status: &str,
        elapsed_ms: Option<u128>,
        detail: Option<&str>,
        error_kind: Option<&str>,
    ) -> Self {
        Self {
            kind: terminal_inline(kind),
            status: terminal_inline(status),
            elapsed_ms,
            detail: detail.map(progress_detail).unwrap_or_else(|| "none".into()),
            error_kind: error_kind.map(terminal_inline),
        }
    }

    pub(in crate::chat) fn phase(&self) -> &'static str {
        progress_phase(&self.status)
    }

    pub(in crate::chat) fn spinner(&self) -> &'static str {
        progress_spinner(self.elapsed_ms, &self.status)
    }

    pub(in crate::chat) fn progress_bar(&self) -> &'static str {
        progress_bar(&self.status)
    }
}

pub(super) fn print_workbench_progress(
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    kind: &str,
    status: &str,
    elapsed_ms: Option<u128>,
    detail: Option<&str>,
    error_kind: Option<&str>,
) {
    let snapshot = WorkbenchProgressSnapshot::new(kind, status, elapsed_ms, detail, error_kind);
    runtime.last_progress = Some(snapshot.clone());
    runtime.push_notice(WorkbenchNotice::new(
        progress_notice_kind(status, error_kind),
        kind,
        &format!(
            "status={} elapsed_ms={} detail={}",
            status,
            elapsed_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            detail.unwrap_or("none")
        ),
    ));
    if runtime.fullscreen_stdout_quiet() {
        return;
    }
    let elapsed = elapsed_ms
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".into());
    println!(
        "workbench_progress: kind={} status={} session={} stream={} agent_loop={} elapsed_ms={} detail={}{}",
        snapshot.kind,
        snapshot.status,
        redact_secrets(&runtime.chat_session_id),
        options.stream,
        options.agent_loop,
        elapsed,
        terminal_inline(&snapshot.detail),
        snapshot
            .error_kind
            .as_deref()
            .map(|kind| format!(" error_kind={}", terminal_inline(kind)))
            .unwrap_or_default()
    );
    println!(
        "{}",
        workbench_progress_json_line(runtime, options, &snapshot)
    );
}

fn progress_notice_kind(status: &str, error_kind: Option<&str>) -> WorkbenchNoticeKind {
    if error_kind.is_some() || status == "failed" {
        return WorkbenchNoticeKind::Error;
    }
    if status == "approval_pending" {
        return WorkbenchNoticeKind::Approval;
    }
    WorkbenchNoticeKind::Progress
}

fn workbench_progress_json_line(
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    snapshot: &WorkbenchProgressSnapshot,
) -> String {
    let actions = snapshot
        .error_kind
        .as_deref()
        .map(|kind| interactive_chat_turn_error_actions(kind, &snapshot.detail))
        .unwrap_or_else(|| json!([]));
    let payload = json!({
        "schema": "ikaros-workbench-progress-v1",
        "version": 1,
        "kind": snapshot.kind,
        "status": snapshot.status,
        "session_id": redact_secrets(&runtime.chat_session_id),
        "provider": terminal_inline(runtime.provider.name()),
        "model": terminal_inline(&runtime.model_config.model),
        "stream": options.stream,
        "agent_loop": options.agent_loop,
        "phase": snapshot.phase(),
        "spinner": snapshot.spinner(),
        "progress_bar": snapshot.progress_bar(),
        "elapsed_ms": snapshot.elapsed_ms,
        "detail": terminal_inline(&snapshot.detail),
        "error_kind": snapshot.error_kind,
        "actions": actions,
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-progress-v1","version":1,"kind":"unknown","status":"failed","detail":"serialization_failed"}"#
            .to_owned()
    });
    format!("workbench_progress_json: {encoded}")
}

fn progress_detail(input: &str) -> String {
    const MAX_CHARS: usize = 160;
    let redacted = redact_secrets(input).replace(['\n', '\r'], " ");
    let mut output = String::new();
    for (index, ch) in redacted.chars().enumerate() {
        if index >= MAX_CHARS {
            output.push_str("...");
            return output;
        }
        output.push(ch);
    }
    output
}

pub(in crate::chat) fn progress_phase(status: &str) -> &'static str {
    match status {
        "running" => "active",
        "queued" => "queued",
        "approval_pending" => "waiting_approval",
        "failed" => "recoverable",
        "completed" => "done",
        "cancelled" => "cancelled",
        _ => "idle",
    }
}

pub(in crate::chat) fn progress_spinner(elapsed_ms: Option<u128>, status: &str) -> &'static str {
    if !matches!(status, "running" | "queued" | "approval_pending") {
        return "-";
    }
    match elapsed_ms.unwrap_or_default() / 250 % 4 {
        0 => "|",
        1 => "/",
        2 => "-",
        _ => "\\",
    }
}

pub(in crate::chat) fn progress_bar(status: &str) -> &'static str {
    match status {
        "running" => "[###-------]",
        "queued" => "[#---------]",
        "approval_pending" => "[#####-----]",
        "completed" => "[##########]",
        "failed" => "[!!!-------]",
        "cancelled" => "[xxx-------]",
        _ => "[----------]",
    }
}
