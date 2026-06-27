// SPDX-License-Identifier: GPL-3.0-only

mod cancel;
mod output;
mod queue;
mod status;

use anyhow::Result;
use ikaros_state::session::{
    SessionContinuation, SessionContinuationStatus, SessionId, SessionStore,
};
use std::collections::VecDeque;

pub(in crate::chat) use ikaros_agent::chat::workbench_state::{
    WorkbenchCancelReport, WorkbenchCancelTarget, cancel_session_continuations,
    requeue_workbench_continuation,
};

pub(super) use cancel::handle_cancel_command;
pub(in crate::chat) use output::continuations_json_line;
pub(super) use queue::handle_queue_command;
pub(super) use status::print_workbench_continuation_status;

pub(in crate::chat) type WorkbenchSelectedContinuationCancelReport =
    cancel::WorkbenchSelectedContinuationCancelReport;
pub(super) type WorkbenchSelectedInputClearReport = queue::WorkbenchSelectedInputClearReport;

pub(in crate::chat) fn cancel_selected_screen_continuation(
    store: &dyn SessionStore,
    session_id: &SessionId,
    approval_side_panel_rows: usize,
    pending_inputs: &VecDeque<String>,
    side_selection: usize,
    reason: &str,
) -> Result<WorkbenchSelectedContinuationCancelReport> {
    cancel::cancel_selected_screen_continuation(
        store,
        session_id,
        approval_side_panel_rows,
        pending_inputs,
        side_selection,
        reason,
    )
}

pub(super) fn clear_selected_screen_input(
    approval_side_panel_rows: usize,
    pending_inputs: &mut VecDeque<String>,
    side_selection: usize,
) -> WorkbenchSelectedInputClearReport {
    queue::clear_selected_screen_input(approval_side_panel_rows, pending_inputs, side_selection)
}

pub(super) fn continuation_status_count(
    continuations: &[SessionContinuation],
    status: SessionContinuationStatus,
) -> usize {
    output::continuation_status_count(continuations, status)
}
