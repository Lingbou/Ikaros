// SPDX-License-Identifier: GPL-3.0-only

mod agent;
mod approval;
mod attachment;
mod code_alias;
mod context;
mod continuations;
mod debug;
mod dispatch;
mod evidence;
mod mcp;
mod output;
mod parse;
mod provider;
mod runtime;
mod screen;
mod session;
mod state;
mod status;
#[cfg(test)]
mod tests;

use ikaros_terminal::terminal_inline;

use approval::handle_approval_command;
pub(in crate::chat) use context::InteractiveCommandContext;
use continuations::print_workbench_continuation_status;
#[cfg(test)]
pub(super) use continuations::{cancel_selected_screen_continuation, continuations_json_line};
pub(super) use debug::print_debug_status_for_human;
pub(in crate::chat) use dispatch::handle_interactive_chat_command;
use evidence::append_workbench_evidence;
use output::print_default_inline_lines;
pub(in crate::chat) use output::suppress_fullscreen_stdout_command;
#[cfg(test)]
use parse::parse_trace_request;
use parse::{parse_debug_dump_recent_logs, parse_debug_logs_args, parse_timeline_request};
pub(in crate::chat) use runtime::InteractiveChatRuntime;
#[allow(unused_imports)]
pub(in crate::chat) use runtime::should_emit_default_inline_stdout;
#[allow(unused_imports)]
pub(in crate::chat) use state::ClearInteractiveSessionReport;
pub(in crate::chat) use state::clear_interactive_session;
pub(super) use status::{
    InteractiveChatStatusInput, available_agent_lines, available_agent_lines_for_human,
    format_interactive_chat_status,
};
