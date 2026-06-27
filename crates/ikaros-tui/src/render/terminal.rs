// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub fn render_tui_workbench_snapshot(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: u16,
    height: u16,
) -> Result<String> {
    let backend = TestBackend::new(width.max(40), height.max(10));
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| draw_tui_workbench_frame(frame, screen, state))?;
    Ok(buffer_snapshot(terminal.backend().buffer()))
}

pub fn render_fullscreen_terminal_frame(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> String {
    format!(
        "{}{}",
        render_fullscreen_terminal_envelope(screen, state, width, height),
        fullscreen_terminal_exit_sequence()
    )
}

fn render_fullscreen_terminal_envelope(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> String {
    let frame = render_tui_workbench_snapshot(screen, state, width as u16, height as u16)
        .unwrap_or_else(|_| render_fullscreen_workbench_with_state(screen, state, width, height));
    format!("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H{frame}")
}

fn fullscreen_terminal_exit_sequence() -> &'static str {
    "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1007l\x1b[?25h\x1b[?1049l"
}
