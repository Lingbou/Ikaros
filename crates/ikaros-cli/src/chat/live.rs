// SPDX-License-Identifier: GPL-3.0-only

mod activity;
mod cells;
mod sink;
#[cfg(test)]
mod tests;

pub(super) use cells::print_live_event_cells;
pub(super) use sink::{WorkbenchLiveEventSink, emit_interactive_chat_turn_failure_evidence};

#[cfg(test)]
pub(in crate::chat) use activity::{
    human_activity_line_for_terminal, human_activity_lines, human_activity_lines_with_debug_context,
};
#[cfg(test)]
pub(in crate::chat) use cells::{
    compact_live_event_cells, compact_live_event_cells_with_debug, default_live_cell_event,
    live_cells_json_line,
};
#[cfg(test)]
use ikaros_terminal::{AgentTextDeltaState, finish_agent_text_delta, format_agent_text_delta};
