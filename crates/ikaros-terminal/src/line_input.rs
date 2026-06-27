// SPDX-License-Identifier: GPL-3.0-only

mod display;
mod history;
mod inline_composer;
mod intro;
mod popup;
mod read;
mod render_state;
mod submitted;
mod ui;

const INLINE_COMPOSER_HINT_LIMIT: usize = 5;

pub use self::history::{
    clear_visible_terminal, print_inline_history_lines, print_inline_history_text,
    print_inline_turn_separator, print_inline_turn_worked_separator,
};
pub use self::read::read_workbench_terminal_line_input;
pub use self::render_state::WorkbenchLineEditorRenderState;
pub use self::ui::WorkbenchLineInputUi;
