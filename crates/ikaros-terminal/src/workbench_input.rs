// SPDX-License-Identifier: GPL-3.0-only

mod control;
mod events;
mod state;

#[cfg(test)]
mod tests;

pub use self::control::handle_workbench_input_control;
pub use self::events::{
    WorkbenchInputAction, WorkbenchInputEvent, WorkbenchTerminalInputEvent,
    WorkbenchTerminalInputOutcome, apply_workbench_terminal_input_event,
    parse_workbench_input_event, parse_workbench_terminal_event,
    parse_workbench_terminal_key_event,
};
pub use self::state::{WorkbenchInputState, format_workbench_input_state};
