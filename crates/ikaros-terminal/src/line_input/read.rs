// SPDX-License-Identifier: GPL-3.0-only

use super::{
    inline_composer::render_workbench_inline_composer, intro::render_workbench_inline_intro,
    render_state::WorkbenchLineEditorRenderState, submitted::insert_submitted_user_message,
    ui::WorkbenchLineInputUi,
};
use crate::{
    FullscreenRawModeGuard, WorkbenchInputState, WorkbenchTerminalInputOutcome,
    apply_workbench_terminal_input_event, disable_mouse_tracking_best_effort,
    fullscreen_terminal_event_input_available, parse_workbench_terminal_event, terminal_inline,
};
use anyhow::Result;
use crossterm::event::{self, Event as CrosstermEvent};
use std::io::{self, Write};
pub fn read_workbench_terminal_line_input(
    input_state: &mut WorkbenchInputState,
    fallback_line: &mut String,
    ui: Option<&WorkbenchLineInputUi>,
    terminal_modes_already_enabled: bool,
    render_state: &mut WorkbenchLineEditorRenderState,
) -> Result<Option<String>> {
    if !fullscreen_terminal_event_input_available() {
        return read_workbench_stdio_line_input(fallback_line);
    }
    disable_mouse_tracking_best_effort();
    if let Some(ui) = ui.filter(|ui| ui.show_intro) {
        render_workbench_inline_intro(ui)?;
    }
    let _raw_mode =
        match FullscreenRawModeGuard::enable_for_line_input(!terminal_modes_already_enabled) {
            Ok(guard) => guard,
            Err(error) => {
                if ui.is_none() {
                    println!(
                        "workbench_input_mode: line fallback=raw_unavailable reason={}",
                        terminal_inline(&error.to_string())
                    );
                }
                return read_workbench_stdio_line_input(fallback_line);
            }
        };
    input_state.set_buffer("");
    render_workbench_line_editor(input_state, ui, render_state)?;
    loop {
        let terminal_event = event::read()?;
        if matches!(terminal_event, CrosstermEvent::Resize(_, _)) {
            render_workbench_line_editor(input_state, ui, render_state)?;
            continue;
        }
        let Some(input_event) = parse_workbench_terminal_event(terminal_event) else {
            continue;
        };
        match apply_workbench_terminal_input_event(input_state, input_event) {
            WorkbenchTerminalInputOutcome::Pending => {
                render_workbench_line_editor(input_state, ui, render_state)?;
            }
            WorkbenchTerminalInputOutcome::Submit(input) => {
                render_state.clear_rows_before_history_insert()?;
                if ui.is_some() {
                    insert_submitted_user_message(&input)?;
                } else {
                    println!();
                }
                return Ok(Some(input));
            }
            WorkbenchTerminalInputOutcome::Exit => {
                render_state.clear_rows_before_history_insert()?;
                if ui.is_none() {
                    println!();
                }
                return Ok(None);
            }
        }
    }
}

fn read_workbench_stdio_line_input(fallback_line: &mut String) -> Result<Option<String>> {
    disable_mouse_tracking_best_effort();
    print!("* ");
    io::stdout().flush()?;
    fallback_line.clear();
    if io::stdin().read_line(fallback_line)? == 0 {
        return Ok(None);
    }
    Ok(Some(
        fallback_line.trim_end_matches(['\n', '\r']).to_owned(),
    ))
}

fn render_workbench_line_editor(
    input_state: &WorkbenchInputState,
    ui: Option<&WorkbenchLineInputUi>,
    state: &mut WorkbenchLineEditorRenderState,
) -> Result<()> {
    if let Some(ui) = ui {
        return render_workbench_inline_composer(input_state, ui, state);
    }
    let completions = input_state.completion_candidates();
    let completion_hint = if completions.is_empty() {
        String::new()
    } else {
        format!("  [tab: {}]", terminal_inline(&completions.join(", ")))
    };
    let history_search_hint = if input_state.history_search_active() {
        let candidates = input_state.history_search_candidates(3);
        let candidates = if candidates.is_empty() {
            "none".into()
        } else {
            terminal_inline(&candidates.join(" | "))
        };
        format!(
            "  [ctrl-r: {} matches={}]",
            terminal_inline(&input_state.history_search_summary()),
            candidates
        )
    } else {
        String::new()
    };
    let dirty_marker = if input_state.buffer().contains('\n') {
        " [multi]"
    } else if input_state.history_search_active() {
        " [search]"
    } else {
        ""
    };
    print!(
        "\r\x1b[2K›{} {}{}{}",
        dirty_marker,
        input_state.cursor_view(),
        completion_hint,
        history_search_hint
    );
    io::stdout().flush()?;
    state.rows = 1;
    Ok(())
}
