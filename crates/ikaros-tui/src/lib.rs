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
use std::mem;

mod actions;
mod input;
mod input_model;
mod layout;
mod markdown;
mod notice;
mod panels;
mod progress;
mod render;
mod selection;
mod slash;
mod slash_popup;
mod streaming;
mod surface;
#[cfg(test)]
mod tests;
mod transcript;
mod workbench_input;

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
pub use input::apply_workbench_screen_args;
pub use input::{
    apply_workbench_screen_key_event, apply_workbench_screen_key_event_with_view,
    apply_workbench_screen_mouse_event, apply_workbench_screen_mouse_event_with_view,
};
pub use markdown::{
    TerminalMarkdownRenderer, color_assistant_bullet_for_terminal, is_markdown_table_line,
    markdown_heading, render_assistant_markdown_transcript, render_terminal_markdown,
    render_terminal_markdown_lines,
};
pub use notice::{WorkbenchNotice, WorkbenchNoticeKind};
pub use progress::{WorkbenchProgressSnapshot, progress_bar, progress_phase, progress_spinner};
#[cfg(test)]
pub(crate) use render::render_tui_workbench_snapshot;
pub use render::{
    render_fullscreen_terminal_frame, render_fullscreen_workbench_with_state, screen_json_line,
    screen_selected_actions_json_line, screen_selected_actions_line, screen_selected_cell_line,
};
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
pub use streaming::{MarkdownStreamCollector, StreamFinish, TerminalStreamRenderer};
pub use transcript::{
    AssistantMarkdownCell, SeparatorCell, StreamingTailCell, ToolActivityCell, TranscriptCell,
    UserCell,
};
pub use workbench_input::{
    WorkbenchInputAction, WorkbenchInputEvent, WorkbenchInputState, WorkbenchTerminalInputEvent,
    WorkbenchTerminalInputOutcome, apply_workbench_terminal_input_event,
    format_workbench_input_state, parse_workbench_input_event, parse_workbench_terminal_event,
    parse_workbench_terminal_key_event,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TerminalMode {
    #[default]
    MainScreen,
    FullscreenReserved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiRunConfig {
    pub terminal_mode: TerminalMode,
    pub initial_composer_text: String,
}

impl Default for TuiRunConfig {
    fn default() -> Self {
        Self {
            terminal_mode: TerminalMode::MainScreen,
            initial_composer_text: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiRunResult {
    pub terminal_mode: TerminalMode,
    pub submitted_input: Option<String>,
}

pub async fn run(config: TuiRunConfig) -> anyhow::Result<TuiRunResult> {
    let terminal_mode = config.terminal_mode;
    let _composer = ComposerState::with_input(terminal_mode, config.initial_composer_text);

    Ok(TuiRunResult {
        terminal_mode,
        submitted_input: None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerState {
    terminal_mode: TerminalMode,
    input: String,
}

impl Default for ComposerState {
    fn default() -> Self {
        Self {
            terminal_mode: TerminalMode::MainScreen,
            input: String::new(),
        }
    }
}

impl ComposerState {
    pub fn new(terminal_mode: TerminalMode) -> Self {
        Self {
            terminal_mode,
            input: String::new(),
        }
    }

    pub fn with_input(terminal_mode: TerminalMode, input: impl Into<String>) -> Self {
        Self {
            terminal_mode,
            input: input.into(),
        }
    }

    pub fn terminal_mode(&self) -> TerminalMode {
        self.terminal_mode
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    pub fn apply(&mut self, action: ComposerAction) -> ComposerOutcome {
        match action {
            ComposerAction::InsertChar(ch) => {
                self.input.push(ch);
                ComposerOutcome::Updated
            }
            ComposerAction::InsertText(text) => {
                self.input.push_str(&text);
                ComposerOutcome::Updated
            }
            ComposerAction::Backspace => {
                let _ = self.input.pop();
                ComposerOutcome::Updated
            }
            ComposerAction::Clear => {
                self.input.clear();
                ComposerOutcome::Updated
            }
            ComposerAction::Enter => self.submit(),
            ComposerAction::ShiftEnter | ComposerAction::AltEnter => {
                self.input.push('\n');
                ComposerOutcome::NewlineInserted
            }
        }
    }

    fn submit(&mut self) -> ComposerOutcome {
        if self.input.trim().is_empty() {
            return ComposerOutcome::Ignored;
        }

        ComposerOutcome::Submitted(mem::take(&mut self.input))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerAction {
    InsertChar(char),
    InsertText(String),
    Backspace,
    Clear,
    Enter,
    ShiftEnter,
    AltEnter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerOutcome {
    Updated,
    NewlineInserted,
    Submitted(String),
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolActivityStatus {
    Started,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolActivity {
    pub name: String,
    pub status: ToolActivityStatus,
    pub detail: Option<String>,
}

impl ToolActivity {
    pub fn new(name: impl Into<String>, status: ToolActivityStatus) -> Self {
        Self {
            name: name.into(),
            status,
            detail: None,
        }
    }

    pub fn completed(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: ToolActivityStatus::Completed,
            detail: Some(detail.into()),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

pub fn render_tool_activity(activity: &ToolActivity) -> Vec<String> {
    let mut lines = vec![tool_activity_title(activity)];
    if let Some(detail) = activity
        .detail
        .as_deref()
        .map(terminal_inline)
        .filter(|detail| !detail.is_empty())
    {
        lines.push(format!("  └ {detail}"));
    }
    lines
}

fn tool_activity_title(activity: &ToolActivity) -> String {
    let name = terminal_inline(&activity.name);
    match activity.status {
        ToolActivityStatus::Started => format!("• Running {name}"),
        ToolActivityStatus::Failed => format!("• Tool failed: {name}"),
        ToolActivityStatus::Cancelled => format!("• Cancelled {name}"),
        ToolActivityStatus::Completed => completed_tool_activity_title(&name),
    }
}

fn completed_tool_activity_title(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.contains("read")
        || lower.contains("search")
        || lower.contains("list")
        || lower.contains("grep")
        || lower.contains("rg")
        || lower.contains("find")
        || lower.contains("explore")
    {
        "• Explored".to_owned()
    } else if lower.contains("write")
        || lower.contains("edit")
        || lower.contains("patch")
        || lower.contains("apply")
        || lower.contains("create")
    {
        "• Edited".to_owned()
    } else if lower.contains("test")
        || lower.contains("check")
        || lower.contains("clippy")
        || lower.contains("build")
    {
        "• Checked".to_owned()
    } else {
        format!("• Ran {name}")
    }
}

pub fn terminal_inline(input: &str) -> String {
    let stripped = strip_terminal_control_sequences(input);
    ikaros_core::redact_secrets(&strip_bare_sgr_mouse_sequences(&stripped))
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect()
}

pub fn terminal_message(input: &str) -> String {
    let stripped = strip_terminal_control_sequences(input);
    ikaros_core::redact_secrets(&strip_bare_sgr_mouse_sequences(&stripped))
        .chars()
        .filter_map(|ch| match ch {
            '\n' => Some('\n'),
            '\r' => None,
            ch if ch.is_control() => Some('_'),
            ch => Some(ch),
        })
        .collect()
}

fn strip_terminal_control_sequences(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            output.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                skip_until_string_terminator(&mut chars);
            }
            Some('P' | '_' | '^') => {
                chars.next();
                skip_until_string_terminator(&mut chars);
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    output
}

fn strip_bare_sgr_mouse_sequences(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0usize;
    'outer: while index < chars.len() {
        if chars[index] == '['
            && chars.get(index + 1) == Some(&'[')
            && chars.get(index + 2) == Some(&'<')
        {
            let mut cursor = index + 3;
            let mut has_digit = false;
            let mut semicolons = 0usize;
            while cursor < chars.len() {
                match chars[cursor] {
                    '0'..='9' => {
                        has_digit = true;
                        cursor += 1;
                    }
                    ';' => {
                        semicolons += 1;
                        cursor += 1;
                    }
                    'M' | 'm' if has_digit && semicolons >= 2 => {
                        index = cursor + 1;
                        continue 'outer;
                    }
                    _ => break,
                }
            }
        }
        if chars[index] == '[' && chars.get(index + 1) == Some(&'<') {
            let mut cursor = index + 2;
            let mut has_digit = false;
            let mut semicolons = 0usize;
            while cursor < chars.len() {
                match chars[cursor] {
                    '0'..='9' => {
                        has_digit = true;
                        cursor += 1;
                    }
                    ';' => {
                        semicolons += 1;
                        cursor += 1;
                    }
                    'M' | 'm' if has_digit && semicolons >= 2 => {
                        index = cursor + 1;
                        continue 'outer;
                    }
                    _ => break,
                }
            }
            if cursor == chars.len() && (has_digit || semicolons > 0) {
                break;
            }
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}

fn skip_until_string_terminator(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(ch) = chars.next() {
        if ch == '\u{7}' {
            break;
        }
        if ch == '\u{1b}' && chars.peek() == Some(&'\\') {
            chars.next();
            break;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchCellKind {
    Session,
    Model,
    Tool,
    Context,
    Memory,
    Coding,
    Audit,
    Continuation,
    Approval,
    Error,
}

impl WorkbenchCellKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Model => "model",
            Self::Tool => "tool",
            Self::Context => "context",
            Self::Memory => "memory",
            Self::Coding => "coding",
            Self::Audit => "audit",
            Self::Continuation => "continuation",
            Self::Approval => "approval",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchCell {
    pub kind: WorkbenchCellKind,
    pub title: String,
    pub detail: String,
}

impl WorkbenchCell {
    pub fn render(&self) -> String {
        format!(
            "cell kind={} title={} detail={}",
            self.kind.as_str(),
            terminal_inline(&self.title),
            terminal_inline(&self.detail)
        )
    }
}

pub fn render_workbench_snapshot(cells: &[WorkbenchCell], width: usize) -> String {
    let width = width.max(16);
    let mut output = format!("snapshot width={width}\n");
    for cell in cells {
        let prefix = format!("[{}]", cell.kind.as_str());
        let title = terminal_inline(&cell.title);
        if prefix.chars().count() + 1 + title.chars().count() <= width {
            output.push_str(&format!("{prefix} {title}\n"));
        } else {
            output.push_str(&prefix);
            output.push('\n');
            for line in wrap_snapshot_detail(&title, width.saturating_sub(2)) {
                output.push_str("  ");
                output.push_str(&line);
                output.push('\n');
            }
        }
        for line in wrap_snapshot_detail(&terminal_inline(&cell.detail), width.saturating_sub(2)) {
            output.push_str("  ");
            output.push_str(&line);
            output.push('\n');
        }
    }
    output
}

fn wrap_snapshot_detail(detail: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in detail.split_whitespace() {
        if word.chars().count() > width {
            if !current.is_empty() {
                lines.push(current);
                current = String::new();
            }
            let mut chunk = String::new();
            for ch in word.chars() {
                if chunk.chars().count() == width {
                    lines.push(chunk);
                    chunk = String::new();
                }
                chunk.push(ch);
            }
            if !chunk.is_empty() {
                current = chunk;
            }
            continue;
        }
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_owned();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push("none".into());
    }
    lines
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchScreen {
    pub title: String,
    pub status: Vec<WorkbenchCell>,
    pub timeline: Vec<WorkbenchCell>,
    pub main: Vec<WorkbenchCell>,
    pub side: Vec<WorkbenchCell>,
    pub footer: String,
    pub input_hint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchScreenPanel {
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
pub enum WorkbenchScreenAction {
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
pub enum WorkbenchScreenApprovalAction {
    Approve,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchScreenContinuationAction {
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchScreenInputAction {
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchScreenOpenAction {
    OpenSelected,
    ConfirmSelected,
}

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
