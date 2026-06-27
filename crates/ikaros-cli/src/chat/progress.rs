// SPDX-License-Identifier: GPL-3.0-only

use super::interactive::InteractiveChatRuntime;
use super::notice::{WorkbenchNotice, WorkbenchNoticeKind};
use super::{interactive_chat_turn_error_actions, workbench::terminal_inline};
use ikaros_core::redact_secrets;
use ikaros_runtime::ChatRunOptions;
use serde_json::json;

pub(in crate::chat) use ikaros_tui::WorkbenchProgressSnapshot;

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
    if runtime.machine_stdout_quiet() {
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
