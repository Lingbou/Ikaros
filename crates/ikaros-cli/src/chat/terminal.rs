// SPDX-License-Identifier: GPL-3.0-only

use super::{
    interactive::InteractiveChatRuntime,
    workbench::{
        self, WorkbenchInputAction, WorkbenchInputEvent, WorkbenchInputState,
        WorkbenchScreenOpenAction, WorkbenchScreenState, WorkbenchTerminalInputOutcome,
        apply_workbench_terminal_input_event, format_workbench_input_state,
        parse_workbench_input_event, parse_workbench_terminal_event, terminal_inline,
    },
};
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event as CrosstermEvent, KeyCode,
        KeyEvent, KeyModifiers,
    },
    terminal::{disable_raw_mode, enable_raw_mode, size as terminal_size},
};
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_models::ModelUsageLedger;
use ikaros_runtime::ChatRunOptions;
use std::{
    io::{self, IsTerminal, Write},
    path::Path,
};

pub(super) const BRACKETED_PASTE_START: &str = "\u{1b}[200~";
const BRACKETED_PASTE_END: &str = "\u{1b}[201~";

pub(super) fn read_fullscreen_workbench_input(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
    input_state: &mut WorkbenchInputState,
    terminal: Option<&mut workbench::PersistentWorkbenchTerminal>,
) -> Result<Option<String>> {
    if !fullscreen_terminal_event_input_available() {
        eprintln!("fullscreen_input_mode: line fallback=non_tty");
        return read_fullscreen_workbench_line_input(input_state);
    }
    if terminal.is_none() {
        let _raw_mode = match FullscreenRawModeGuard::enable() {
            Ok(guard) => guard,
            Err(error) => {
                eprintln!(
                    "fullscreen_input_mode: line fallback=raw_unavailable reason={}",
                    terminal_inline(&error.to_string())
                );
                return read_fullscreen_workbench_line_input(input_state);
            }
        };
        return read_fullscreen_workbench_input_loop(
            config,
            paths,
            workspace,
            runtime,
            options,
            usage_ledger,
            input_state,
            None,
        );
    }
    read_fullscreen_workbench_input_loop(
        config,
        paths,
        workspace,
        runtime,
        options,
        usage_ledger,
        input_state,
        terminal,
    )
}

fn read_fullscreen_workbench_input_loop(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
    input_state: &mut WorkbenchInputState,
    mut terminal: Option<&mut workbench::PersistentWorkbenchTerminal>,
) -> Result<Option<String>> {
    let base_screen_cache = if terminal.is_some() {
        Some(workbench::build_screen_status(
            workbench::WorkbenchScreenStatusContext {
                config,
                paths,
                workspace,
                runtime,
                options,
                usage_ledger,
            },
        )?)
    } else {
        None
    };
    draw_or_print_fullscreen_workbench_input_state(
        workbench::WorkbenchScreenStatusContext {
            config,
            paths,
            workspace,
            runtime,
            options,
            usage_ledger,
        },
        &runtime.screen_state,
        input_state,
        terminal.as_deref_mut(),
        base_screen_cache.as_ref(),
    )?;
    io::stdout().flush()?;
    loop {
        let terminal_event = event::read()?;
        match &terminal_event {
            CrosstermEvent::Key(key) => {
                if input_state.buffer_is_empty() || fullscreen_screen_key_has_priority(*key) {
                    let screen = fullscreen_workbench_event_screen(
                        workbench::WorkbenchScreenStatusContext {
                            config,
                            paths,
                            workspace,
                            runtime,
                            options,
                            usage_ledger,
                        },
                        input_state,
                        base_screen_cache.as_ref(),
                    )?;
                    let (width, height) = fullscreen_workbench_event_size();
                    if workbench::apply_workbench_screen_key_event_with_view(
                        &mut runtime.screen_state,
                        *key,
                        Some(&screen),
                        width,
                        height,
                    ) {
                        if let Some(command) = fullscreen_screen_key_command(
                            config,
                            paths,
                            workspace,
                            runtime,
                            options,
                            usage_ledger,
                        )? {
                            return Ok(Some(command));
                        }
                        draw_or_print_fullscreen_workbench_input_state(
                            workbench::WorkbenchScreenStatusContext {
                                config,
                                paths,
                                workspace,
                                runtime,
                                options,
                                usage_ledger,
                            },
                            &runtime.screen_state,
                            input_state,
                            terminal.as_deref_mut(),
                            base_screen_cache.as_ref(),
                        )?;
                        io::stdout().flush()?;
                        continue;
                    }
                }
            }
            CrosstermEvent::Mouse(mouse) => {
                let screen = fullscreen_workbench_event_screen(
                    workbench::WorkbenchScreenStatusContext {
                        config,
                        paths,
                        workspace,
                        runtime,
                        options,
                        usage_ledger,
                    },
                    input_state,
                    base_screen_cache.as_ref(),
                )?;
                let (width, height) = fullscreen_workbench_event_size();
                if workbench::apply_workbench_screen_mouse_event_with_view(
                    &mut runtime.screen_state,
                    *mouse,
                    Some(&screen),
                    width,
                    height,
                ) {
                    draw_or_print_fullscreen_workbench_input_state(
                        workbench::WorkbenchScreenStatusContext {
                            config,
                            paths,
                            workspace,
                            runtime,
                            options,
                            usage_ledger,
                        },
                        &runtime.screen_state,
                        input_state,
                        terminal.as_deref_mut(),
                        base_screen_cache.as_ref(),
                    )?;
                    io::stdout().flush()?;
                    continue;
                }
            }
            CrosstermEvent::Resize(_, _) => {
                draw_or_print_fullscreen_workbench_input_state(
                    workbench::WorkbenchScreenStatusContext {
                        config,
                        paths,
                        workspace,
                        runtime,
                        options,
                        usage_ledger,
                    },
                    &runtime.screen_state,
                    input_state,
                    terminal.as_deref_mut(),
                    base_screen_cache.as_ref(),
                )?;
                io::stdout().flush()?;
                continue;
            }
            _ => {}
        }
        let Some(input_event) = parse_workbench_terminal_event(terminal_event) else {
            continue;
        };
        match apply_workbench_terminal_input_event(input_state, input_event) {
            WorkbenchTerminalInputOutcome::Pending => {
                draw_or_print_fullscreen_workbench_input_state(
                    workbench::WorkbenchScreenStatusContext {
                        config,
                        paths,
                        workspace,
                        runtime,
                        options,
                        usage_ledger,
                    },
                    &runtime.screen_state,
                    input_state,
                    terminal.as_deref_mut(),
                    base_screen_cache.as_ref(),
                )?;
                io::stdout().flush()?;
            }
            WorkbenchTerminalInputOutcome::Submit(input) => return Ok(Some(input)),
            WorkbenchTerminalInputOutcome::Exit => return Ok(None),
        }
    }
}

