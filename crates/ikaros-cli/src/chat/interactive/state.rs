// SPDX-License-Identifier: GPL-3.0-only

use ikaros_agent::chat::{ChatRunOptions, new_chat_session_id};

use crate::chat::workbench::WorkbenchScreenState;

use super::InteractiveChatRuntime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat) struct ClearInteractiveSessionReport {
    pub(in crate::chat) old_session_id: String,
    pub(in crate::chat) new_session_id: String,
    pub(in crate::chat) pending_inputs_cleared: usize,
    pub(in crate::chat) attachments_cleared: usize,
}

pub(in crate::chat) fn clear_interactive_session(
    runtime: &mut InteractiveChatRuntime,
    options: &mut ChatRunOptions,
) -> ClearInteractiveSessionReport {
    let old_session_id = runtime.chat_session_id.clone();
    let new_session_id = new_chat_session_id();
    let pending_inputs_cleared = runtime.pending_inputs.len();
    let attachments_cleared = runtime.pending_content_blocks.len();

    runtime.chat_session_id = new_session_id.clone();
    options.session_id = Some(new_session_id.clone());
    runtime.pending_inputs.clear();
    runtime.pending_content_blocks.clear();
    runtime.screen_state = WorkbenchScreenState::default();
    runtime.last_progress = None;
    runtime.notices.clear();
    runtime.pending_input_drain_requested = false;

    ClearInteractiveSessionReport {
        old_session_id,
        new_session_id,
        pending_inputs_cleared,
        attachments_cleared,
    }
}
