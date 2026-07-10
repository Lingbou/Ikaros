// SPDX-License-Identifier: GPL-3.0-only

mod args;
mod attachments;
mod command;
mod errors;
mod history;
mod interactive;
mod live;
mod notice;
mod output;
mod pending;
mod progress;
mod runtime;
mod session_id;
mod single;
mod slash;
mod turn;
mod workbench;

pub(crate) use args::ChatArgs;
#[cfg(test)]
use command::sanitize_interactive_input;
pub(crate) use command::{chat_command, default_chat_command};
pub(in crate::chat) use errors::{
    interactive_chat_turn_error_actions, interactive_chat_turn_error_kind, suggested_budget_command,
};
#[cfg(test)]
use errors::{
    interactive_chat_turn_error_json_line, interactive_chat_turn_recovery_hint,
    interactive_command_error_json_line,
};
#[cfg(test)]
use live::{compact_live_event_cells, default_live_cell_event, live_cells_json_line};
#[cfg(test)]
use runtime::initial_interactive_runtime;
#[cfg(test)]
use session_id::interactive_chat_session_id;

#[cfg(test)]
mod tests;
