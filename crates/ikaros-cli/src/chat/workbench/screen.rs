// SPDX-License-Identifier: GPL-3.0-only

use super::{
    SlashCommandPaletteItem, WorkbenchCell, WorkbenchCellKind, render_terminal_markdown,
    slash_command_palette_items, slash_command_palette_summary, terminal_inline,
};
use crate::chat::output::{is_markdown_table_line, markdown_heading};
use crate::chat::progress::{progress_bar, progress_phase, progress_spinner};
use anyhow::{Result, anyhow};
use crossterm::{
    cursor::{Hide, Show},
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{CrosstermBackend, TestBackend},
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::io::{self, IsTerminal};

mod actions;
mod input;
mod input_model;
mod layout;
mod panels;
mod render;
mod selection;
mod surface;
#[cfg(test)]
mod tests;

use actions::action_selection_is_command_palette;
use input_model::selected_palette_command;

#[cfg(test)]
use actions::action_menu_queue_items_json;
#[cfg(test)]
use input::parse_workbench_screen_state;
#[cfg(test)]
use input_model::command_palette_overlay_json;
#[cfg(test)]
use panels::screen_queue_panel_json;
#[cfg(test)]
use render::render_tui_workbench_snapshot;

pub(in crate::chat) use input::{
    apply_workbench_screen_args, apply_workbench_screen_key_event_with_view,
    apply_workbench_screen_mouse_event_with_view,
};
#[cfg(test)]
pub(in crate::chat) use input::{
    apply_workbench_screen_key_event, apply_workbench_screen_mouse_event,
};
pub(in crate::chat) use render::{
    PersistentWorkbenchTerminal, draw_persistent_fullscreen_terminal_frame,
    fullscreen_terminal_exit_sequence, render_fullscreen_terminal_frame,
    render_fullscreen_workbench_with_state, render_persistent_fullscreen_terminal_frame,
    screen_json_line, screen_selected_actions_json_line, screen_selected_actions_line,
    screen_selected_cell_line,
};
pub(in crate::chat) use selection::{
    command_requires_explicit_action, screen_selected_primary_action,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat) struct WorkbenchScreen {
    pub(in crate::chat) title: String,
    pub(in crate::chat) status: Vec<WorkbenchCell>,
    pub(in crate::chat) timeline: Vec<WorkbenchCell>,
    pub(in crate::chat) main: Vec<WorkbenchCell>,
    pub(in crate::chat) side: Vec<WorkbenchCell>,
    pub(in crate::chat) footer: String,
    pub(in crate::chat) input_hint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum WorkbenchScreenPanel {
    Status,
    Timeline,
    Main,
    Side,
}

impl WorkbenchScreenPanel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Timeline => "timeline",
            Self::Main => "main",
            Self::Side => "side",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Status => Self::Timeline,
            Self::Timeline => Self::Main,
            Self::Main => Self::Side,
            Self::Side => Self::Status,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Status => Self::Side,
            Self::Timeline => Self::Status,
            Self::Main => Self::Timeline,
            Self::Side => Self::Main,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum WorkbenchScreenAction {
    FocusNext,
    FocusPrevious,
    ScrollDown,
    ScrollUp,
    PageDown,
    PageUp,
    ScrollTop,
    SelectNext,
    SelectPrevious,
    SelectFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum WorkbenchScreenApprovalAction {
    Approve,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum WorkbenchScreenContinuationAction {
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum WorkbenchScreenInputAction {
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum WorkbenchScreenOpenAction {
    OpenSelected,
    ConfirmSelected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat) struct WorkbenchScreenState {
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
    pub(in crate::chat) fn focused_panel(&self) -> WorkbenchScreenPanel {
        self.focused
    }

    pub(in crate::chat) fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub(in crate::chat) fn raw_mode(&self) -> bool {
        self.raw_mode
    }

    pub(in crate::chat) fn side_selection(&self) -> usize {
        self.side_selection
    }

    pub(in crate::chat) fn command_palette_open(&self) -> bool {
        self.command_palette_open
    }

    pub(in crate::chat) fn close_command_palette(&mut self) -> bool {
        self.close_command_palette_state()
    }

    pub(in crate::chat) fn selected_command_palette_command(&self) -> Option<String> {
        selected_palette_command(self)
    }

    pub(in crate::chat) fn take_approval_action(
        &mut self,
    ) -> Option<WorkbenchScreenApprovalAction> {
        self.approval_action.take()
    }

    pub(in crate::chat) fn take_continuation_action(
        &mut self,
    ) -> Option<WorkbenchScreenContinuationAction> {
        self.continuation_action.take()
    }

    pub(in crate::chat) fn take_input_action(&mut self) -> Option<WorkbenchScreenInputAction> {
        self.input_action.take()
    }

    pub(in crate::chat) fn take_open_action(&mut self) -> Option<WorkbenchScreenOpenAction> {
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

    pub(in crate::chat) fn apply(&mut self, action: WorkbenchScreenAction) {
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
