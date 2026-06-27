// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use super::{actions::*, input_model::*, layout::*, panels::*, selection::*, surface::*};

mod bottom;
mod chat;
mod export;
mod frame;
mod overlay;
mod terminal;

pub(in crate::chat::workbench::screen) use bottom::*;
pub(in crate::chat::workbench::screen) use chat::*;
pub(in crate::chat::workbench::screen) use export::*;
pub(in crate::chat) use export::{
    screen_json_line, screen_selected_actions_json_line, screen_selected_actions_line,
    screen_selected_cell_line,
};
pub(in crate::chat) use frame::{draw_tui_workbench_frame, render_fullscreen_workbench_with_state};
pub(in crate::chat::workbench::screen) use overlay::*;
#[cfg(test)]
pub(in crate::chat) use terminal::render_tui_workbench_snapshot;
pub(in crate::chat) use terminal::{
    PersistentWorkbenchTerminal, draw_persistent_fullscreen_terminal_frame,
    fullscreen_terminal_exit_sequence, render_fullscreen_terminal_frame,
    render_persistent_fullscreen_terminal_frame,
};
