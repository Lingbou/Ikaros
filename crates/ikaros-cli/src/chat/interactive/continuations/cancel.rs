// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, anyhow};
use ikaros_state::session::{
    SessionContinuationStatus, SessionId, SessionStore, SqliteSessionStore,
};
use std::collections::VecDeque;

use crate::chat::notice::{WorkbenchNotice, WorkbenchNoticeKind};

use super::super::{InteractiveChatRuntime, terminal_inline};
use super::output::continuations_json_line;
use super::{WorkbenchCancelReport, WorkbenchCancelTarget, cancel_session_continuations};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat) struct WorkbenchSelectedContinuationCancelReport {
    pub(in crate::chat) continuation_id: Option<String>,
    pub(in crate::chat) report: WorkbenchCancelReport,
}

pub(in crate::chat) fn cancel_selected_screen_continuation(
    store: &dyn SessionStore,
    session_id: &SessionId,
    approval_side_panel_rows: usize,
    pending_inputs: &VecDeque<String>,
    side_selection: usize,
    reason: &str,
) -> Result<WorkbenchSelectedContinuationCancelReport> {
    let first_continuation_row = approval_side_panel_rows + pending_inputs.len().min(4);
    let empty = || WorkbenchSelectedContinuationCancelReport {
        continuation_id: None,
        report: WorkbenchCancelReport {
            cancelled: 0,
            skipped: 0,
            missing: 0,
        },
    };
    if side_selection < first_continuation_row {
        return Ok(empty());
    }
    let continuation_index = side_selection - first_continuation_row;
    let continuations = store.continuations(session_id)?;
    let Some(continuation_id) = continuations
        .iter()
        .filter(|continuation| {
            matches!(
                continuation.status,
                SessionContinuationStatus::Queued | SessionContinuationStatus::Running
            )
        })
        .take(4)
        .nth(continuation_index)
        .map(|continuation| continuation.continuation_id.as_str().to_owned())
    else {
        return Ok(empty());
    };
    let report = cancel_session_continuations(
        store,
        session_id,
        WorkbenchCancelTarget::Continuation(continuation_id.clone()),
        reason,
    )?;
    Ok(WorkbenchSelectedContinuationCancelReport {
        continuation_id: Some(continuation_id),
        report,
    })
}

pub(in crate::chat::interactive) fn handle_cancel_command(
    args: Vec<&str>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    let target = match args.as_slice() {
        [] | ["all"] => WorkbenchCancelTarget::All,
        [continuation_id] => WorkbenchCancelTarget::Continuation((*continuation_id).to_owned()),
        _ => return Err(anyhow!("usage: /cancel [all|<continuation-id>]")),
    };
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let report =
        cancel_session_continuations(&store, &session_id, target.clone(), "workbench cancel")?;
    let target_label = match target {
        WorkbenchCancelTarget::All => "all".to_owned(),
        WorkbenchCancelTarget::Continuation(id) => terminal_inline(&id),
    };
    if runtime.fullscreen_stdout_quiet() {
        runtime.push_notice(WorkbenchNotice::new(
            WorkbenchNoticeKind::Continuation,
            "cancel",
            &format!(
                "target={} cancelled={} skipped={} missing={}",
                terminal_inline(&target_label),
                report.cancelled,
                report.skipped,
                report.missing
            ),
        ));
    } else if runtime.default_inline_stdout() {
        println!("* Cancel");
        println!("  target: {}", terminal_inline(&target_label));
        println!("  cancelled: {}", report.cancelled);
        println!("  skipped: {}", report.skipped);
        println!("  missing: {}", report.missing);
    } else {
        println!(
            "workbench_cancel: target={} cancelled={} skipped={} missing={}",
            terminal_inline(&target_label),
            report.cancelled,
            report.skipped,
            report.missing
        );
        println!(
            "{}",
            continuations_json_line(&store.continuations(&session_id)?)
        );
    }
    Ok(())
}
