// SPDX-License-Identifier: GPL-3.0-only

mod cells;
mod history;
mod mentions;
mod slash;
mod status;
#[cfg(test)]
mod tests;

pub(super) use cells::{agent_event_cell, coding_event_cells, session_entry_cell};
pub(super) use history::{
    append_workbench_history, load_workbench_history_entries, normalize_session_id, path_display,
    print_workbench_input_history, workbench_history_path, workbench_input_history_human_lines,
};
pub(super) use ikaros_terminal::render_terminal_markdown_for_current_width as render_terminal_markdown;
#[cfg(test)]
pub(super) use ikaros_terminal::render_workbench_snapshot;
pub(super) use ikaros_terminal::{
    WorkbenchCell, WorkbenchCellKind, WorkbenchScreen, WorkbenchScreenApprovalAction,
    WorkbenchScreenContinuationAction, WorkbenchScreenInputAction, WorkbenchScreenOpenAction,
    WorkbenchScreenState, apply_workbench_screen_args, command_requires_explicit_action,
    render_fullscreen_terminal_frame, render_fullscreen_workbench_with_state, screen_json_line,
    screen_selected_actions_json_line, screen_selected_actions_line, screen_selected_cell_line,
    screen_selected_primary_action,
};
use ikaros_terminal::{terminal_inline, terminal_message};
pub(super) use mentions::{context_mentions_human_lines, print_context_mentions};
pub(super) use slash::{
    format_slash_command_help, print_slash_commands, print_slash_commands_for_human,
    slash_command_registry_summary, slash_commands_human_lines, suggest_slash_command,
};
pub(super) use status::{
    TimelineRequest, TimelineVerbosity, active_model_budget_status, context_status_human_lines,
    format_model_budget_status, mcp_status_human_lines, memory_status_human_lines,
    model_status_human_lines, print_approval_status, print_context_status,
    print_context_status_for_human, print_diff_status, print_diff_status_for_human,
    print_mcp_status, print_mcp_status_for_human, print_memory_status,
    print_memory_status_for_human, print_model_status, print_model_status_for_human,
    print_provider_status_for_human, print_rag_status, print_rag_status_for_human,
    print_replay_status, print_replay_status_for_human, print_screen_status_with_state,
    print_session_export, print_session_history, print_session_status, print_session_summaries,
    print_tools_status, print_tools_status_for_human, print_trace_status,
    print_trace_status_for_human, print_workbench_status, print_workbench_status_for_human,
    provider_status_human_lines, rag_status_human_lines, selected_screen_primary_action,
    session_history_human_lines, session_status_human_lines, session_summaries_human_lines,
    tools_status_human_lines, workbench_status_human_lines,
};

pub(super) fn format_workbench_help() -> String {
    format_slash_command_help()
}
