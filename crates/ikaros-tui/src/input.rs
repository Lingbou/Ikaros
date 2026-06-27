// SPDX-License-Identifier: GPL-3.0-only

use super::input_model::selected_palette_command;
use super::selection::*;
use super::*;

#[cfg(test)]
pub(super) fn parse_workbench_screen_state(args: &[&str]) -> Result<WorkbenchScreenState> {
    let mut state = WorkbenchScreenState::default();
    apply_workbench_screen_args(&mut state, args)?;
    Ok(state)
}

pub fn apply_workbench_screen_args(state: &mut WorkbenchScreenState, args: &[&str]) -> Result<()> {
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--focus" | "focus" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /screen --focus <status|timeline|main|side>"))?;
                state.focused = parse_panel(value)?;
                state.title_selection = None;
                index += 2;
            }
            "--focus-next" | "tab" => {
                state.apply(WorkbenchScreenAction::FocusNext);
                index += 1;
            }
            "--focus-prev" | "--focus-previous" | "shift-tab" => {
                state.apply(WorkbenchScreenAction::FocusPrevious);
                index += 1;
            }
            "--scroll" | "scroll" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /screen --scroll <lines>"))?;
                let scroll = parse_usize(value, "screen scroll")?;
                *state.focused_scroll_mut() = scroll;
                index += 2;
            }
            "--down" | "down" | "j" => {
                apply_screen_down_arg(state);
                index += 1;
            }
            "--up" | "up" | "k" => {
                apply_screen_up_arg(state);
                index += 1;
            }
            "--page-down" | "page-down" | "pgdn" => {
                apply_screen_page_down_arg(state);
                index += 1;
            }
            "--page-up" | "page-up" | "pgup" => {
                apply_screen_page_up_arg(state);
                index += 1;
            }
            "--top" | "top" | "home" => {
                apply_screen_top_arg(state);
                index += 1;
            }
            "--select" | "select" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /screen --select <row>"))?;
                *state.focused_selection_mut() =
                    parse_usize(value, "screen selection")?.saturating_sub(1);
                state.title_selection = None;
                state.action_selection = None;
                index += 2;
            }
            "--select-title" | "select-title" => {
                let (selector, next_index) =
                    parse_screen_selector(args, index + 1, "select-title")?;
                state.title_selection = Some(selector);
                state.action_selection = None;
                index = next_index;
            }
            "--select-kind" | "select-kind" => {
                let (selector, next_index) = parse_screen_selector(args, index + 1, "select-kind")?;
                state.title_selection = Some(selector);
                state.action_selection = None;
                index = next_index;
            }
            "--select-action" | "select-action" => {
                let (selector, next_index) =
                    parse_screen_selector(args, index + 1, "select-action")?;
                state.action_selection = Some(selector);
                state.title_selection = None;
                index = next_index;
            }
            "--select-next" | "select-next" => {
                state.apply(WorkbenchScreenAction::SelectNext);
                index += 1;
            }
            "--select-prev" | "--select-previous" | "select-prev" => {
                state.apply(WorkbenchScreenAction::SelectPrevious);
                index += 1;
            }
            "--select-first" | "select-first" => {
                state.apply(WorkbenchScreenAction::SelectFirst);
                index += 1;
            }
            "--fullscreen" | "fullscreen" => {
                state.fullscreen = true;
                index += 1;
            }
            "--inline" | "inline" => {
                state.fullscreen = false;
                index += 1;
            }
            "--raw" | "raw" => {
                state.raw_mode = true;
                index += 1;
            }
            "--rich" | "rich" => {
                state.raw_mode = false;
                index += 1;
            }
            "--palette" | "palette" => {
                let query = args
                    .get(index + 1)
                    .filter(|value| query_token_is_palette_filter(value))
                    .map(|value| (*value).to_owned());
                state.open_command_palette(query);
                index += if args
                    .get(index + 1)
                    .is_some_and(|value| query_token_is_palette_filter(value))
                {
                    2
                } else {
                    1
                };
            }
            "--palette-query" | "palette-query" => {
                let (query, next_index) = parse_screen_palette_query(args, index + 1)?;
                state.open_command_palette(Some(query));
                index = next_index;
            }
            "--close-palette" | "close-palette" => {
                state.close_command_palette();
                index += 1;
            }
            "approve-selected" | "--approve-selected" | "approve" | "a" => {
                state.approval_action = Some(WorkbenchScreenApprovalAction::Approve);
                index += 1;
            }
            "deny-selected" | "--deny-selected" | "deny" | "d" => {
                state.approval_action = Some(WorkbenchScreenApprovalAction::Deny);
                index += 1;
            }
            "cancel-selected" | "--cancel-selected" | "cancel" | "c" => {
                state.continuation_action = Some(WorkbenchScreenContinuationAction::Cancel);
                index += 1;
            }
            "clear-selected" | "--clear-selected" | "clear" | "x" => {
                state.input_action = Some(WorkbenchScreenInputAction::Clear);
                index += 1;
            }
            "open-selected" | "--open-selected" | "open" | "enter" => {
                state.open_action = Some(WorkbenchScreenOpenAction::OpenSelected);
                index += 1;
            }
            "confirm-selected" | "--confirm-selected" | "confirm" => {
                state.open_action = Some(WorkbenchScreenOpenAction::ConfirmSelected);
                index += 1;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /screen [--focus status|timeline|main|side] [--scroll N] [--select N] [--select-title title] [--select-kind kind] [--select-action command] [--palette [query]|--palette-query query|--close-palette] [--down|--up|--page-down|--page-up|--top] [--focus-next|--focus-prev] [--fullscreen|--inline] [--raw|--rich] [approve-selected|deny-selected|cancel-selected|clear-selected|open-selected|confirm-selected]"
                ));
            }
            unknown => {
                return Err(anyhow!(
                    "unknown /screen argument '{}'; expected --focus, --scroll, --select, --select-title, --select-kind, --select-action, --palette, --palette-query, --close-palette, --down, --up, --page-down, --page-up, --top, --focus-next, --focus-prev, --fullscreen, --inline, --raw, --rich, approve-selected, deny-selected, cancel-selected, clear-selected, open-selected, or confirm-selected",
                    terminal_inline(unknown)
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn apply_screen_down_arg(state: &mut WorkbenchScreenState) {
    if state.command_palette_open {
        state.select_next_palette_item();
    } else {
        state.apply(WorkbenchScreenAction::ScrollDown);
    }
}

pub(super) fn apply_screen_up_arg(state: &mut WorkbenchScreenState) {
    if state.command_palette_open {
        state.select_previous_palette_item();
    } else {
        state.apply(WorkbenchScreenAction::ScrollUp);
    }
}

pub(super) fn apply_screen_page_down_arg(state: &mut WorkbenchScreenState) {
    if state.command_palette_open {
        for _ in 0..5 {
            state.select_next_palette_item();
        }
    } else {
        state.apply(WorkbenchScreenAction::PageDown);
    }
}

pub(super) fn apply_screen_page_up_arg(state: &mut WorkbenchScreenState) {
    if state.command_palette_open {
        for _ in 0..5 {
            state.select_previous_palette_item();
        }
    } else {
        state.apply(WorkbenchScreenAction::PageUp);
    }
}

pub(super) fn apply_screen_top_arg(state: &mut WorkbenchScreenState) {
    if state.command_palette_open {
        state.command_palette_selection = 0;
        state.select_action("global_palette");
    } else {
        state.apply(WorkbenchScreenAction::ScrollTop);
    }
}

pub(super) fn query_token_is_palette_filter(value: &str) -> bool {
    !value.starts_with("--")
        && !matches!(
            value,
            "focus"
                | "scroll"
                | "select"
                | "select-title"
                | "select-kind"
                | "select-action"
                | "tab"
                | "shift-tab"
                | "down"
                | "j"
                | "up"
                | "k"
                | "page-down"
                | "pgdn"
                | "page-up"
                | "pgup"
                | "top"
                | "home"
                | "focus-next"
                | "focus-prev"
                | "focus-previous"
                | "select-next"
                | "select-prev"
                | "select-previous"
                | "select-first"
                | "fullscreen"
                | "inline"
                | "raw"
                | "rich"
                | "palette"
                | "palette-query"
                | "close-palette"
                | "approve"
                | "a"
                | "deny"
                | "d"
                | "cancel"
                | "c"
                | "clear"
                | "x"
                | "open"
                | "enter"
                | "confirm"
                | "open-selected"
                | "confirm-selected"
                | "approve-selected"
                | "deny-selected"
                | "cancel-selected"
                | "clear-selected"
        )
}

pub(super) fn parse_screen_palette_query(args: &[&str], start: usize) -> Result<(String, usize)> {
    let Some(first) = args.get(start) else {
        return Err(anyhow!("usage: /screen --palette-query <value>"));
    };
    if is_screen_palette_query_boundary(first) {
        return Err(anyhow!("usage: /screen --palette-query <value>"));
    }
    let mut parts = Vec::new();
    let mut index = start;
    while let Some(value) = args.get(index) {
        if is_screen_palette_query_boundary(value) {
            break;
        }
        parts.push(*value);
        index += 1;
    }
    Ok((terminal_inline(&parts.join(" ")), index))
}

pub(super) fn is_screen_palette_query_boundary(value: &str) -> bool {
    value.starts_with("--")
        || matches!(
            value,
            "approve-selected"
                | "deny-selected"
                | "cancel-selected"
                | "clear-selected"
                | "open-selected"
                | "confirm-selected"
        )
}

pub fn apply_workbench_screen_key_event(state: &mut WorkbenchScreenState, event: KeyEvent) -> bool {
    apply_workbench_screen_key_event_with_view(state, event, None, 0, 0)
}

pub fn apply_workbench_screen_key_event_with_view(
    state: &mut WorkbenchScreenState,
    event: KeyEvent,
    screen: Option<&WorkbenchScreen>,
    width: usize,
    height: usize,
) -> bool {
    if event.kind != KeyEventKind::Press {
        return false;
    }
    let control = event.modifiers.contains(KeyModifiers::CONTROL);
    match event.code {
        KeyCode::Esc => return state.clear_transient_selection(),
        KeyCode::Char('c' | 'C') if control => {
            if state.clear_transient_selection() {
                return true;
            }
        }
        KeyCode::F(2) => {
            state.raw_mode = !state.raw_mode;
            return true;
        }
        KeyCode::F(3) => {
            state.raw_mode = true;
            return true;
        }
        KeyCode::F(4) => {
            state.raw_mode = false;
            return true;
        }
        KeyCode::F(1) => {
            state.select_action("global_help");
            return true;
        }
        KeyCode::F(5) => {
            state.open_command_palette(None);
            return true;
        }
        _ => {}
    }
    if state.command_palette_open {
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            return match event.code {
                KeyCode::Char('u' | 'U') => {
                    state.clear_command_palette_query();
                    true
                }
                _ => false,
            };
        }
        if event.modifiers.contains(KeyModifiers::ALT) && event.code == KeyCode::Enter {
            if selected_palette_command(state).is_some() {
                state.open_action = Some(WorkbenchScreenOpenAction::ConfirmSelected);
            }
            return true;
        }
        if event.modifiers.contains(KeyModifiers::ALT) {
            return match event.code {
                KeyCode::Char('1') => {
                    state.focus_panel(WorkbenchScreenPanel::Status);
                    true
                }
                KeyCode::Char('2') => {
                    state.focus_panel(WorkbenchScreenPanel::Timeline);
                    true
                }
                KeyCode::Char('3') => {
                    state.focus_panel(WorkbenchScreenPanel::Main);
                    true
                }
                KeyCode::Char('4') => {
                    state.focus_panel(WorkbenchScreenPanel::Side);
                    true
                }
                _ => true,
            };
        }
        return match event.code {
            KeyCode::Up => {
                state.select_previous_palette_item();
                true
            }
            KeyCode::Down => {
                state.select_next_palette_item();
                true
            }
            KeyCode::Tab => {
                state.cycle_next_palette_item();
                true
            }
            KeyCode::PageUp => {
                for _ in 0..5 {
                    state.select_previous_palette_item();
                }
                true
            }
            KeyCode::PageDown => {
                for _ in 0..5 {
                    state.select_next_palette_item();
                }
                true
            }
            KeyCode::Home => {
                state.command_palette_selection = 0;
                state.select_action("global_palette");
                true
            }
            KeyCode::Enter => {
                if selected_palette_command(state).is_some() {
                    state.open_action = Some(WorkbenchScreenOpenAction::OpenSelected);
                }
                true
            }
            KeyCode::Backspace => {
                state.pop_command_palette_query_char();
                true
            }
            KeyCode::Char(ch) => {
                state.append_command_palette_query_char(ch);
                true
            }
            _ => false,
        };
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        return match event.code {
            KeyCode::Char('t' | 'T') if state.focused == WorkbenchScreenPanel::Timeline => {
                state.select_action("timeline_all");
                true
            }
            _ => false,
        };
    }
    let alt = event.modifiers.contains(KeyModifiers::ALT);
    if !state.raw_mode && !alt {
        match event.code {
            KeyCode::Up => {
                state.scroll_chat_history_up(1);
                return true;
            }
            KeyCode::Down => {
                state.scroll_chat_history_down(1, chat_max_scroll(screen, state, width, height));
                return true;
            }
            KeyCode::PageUp => {
                state.scroll_chat_history_up(10);
                return true;
            }
            KeyCode::PageDown => {
                state.scroll_chat_history_down(10, chat_max_scroll(screen, state, width, height));
                return true;
            }
            KeyCode::Home => {
                state.scroll_chat_history_top();
                return true;
            }
            KeyCode::End => {
                state.scroll_chat_history_bottom();
                return true;
            }
            _ => {}
        }
    }
    match event.code {
        KeyCode::Tab if !alt => state.apply(WorkbenchScreenAction::FocusNext),
        KeyCode::BackTab if !alt => state.apply(WorkbenchScreenAction::FocusPrevious),
        KeyCode::Down if !alt => state.apply(WorkbenchScreenAction::ScrollDown),
        KeyCode::Up if !alt => state.apply(WorkbenchScreenAction::ScrollUp),
        KeyCode::PageDown if !alt => state.apply(WorkbenchScreenAction::PageDown),
        KeyCode::PageUp if !alt => state.apply(WorkbenchScreenAction::PageUp),
        KeyCode::Home if !alt => state.apply(WorkbenchScreenAction::ScrollTop),
        KeyCode::Right if !alt => state.apply(WorkbenchScreenAction::SelectNext),
        KeyCode::Left if !alt => state.apply(WorkbenchScreenAction::SelectPrevious),
        KeyCode::Enter if !alt => state.open_action = Some(WorkbenchScreenOpenAction::OpenSelected),
        KeyCode::Enter if alt => {
            state.open_action = Some(WorkbenchScreenOpenAction::ConfirmSelected);
        }
        KeyCode::Char('j') if alt => state.apply(WorkbenchScreenAction::ScrollDown),
        KeyCode::Char('k') if alt => state.apply(WorkbenchScreenAction::ScrollUp),
        KeyCode::Char('l') if alt => state.apply(WorkbenchScreenAction::SelectNext),
        KeyCode::Char('h') if alt => state.apply(WorkbenchScreenAction::SelectPrevious),
        KeyCode::Char('1') if alt => state.focus_panel(WorkbenchScreenPanel::Status),
        KeyCode::Char('2') if alt => state.focus_panel(WorkbenchScreenPanel::Timeline),
        KeyCode::Char('3') if alt => state.focus_panel(WorkbenchScreenPanel::Main),
        KeyCode::Char('4') if alt => state.focus_panel(WorkbenchScreenPanel::Side),
        KeyCode::Char('i' | 'I') if alt => state.select_action("interrupt_cancel"),
        KeyCode::Char('m' | 'M') if alt => state.select_action("primary"),
        KeyCode::Char('r' | 'R') if alt => state.select_action("recovery_primary"),
        KeyCode::Char('o' | 'O') if alt => state.select_action("approval_approve"),
        KeyCode::Char('q' | 'Q') if alt => state.select_action("queue_cancel_all"),
        KeyCode::Char('a') if alt => {
            state.approval_action = Some(WorkbenchScreenApprovalAction::Approve);
        }
        KeyCode::Char('d') if alt => {
            state.approval_action = Some(WorkbenchScreenApprovalAction::Deny);
        }
        KeyCode::Char('c') if alt => {
            state.continuation_action = Some(WorkbenchScreenContinuationAction::Cancel);
        }
        KeyCode::Char('x') if alt => state.input_action = Some(WorkbenchScreenInputAction::Clear),
        _ => return false,
    }
    true
}

pub fn apply_workbench_screen_mouse_event(
    state: &mut WorkbenchScreenState,
    event: MouseEvent,
) -> bool {
    apply_workbench_screen_mouse_event_with_view(state, event, None, 0, 0)
}

pub fn apply_workbench_screen_mouse_event_with_view(
    state: &mut WorkbenchScreenState,
    event: MouseEvent,
    screen: Option<&WorkbenchScreen>,
    width: usize,
    height: usize,
) -> bool {
    match event.kind {
        MouseEventKind::ScrollUp => {
            state.scroll_chat_history_up(3);
            true
        }
        MouseEventKind::ScrollDown => {
            state.scroll_chat_history_down(3, chat_max_scroll(screen, state, width, height));
            true
        }
        _ => false,
    }
}

fn chat_max_scroll(
    screen: Option<&WorkbenchScreen>,
    state: &WorkbenchScreenState,
    width: usize,
    height: usize,
) -> Option<usize> {
    screen.map(|screen| render::chat_surface_max_scroll(screen, state, width, height))
}

pub(super) fn parse_panel(value: &str) -> Result<WorkbenchScreenPanel> {
    match value {
        "status" | "header" | "top" => Ok(WorkbenchScreenPanel::Status),
        "timeline" | "trace" | "replay" => Ok(WorkbenchScreenPanel::Timeline),
        "main" | "context" | "coding" => Ok(WorkbenchScreenPanel::Main),
        "side" | "approval" | "approvals" | "queue" => Ok(WorkbenchScreenPanel::Side),
        unknown => Err(anyhow!(
            "unknown screen focus panel '{}'; expected status, timeline, main, or side",
            terminal_inline(unknown)
        )),
    }
}

pub(super) fn parse_usize(value: &str, field: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .map_err(|_| anyhow!("{field} must be a non-negative number"))
}