fn fullscreen_screen_key_has_priority(key: KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C')))
}

fn draw_or_print_fullscreen_workbench_input_state(
    context: workbench::WorkbenchScreenStatusContext<'_>,
    screen_state: &WorkbenchScreenState,
    input_state: &WorkbenchInputState,
    terminal: Option<&mut workbench::PersistentWorkbenchTerminal>,
    base_screen_cache: Option<&workbench::WorkbenchScreen>,
) -> Result<()> {
    if let Some(terminal) = terminal {
        if let Some(base_screen) = base_screen_cache {
            let mut screen = base_screen.clone();
            workbench::apply_input_state_to_cached_screen(&mut screen, input_state);
            return terminal.draw(&screen, screen_state);
        }
        return workbench::draw_persistent_screen_status_with_input_state(
            context,
            screen_state,
            input_state,
            terminal,
        );
    }
    workbench::print_persistent_screen_status_with_input_state(context, screen_state, input_state)
}

fn fullscreen_workbench_event_screen(
    context: workbench::WorkbenchScreenStatusContext<'_>,
    input_state: &WorkbenchInputState,
    base_screen_cache: Option<&workbench::WorkbenchScreen>,
) -> Result<workbench::WorkbenchScreen> {
    let mut screen = if let Some(base_screen) = base_screen_cache {
        base_screen.clone()
    } else {
        workbench::build_screen_status(context)?
    };
    workbench::apply_input_state_to_cached_screen(&mut screen, input_state);
    Ok(screen)
}

fn fullscreen_workbench_event_size() -> (usize, usize) {
    terminal_size()
        .map(|(width, height)| (usize::from(width), usize::from(height)))
        .unwrap_or((80, 24))
}

