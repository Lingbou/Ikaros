// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat) fn render_tui_workbench_snapshot(
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

pub(in crate::chat) struct PersistentWorkbenchTerminal {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl PersistentWorkbenchTerminal {
    pub(in crate::chat) fn enter() -> Result<Option<Self>> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Ok(None);
        }
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            Hide,
            EnableBracketedPaste,
            EnableMouseCapture
        ) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        Ok(Some(Self { terminal }))
    }

    pub(in crate::chat) fn draw(
        &mut self,
        screen: &WorkbenchScreen,
        state: &WorkbenchScreenState,
    ) -> Result<()> {
        self.terminal
            .draw(|frame| draw_tui_workbench_frame(frame, screen, state))?;
        Ok(())
    }
}

impl Drop for PersistentWorkbenchTerminal {
    fn drop(&mut self) {
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            DisableMouseCapture,
            Show,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
    }
}

pub(in crate::chat) fn render_fullscreen_terminal_frame(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> String {
    format!(
        "{}{}",
        render_persistent_fullscreen_terminal_frame(screen, state, width, height),
        fullscreen_terminal_exit_sequence()
    )
}

pub(in crate::chat) fn render_persistent_fullscreen_terminal_frame(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> String {
    let frame = render_tui_workbench_snapshot(screen, state, width as u16, height as u16)
        .unwrap_or_else(|_| render_fullscreen_workbench_with_state(screen, state, width, height));
    format!("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H{frame}")
}

pub(in crate::chat) fn draw_persistent_fullscreen_terminal_frame(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> Result<bool> {
    if !io::stdout().is_terminal() {
        return Ok(false);
    }
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    terminal.draw(|frame| draw_tui_workbench_frame(frame, screen, state))?;
    Ok(true)
}

pub(in crate::chat) fn fullscreen_terminal_exit_sequence() -> &'static str {
    "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?25h\x1b[?1049l"
}
