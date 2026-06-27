// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, anyhow};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

mod actions;
mod activity;
mod body;
mod composer;
mod input;
mod input_model;
mod input_read;
mod layout;
mod line_input;
mod markdown;
mod notice;
mod panels;
mod progress;
mod render;
mod running_turn;
mod sanitize;
mod selection;
mod slash;
mod slash_popup;
mod stream;
mod streaming;
mod surface;
mod terminal_control;
#[cfg(test)]
mod tests;
mod text;
mod transcript;
mod workbench_cell;
mod workbench_input;
mod workbench_screen;
mod workbench_status;

use actions::action_selection_is_command_palette;
use input_model::selected_palette_command;

#[cfg(test)]
use input::parse_workbench_screen_state;
#[cfg(test)]
use input_model::command_palette_overlay_json;
#[cfg(test)]
use panels::screen_queue_panel_json;

#[cfg(test)]
pub(crate) use actions::action_menu_queue_items_json;
pub use activity::{
    ToolActivity, ToolActivityStatus, human_activity_line_for_terminal, render_tool_activity,
};
pub use body::{BodyAdapter, CliBodyAdapter};
pub use composer::{ComposerAction, ComposerOutcome, ComposerState, TerminalMode};
pub use input::apply_workbench_screen_args;
pub use input::{
    apply_workbench_screen_key_event, apply_workbench_screen_key_event_with_view,
    apply_workbench_screen_mouse_event, apply_workbench_screen_mouse_event_with_view,
};
pub use input_read::{
    BRACKETED_PASTE_START, MULTILINE_TERMINATOR, read_bracketed_paste_message,
    read_multiline_message,
};
pub use line_input::{
    WorkbenchLineEditorRenderState, WorkbenchLineInputUi, clear_visible_terminal,
    print_inline_history_lines, print_inline_history_text, print_inline_turn_separator,
    print_inline_turn_worked_separator, read_workbench_terminal_line_input,
};
pub use markdown::{
    TerminalMarkdownRenderer, color_assistant_bullet_for_terminal, is_markdown_table_line,
    markdown_heading, render_assistant_markdown_transcript,
    render_assistant_markdown_transcript_for_current_width, render_terminal_markdown,
    render_terminal_markdown_for_current_width, render_terminal_markdown_lines,
};
pub use notice::{WorkbenchNotice, WorkbenchNoticeKind};
pub use progress::{WorkbenchProgressSnapshot, progress_bar, progress_phase, progress_spinner};
#[cfg(test)]
pub(crate) use render::render_tui_workbench_snapshot;
pub use render::{
    render_fullscreen_terminal_frame, render_fullscreen_workbench_with_state, screen_json_line,
    screen_selected_actions_json_line, screen_selected_actions_line, screen_selected_cell_line,
};
pub use running_turn::{
    RunningTurnInputCapture, RunningTurnTerminal, normalize_raw_terminal_newlines,
};
pub use sanitize::{terminal_inline, terminal_message};
pub use selection::{command_requires_explicit_action, screen_selected_primary_action};
pub use slash::{
    SlashCommandCompletion, SlashCommandPaletteItem, SlashCommandPaletteSummary,
    format_slash_command_help, print_slash_commands, print_slash_commands_for_human,
    slash_command_completion_candidates, slash_command_palette_items,
    slash_command_palette_summary, slash_command_registry_summary, slash_commands_human_lines,
    slash_completion_query, suggest_slash_command,
};
pub use slash_popup::{
    SlashCommandItem, SlashCommandPopup, SlashCommandPopupState, SlashCommandSpec,
};
pub use stream::{
    AgentTextDeltaState, finish_agent_text_delta, format_agent_text_delta, terminal_width,
};
pub use streaming::{MarkdownStreamCollector, StreamFinish, TerminalStreamRenderer};
pub use terminal_control::{
    FullscreenRawModeGuard, ResetScrollRegion, SetScrollRegion, WorkbenchTerminalInputSessionGuard,
    disable_mouse_tracking_best_effort, fullscreen_terminal_event_input_available,
};
pub use transcript::{
    AssistantMarkdownCell, SeparatorCell, StreamingTailCell, ToolActivityCell, TranscriptCell,
    UserCell,
};
pub use workbench_cell::{WorkbenchCell, WorkbenchCellKind, render_workbench_snapshot};
pub use workbench_input::{
    WorkbenchInputAction, WorkbenchInputEvent, WorkbenchInputState, WorkbenchTerminalInputEvent,
    WorkbenchTerminalInputOutcome, apply_workbench_terminal_input_event,
    format_workbench_input_state, handle_workbench_input_control, parse_workbench_input_event,
    parse_workbench_terminal_event, parse_workbench_terminal_key_event,
};
pub use workbench_screen::{
    WorkbenchScreen, WorkbenchScreenAction, WorkbenchScreenApprovalAction,
    WorkbenchScreenContinuationAction, WorkbenchScreenInputAction, WorkbenchScreenOpenAction,
    WorkbenchScreenPanel,
};
pub use workbench_status::{
    progress_footer_summary, screen_active_work_cells, screen_command_palette_cells,
    screen_progress_status_cell, workbench_screen_dimensions_from_values,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchScreenState {
    focused: WorkbenchScreenPanel,
    status_scroll: usize,
    status_selection: usize,
    timeline_scroll: usize,
    timeline_selection: usize,
    main_scroll: usize,
    main_selection: usize,
    side_scroll: usize,
    side_selection: usize,
    fullscreen: bool,
    title_selection: Option<String>,
    action_selection: Option<String>,
    approval_action: Option<WorkbenchScreenApprovalAction>,
    continuation_action: Option<WorkbenchScreenContinuationAction>,
    input_action: Option<WorkbenchScreenInputAction>,
    open_action: Option<WorkbenchScreenOpenAction>,
    command_palette_open: bool,
    command_palette_query: Option<String>,
    command_palette_selection: usize,
    raw_mode: bool,
}

impl Default for WorkbenchScreenState {
    fn default() -> Self {
        Self {
            focused: WorkbenchScreenPanel::Timeline,
            status_scroll: 0,
            status_selection: 0,
            timeline_scroll: 0,
            timeline_selection: 0,
            main_scroll: 0,
            main_selection: 0,
            side_scroll: 0,
            side_selection: 0,
            fullscreen: false,
            title_selection: None,
            action_selection: None,
            approval_action: None,
            continuation_action: None,
            input_action: None,
            open_action: None,
            command_palette_open: false,
            command_palette_query: None,
            command_palette_selection: 0,
            raw_mode: false,
        }
    }
}

impl WorkbenchScreenState {
    pub fn focused_panel(&self) -> WorkbenchScreenPanel {
        self.focused
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub fn raw_mode(&self) -> bool {
        self.raw_mode
    }

    pub fn side_selection(&self) -> usize {
        self.side_selection
    }

    pub fn command_palette_open(&self) -> bool {
        self.command_palette_open
    }

    pub fn close_command_palette(&mut self) -> bool {
        self.close_command_palette_state()
    }

    pub fn selected_command_palette_command(&self) -> Option<String> {
        selected_palette_command(self)
    }

    pub fn take_approval_action(&mut self) -> Option<WorkbenchScreenApprovalAction> {
        self.approval_action.take()
    }

    pub fn take_continuation_action(&mut self) -> Option<WorkbenchScreenContinuationAction> {
        self.continuation_action.take()
    }

    pub fn take_input_action(&mut self) -> Option<WorkbenchScreenInputAction> {
        self.input_action.take()
    }

    pub fn take_open_action(&mut self) -> Option<WorkbenchScreenOpenAction> {
        self.open_action.take()
    }

    fn select_action(&mut self, selector: &str) {
        self.action_selection = Some(selector.to_owned());
        self.title_selection = None;
    }

    fn open_command_palette(&mut self, query: Option<String>) {
        self.command_palette_open = true;
        self.command_palette_query = query
            .map(|query| terminal_inline(query.trim()))
            .filter(|query| !query.is_empty());
        self.command_palette_selection = 0;
        self.select_action("global_palette");
    }

    fn set_command_palette_query(&mut self, query: String) {
        self.command_palette_open = true;
        self.command_palette_query =
            Some(terminal_inline(&query)).filter(|query| !query.is_empty());
        self.command_palette_selection = 0;
        self.select_action("global_palette");
    }

    fn append_command_palette_query_char(&mut self, ch: char) -> bool {
        if ch.is_control() {
            return false;
        }
        let mut query = self.command_palette_query.clone().unwrap_or_default();
        query.push(ch);
        self.set_command_palette_query(query);
        true
    }

    fn pop_command_palette_query_char(&mut self) -> bool {
        let Some(mut query) = self.command_palette_query.clone() else {
            return false;
        };
        let removed = query.pop().is_some();
        self.set_command_palette_query(query);
        removed
    }

    fn clear_command_palette_query(&mut self) -> bool {
        let had_query = self.command_palette_query.take().is_some();
        self.command_palette_open = true;
        self.command_palette_selection = 0;
        self.select_action("global_palette");
        had_query
    }

    fn close_command_palette_state(&mut self) -> bool {
        let was_open = self.command_palette_open || self.command_palette_query.is_some();
        let had_palette_action_selection = self
            .action_selection
            .as_deref()
            .is_some_and(action_selection_is_command_palette);
        self.command_palette_open = false;
        self.command_palette_query = None;
        self.command_palette_selection = 0;
        if had_palette_action_selection {
            self.action_selection = None;
        }
        was_open || had_palette_action_selection
    }

    fn select_next_palette_item(&mut self) {
        self.command_palette_open = true;
        let len = self.command_palette_len();
        self.command_palette_selection = if len == 0 {
            0
        } else {
            self.command_palette_selection
                .saturating_add(1)
                .min(len.saturating_sub(1))
        };
        self.select_action("global_palette");
    }

    fn cycle_next_palette_item(&mut self) {
        self.command_palette_open = true;
        let len = self.command_palette_len();
        self.command_palette_selection = if len == 0 {
            0
        } else {
            self.command_palette_selection.saturating_add(1) % len
        };
        self.select_action("global_palette");
    }

    fn select_previous_palette_item(&mut self) {
        self.command_palette_open = true;
        self.command_palette_selection = self.command_palette_selection.saturating_sub(1);
        self.select_action("global_palette");
    }

    fn command_palette_len(&self) -> usize {
        slash_command_palette_items(self.command_palette_query.as_deref(), 12).len()
    }

    fn focus_panel(&mut self, panel: WorkbenchScreenPanel) {
        self.focused = panel;
        self.title_selection = None;
        self.action_selection = None;
    }

    fn clear_transient_selection(&mut self) -> bool {
        let had_title_selection = self.title_selection.take().is_some();
        let had_action_selection = self.action_selection.take().is_some();
        let had_open_action = self.open_action.take().is_some();
        let had_approval_action = self.approval_action.take().is_some();
        let had_continuation_action = self.continuation_action.take().is_some();
        let had_input_action = self.input_action.take().is_some();
        let had_palette = self.close_command_palette_state();
        had_title_selection
            || had_action_selection
            || had_open_action
            || had_approval_action
            || had_continuation_action
            || had_input_action
            || had_palette
    }

    pub fn apply(&mut self, action: WorkbenchScreenAction) {
        match action {
            WorkbenchScreenAction::FocusNext => {
                self.focused = self.focused.next();
                self.title_selection = None;
                self.action_selection = None;
            }
            WorkbenchScreenAction::FocusPrevious => {
                self.focused = self.focused.previous();
                self.title_selection = None;
                self.action_selection = None;
            }
            WorkbenchScreenAction::ScrollDown => *self.focused_scroll_mut() += 1,
            WorkbenchScreenAction::ScrollUp => {
                let scroll = self.focused_scroll_mut();
                *scroll = scroll.saturating_sub(1);
            }
            WorkbenchScreenAction::PageDown => *self.focused_scroll_mut() += 10,
            WorkbenchScreenAction::PageUp => {
                let scroll = self.focused_scroll_mut();
                *scroll = scroll.saturating_sub(10);
            }
            WorkbenchScreenAction::ScrollTop => {
                *self.focused_scroll_mut() = 0;
                *self.focused_selection_mut() = 0;
                self.title_selection = None;
                self.action_selection = None;
            }
            WorkbenchScreenAction::SelectNext => {
                *self.focused_selection_mut() += 1;
                self.title_selection = None;
                self.action_selection = None;
            }
            WorkbenchScreenAction::SelectPrevious => {
                let selection = self.focused_selection_mut();
                *selection = selection.saturating_sub(1);
                self.title_selection = None;
                self.action_selection = None;
            }
            WorkbenchScreenAction::SelectFirst => {
                *self.focused_selection_mut() = 0;
                self.title_selection = None;
                self.action_selection = None;
            }
        }
    }

    fn focused_scroll_mut(&mut self) -> &mut usize {
        match self.focused {
            WorkbenchScreenPanel::Status => &mut self.status_scroll,
            WorkbenchScreenPanel::Timeline => &mut self.timeline_scroll,
            WorkbenchScreenPanel::Main => &mut self.main_scroll,
            WorkbenchScreenPanel::Side => &mut self.side_scroll,
        }
    }

    fn focused_selection_mut(&mut self) -> &mut usize {
        match self.focused {
            WorkbenchScreenPanel::Status => &mut self.status_selection,
            WorkbenchScreenPanel::Timeline => &mut self.timeline_selection,
            WorkbenchScreenPanel::Main => &mut self.main_selection,
            WorkbenchScreenPanel::Side => &mut self.side_selection,
        }
    }

    fn scroll_for(&self, panel: WorkbenchScreenPanel) -> usize {
        match panel {
            WorkbenchScreenPanel::Status => self.status_scroll,
            WorkbenchScreenPanel::Timeline => self.timeline_scroll,
            WorkbenchScreenPanel::Main => self.main_scroll,
            WorkbenchScreenPanel::Side => self.side_scroll,
        }
    }

    fn selection_for(&self, panel: WorkbenchScreenPanel) -> usize {
        match panel {
            WorkbenchScreenPanel::Status => self.status_selection,
            WorkbenchScreenPanel::Timeline => self.timeline_selection,
            WorkbenchScreenPanel::Main => self.main_selection,
            WorkbenchScreenPanel::Side => self.side_selection,
        }
    }

    fn scroll_chat_history_up(&mut self, lines: usize) {
        if self.raw_mode {
            let scroll = self.focused_scroll_mut();
            *scroll = scroll.saturating_sub(lines);
        } else {
            self.main_scroll = self.main_scroll.saturating_add(lines);
        }
    }

    fn scroll_chat_history_down(&mut self, lines: usize, max_scroll: Option<usize>) {
        if self.raw_mode {
            let scroll = self.focused_scroll_mut();
            *scroll = (*scroll).saturating_add(lines);
        } else {
            if self.main_scroll == usize::MAX {
                let Some(max_scroll) = max_scroll else {
                    return;
                };
                self.main_scroll = max_scroll;
            }
            self.main_scroll = self.main_scroll.saturating_sub(lines);
        }
    }

    fn scroll_chat_history_top(&mut self) {
        if self.raw_mode {
            *self.focused_scroll_mut() = 0;
        } else {
            self.main_scroll = usize::MAX;
        }
    }

    fn scroll_chat_history_bottom(&mut self) {
        if self.raw_mode {
            *self.focused_scroll_mut() = 0;
        } else {
            self.main_scroll = 0;
        }
    }

    #[cfg(test)]
    fn footer_summary(&self) -> String {
        let focused_scroll = self.scroll_for(self.focused);
        let focused_selection = self.selection_for(self.focused);
        let selected_action = self.action_selection.as_deref().unwrap_or("none");
        if self.focused == WorkbenchScreenPanel::Side {
            return format!(
                "focus={} approval_action=/approval approve id continuation_action=/cancel id input_action=/queue remove N selected={}:{} action={} render={} scroll={}:{} keys=tab arrows alt+a/d/c/x enter alt-enter",
                self.focused.as_str(),
                self.focused.as_str(),
                focused_selection.saturating_add(1),
                selected_action,
                if self.raw_mode { "raw" } else { "rich" },
                self.focused.as_str(),
                focused_scroll,
            );
        }
        if self.focused == WorkbenchScreenPanel::Timeline {
            return format!(
                "focus={} render={} scroll={}:{} selected={}:{} action={} keys=tab/shift-tab arrows pgup/pgdn/home enter alt-enter timeline_tabs=ctrl-t shift-enter ctrl-z/y alt-b/f ctrl-w/alt-d",
                self.focused.as_str(),
                if self.raw_mode { "raw" } else { "rich" },
                self.focused.as_str(),
                focused_scroll,
                self.focused.as_str(),
                focused_selection.saturating_add(1),
                selected_action,
            );
        }
        format!(
            "focus={} render={} scroll={}:{} action={} keys=tab/shift-tab arrows pgup/pgdn/home alt+a/d/c/x enter alt-enter shift-enter ctrl-z/y alt-b/f ctrl-w/alt-d selected={}:{}",
            self.focused.as_str(),
            if self.raw_mode { "raw" } else { "rich" },
            self.focused.as_str(),
            focused_scroll,
            selected_action,
            self.focused.as_str(),
            focused_selection.saturating_add(1),
        )
    }
}