pub(super) fn fullscreen_terminal_event_input_available() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn fullscreen_screen_key_command(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<Option<String>> {
    match take_fullscreen_screen_action(&mut runtime.screen_state) {
        FullscreenScreenAction::Refresh => Ok(None),
        FullscreenScreenAction::Command(command) => Ok(Some(command)),
        FullscreenScreenAction::OpenSelected => {
            let command = workbench::selected_screen_primary_action(
                config,
                paths,
                workspace,
                runtime,
                options,
                usage_ledger,
                &runtime.screen_state,
            )?;
            Ok(command.or_else(|| Some("/screen".into())))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FullscreenScreenAction {
    Refresh,
    OpenSelected,
    Command(String),
}

pub(super) fn take_fullscreen_screen_action(
    state: &mut WorkbenchScreenState,
) -> FullscreenScreenAction {
    if state.take_approval_action().is_some() {
        return FullscreenScreenAction::Command("/screen approve-selected".into());
    }
    if state.take_continuation_action().is_some() {
        return FullscreenScreenAction::Command("/screen cancel-selected".into());
    }
    if state.take_input_action().is_some() {
        return FullscreenScreenAction::Command("/screen clear-selected".into());
    }
    if let Some(action) = state.take_open_action() {
        return match action {
            WorkbenchScreenOpenAction::OpenSelected if state.command_palette_open() => {
                let command = state.selected_command_palette_command();
                state.close_command_palette();
                command
                    .map(FullscreenScreenAction::Command)
                    .unwrap_or(FullscreenScreenAction::Refresh)
            }
            WorkbenchScreenOpenAction::OpenSelected => FullscreenScreenAction::OpenSelected,
            WorkbenchScreenOpenAction::ConfirmSelected => {
                FullscreenScreenAction::Command("/screen confirm-selected".into())
            }
        };
    }
    FullscreenScreenAction::Refresh
}

fn read_fullscreen_workbench_line_input(
    input_state: &mut WorkbenchInputState,
) -> Result<Option<String>> {
    let mut line = String::new();
    print!("ikaros> ");
    io::stdout().flush()?;
    if io::stdin().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let input = line.trim_end_matches(['\n', '\r']).to_owned();
    input_state.set_buffer("");
    Ok(Some(input))
}

pub(super) fn read_workbench_terminal_line_input(
    input_state: &mut WorkbenchInputState,
    fallback_line: &mut String,
) -> Result<Option<String>> {
    if !fullscreen_terminal_event_input_available() {
        return read_workbench_stdio_line_input(fallback_line);
    }
    let _raw_mode = match FullscreenRawModeGuard::enable() {
        Ok(guard) => guard,
        Err(error) => {
            println!(
                "workbench_input_mode: line fallback=raw_unavailable reason={}",
                terminal_inline(&error.to_string())
            );
            return read_workbench_stdio_line_input(fallback_line);
        }
    };
    input_state.set_buffer("");
    render_workbench_line_editor(input_state)?;
    loop {
        let terminal_event = event::read()?;
        let Some(input_event) = parse_workbench_terminal_event(terminal_event) else {
            continue;
        };
        match apply_workbench_terminal_input_event(input_state, input_event) {
            WorkbenchTerminalInputOutcome::Pending => render_workbench_line_editor(input_state)?,
            WorkbenchTerminalInputOutcome::Submit(input) => {
                println!();
                return Ok(Some(input));
            }
            WorkbenchTerminalInputOutcome::Exit => {
                println!();
                return Ok(None);
            }
        }
    }
}

fn read_workbench_stdio_line_input(fallback_line: &mut String) -> Result<Option<String>> {
    print!("ikaros> ");
    io::stdout().flush()?;
    fallback_line.clear();
    if io::stdin().read_line(fallback_line)? == 0 {
        return Ok(None);
    }
    Ok(Some(
        fallback_line.trim_end_matches(['\n', '\r']).to_owned(),
    ))
}

fn render_workbench_line_editor(input_state: &WorkbenchInputState) -> Result<()> {
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
        " multi"
    } else if input_state.history_search_active() {
        " search"
    } else {
        ""
    };
    print!(
        "\r\x1b[2Kikaros{}> {}{}{}",
        dirty_marker,
        input_state.cursor_view(),
        completion_hint,
        history_search_hint
    );
    io::stdout().flush()?;
    Ok(())
}

pub(super) fn handle_workbench_input_control(input: &str, state: &mut WorkbenchInputState) -> bool {
    match parse_workbench_input_event(input) {
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistoryPrevious) => {
            if let Some(selected) = state.apply(WorkbenchInputAction::HistoryPrevious) {
                println!("input_history_selected: {}", terminal_inline(&selected));
            } else {
                println!("input_history_selected: none");
            }
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistoryNext) => {
            if let Some(selected) = state.apply(WorkbenchInputAction::HistoryNext) {
                println!("input_history_selected: {}", terminal_inline(&selected));
            } else {
                println!("input_history_selected: none");
            }
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistorySearchStart) => {
            print_input_edit_state(
                "history_search_start",
                state,
                WorkbenchInputAction::HistorySearchStart,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistorySearchPrevious) => {
            print_input_edit_state(
                "history_search_previous",
                state,
                WorkbenchInputAction::HistorySearchPrevious,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistorySearchNext) => {
            print_input_edit_state(
                "history_search_next",
                state,
                WorkbenchInputAction::HistorySearchNext,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveLeft) => {
            print_input_edit_state("move_left", state, WorkbenchInputAction::MoveLeft);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveRight) => {
            print_input_edit_state("move_right", state, WorkbenchInputAction::MoveRight);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveWordLeft) => {
            print_input_edit_state("move_word_left", state, WorkbenchInputAction::MoveWordLeft);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveWordRight) => {
            print_input_edit_state(
                "move_word_right",
                state,
                WorkbenchInputAction::MoveWordRight,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveStart) => {
            print_input_edit_state("move_start", state, WorkbenchInputAction::MoveStart);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveEnd) => {
            print_input_edit_state("move_end", state, WorkbenchInputAction::MoveEnd);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeletePrevious) => {
            print_input_edit_state(
                "delete_previous",
                state,
                WorkbenchInputAction::DeletePrevious,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteNext) => {
            print_input_edit_state("delete_next", state, WorkbenchInputAction::DeleteNext);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeletePreviousWord) => {
            print_input_edit_state(
                "delete_previous_word",
                state,
                WorkbenchInputAction::DeletePreviousWord,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteNextWord) => {
            print_input_edit_state(
                "delete_next_word",
                state,
                WorkbenchInputAction::DeleteNextWord,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteBeforeCursor) => {
            print_input_edit_state(
                "delete_before_cursor",
                state,
                WorkbenchInputAction::DeleteBeforeCursor,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteAfterCursor) => {
            print_input_edit_state(
                "delete_after_cursor",
                state,
                WorkbenchInputAction::DeleteAfterCursor,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::Undo) => {
            print_input_edit_state("undo", state, WorkbenchInputAction::Undo);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::Redo) => {
            print_input_edit_state("redo", state, WorkbenchInputAction::Redo);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::Refresh) => {
            print_input_edit_state("refresh", state, WorkbenchInputAction::Refresh);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::Complete) => {
            print_input_edit_state("complete", state, WorkbenchInputAction::Complete);
            true
        }
        WorkbenchInputEvent::CompletePrefix(prefix) => {
            state.set_buffer("");
            state.insert_text(&prefix);
            if let Some(completed) = state.apply(WorkbenchInputAction::Complete) {
                println!("input_completion: {}", terminal_inline(&completed));
            } else {
                println!("input_completion: none");
            }
            true
        }
        WorkbenchInputEvent::SubmitLine(_) => false,
    }
}

fn print_input_edit_state(
    action: &str,
    state: &mut WorkbenchInputState,
    input_action: WorkbenchInputAction,
) {
    let buffer = state.apply(input_action).unwrap_or_default();
    println!(
        "input_edit: action={} cursor={} buffer={}",
        action,
        state.cursor(),
        terminal_inline(&buffer)
    );
    println!("{}", format_workbench_input_state(action, state));
}

pub(super) fn read_multiline_message() -> Result<Option<String>> {
    let mut message = String::new();
    let mut line = String::new();
    loop {
        print!("... ");
        io::stdout().flush()?;
        line.clear();
        if io::stdin().read_line(&mut line)? == 0 {
            return if message.is_empty() {
                Ok(None)
            } else {
                Ok(Some(message))
            };
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == workbench::MULTILINE_TERMINATOR {
            return Ok(Some(message));
        }
        message.push_str(trimmed);
        message.push('\n');
    }
}

pub(super) fn read_bracketed_paste_message(first_line: &str) -> Result<Option<String>> {
    let mut message = String::new();
    let mut current = first_line
        .strip_prefix(BRACKETED_PASTE_START)
        .unwrap_or(first_line)
        .to_owned();
    loop {
        if let Some((before_end, _after_end)) = current.split_once(BRACKETED_PASTE_END) {
            message.push_str(before_end.trim_end_matches(['\n', '\r']));
            return Ok(Some(message));
        }
        message.push_str(current.trim_end_matches(['\n', '\r']));
        message.push('\n');
        current.clear();
        if io::stdin().read_line(&mut current)? == 0 {
            return if message.is_empty() {
                Ok(None)
            } else {
                Ok(Some(message))
            };
        }
    }
}

#[allow(dead_code)]
pub(super) struct FullscreenTerminalGuard;

impl Drop for FullscreenTerminalGuard {
    fn drop(&mut self) {
        print!("{}", workbench::fullscreen_terminal_exit_sequence());
        let _ = io::stdout().flush();
    }
}

struct FullscreenRawModeGuard;

impl FullscreenRawModeGuard {
    fn enable() -> Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = crossterm::execute!(io::stdout(), EnableBracketedPaste) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        Ok(Self)
    }
}

impl Drop for FullscreenRawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(io::stdout(), DisableBracketedPaste);
        let _ = disable_raw_mode();
    }
}
