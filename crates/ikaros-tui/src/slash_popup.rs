// SPDX-License-Identifier: GPL-3.0-only

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const DEFAULT_PAGE_ROWS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandSpec {
    pub command: String,
    pub description: String,
    pub category: Option<String>,
    pub hidden_alias: bool,
    pub available_during_task: bool,
}

pub type SlashCommandItem = SlashCommandSpec;

impl SlashCommandSpec {
    pub fn new(command: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            command: normalize_command_name(command),
            description: description.into(),
            category: None,
            hidden_alias: false,
            available_during_task: true,
        }
    }

    pub fn hidden_alias(mut self, hidden_alias: bool) -> Self {
        self.hidden_alias = hidden_alias;
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn available_during_task(mut self, available: bool) -> Self {
        self.available_during_task = available;
        self
    }

    pub fn slash_name(&self) -> String {
        format!("/{}", self.command)
    }

    pub fn command_name(&self) -> &str {
        &self.command
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlashCommandPopupState {
    pub filter: String,
    pub selected: usize,
    pub scroll_top: usize,
}

impl SlashCommandPopupState {
    pub fn selected_index(&self) -> usize {
        self.selected
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandPopup {
    commands: Vec<SlashCommandSpec>,
    state: SlashCommandPopupState,
}

impl SlashCommandPopup {
    pub fn new(commands: Vec<SlashCommandSpec>) -> Self {
        Self {
            commands,
            state: SlashCommandPopupState::default(),
        }
    }

    pub fn state(&self) -> &SlashCommandPopupState {
        &self.state
    }

    pub fn update_filter(&mut self, input: &str) {
        let previous = self.state.filter.clone();
        self.state.filter = slash_filter_token(input);
        if self.state.filter != previous {
            self.select_first();
        }
        self.clamp_selection(usize::MAX);
    }

    pub fn filtered_items(&self) -> Vec<SlashCommandSpec> {
        filtered_commands(&self.commands, &self.state.filter)
    }

    pub fn selected(&self) -> Option<SlashCommandSpec> {
        self.filtered_items().get(self.state.selected).cloned()
    }

    pub fn selected_command(&self) -> Option<String> {
        self.selected().map(|command| command.slash_name())
    }

    pub fn complete_with_tab(&self) -> Option<String> {
        self.selected_command().map(|command| format!("{command} "))
    }

    pub fn select_first(&mut self) {
        self.state.selected = 0;
        self.state.scroll_top = 0;
    }

    pub fn select_exact_match(&mut self) -> bool {
        let filter = self.state.filter.trim().trim_start_matches('/');
        if filter.is_empty() {
            return false;
        }

        let Some(index) = self
            .filtered_items()
            .iter()
            .position(|command| command.command.eq_ignore_ascii_case(filter))
        else {
            return false;
        };
        self.state.selected = index;
        self.ensure_selected_visible(DEFAULT_PAGE_ROWS);
        true
    }

    pub fn move_up(&mut self) {
        let len = self.filtered_items().len();
        if len == 0 {
            self.state.selected = 0;
            return;
        }
        self.state.selected = if self.state.selected == 0 {
            len - 1
        } else {
            self.state.selected - 1
        };
        self.ensure_selected_visible(usize::MAX);
    }

    pub fn move_down(&mut self) {
        let len = self.filtered_items().len();
        if len == 0 {
            self.state.selected = 0;
            return;
        }
        self.state.selected = (self.state.selected + 1) % len;
        self.ensure_selected_visible(usize::MAX);
    }

    pub fn page_up(&mut self) {
        self.page_up_by(DEFAULT_PAGE_ROWS);
    }

    pub fn page_down(&mut self) {
        self.page_down_by(DEFAULT_PAGE_ROWS);
    }

    pub fn page_up_by(&mut self, rows: usize) {
        self.state.selected = self.state.selected.saturating_sub(rows.max(1));
        self.ensure_selected_visible(rows.max(1));
    }

    pub fn page_down_by(&mut self, rows: usize) {
        let len = self.filtered_items().len();
        if len == 0 {
            self.state.selected = 0;
            return;
        }
        self.state.selected = self.state.selected.saturating_add(rows.max(1)).min(len - 1);
        self.ensure_selected_visible(rows.max(1));
    }

    pub fn render_lines(&mut self, width: usize, max_rows: usize) -> Vec<String> {
        if max_rows == 0 {
            return Vec::new();
        }
        let width = width.max(1);
        self.clamp_selection(max_rows);
        let items = self.filtered_items();
        if items.is_empty() {
            return vec![fit_line("  no matching commands", width)];
        }
        let visible = items
            .iter()
            .enumerate()
            .skip(self.state.scroll_top)
            .take(max_rows)
            .collect::<Vec<_>>();
        let name_col = command_name_column_width(&items, width);
        let mut lines = Vec::new();
        for (idx, command) in visible {
            if lines.len() >= max_rows {
                break;
            }
            for line in render_command_row(command, idx == self.state.selected, name_col, width) {
                if lines.len() >= max_rows {
                    break;
                }
                lines.push(line);
            }
        }
        lines
    }

    fn clamp_selection(&mut self, visible_rows: usize) {
        let len = self.filtered_items().len();
        if len == 0 {
            self.state.selected = 0;
            self.state.scroll_top = 0;
            return;
        }
        self.state.selected = self.state.selected.min(len - 1);
        self.ensure_selected_visible(visible_rows);
    }

    fn ensure_selected_visible(&mut self, visible_rows: usize) {
        let visible_rows = visible_rows.max(1);
        if self.state.selected < self.state.scroll_top {
            self.state.scroll_top = self.state.selected;
        } else if self.state.selected >= self.state.scroll_top.saturating_add(visible_rows) {
            self.state.scroll_top = self
                .state
                .selected
                .saturating_add(1)
                .saturating_sub(visible_rows);
        }
    }
}

fn filtered_commands(commands: &[SlashCommandSpec], filter: &str) -> Vec<SlashCommandSpec> {
    let filter = filter.trim().trim_start_matches('/').to_ascii_lowercase();
    if filter.is_empty() {
        return commands
            .iter()
            .filter(|command| !command.hidden_alias)
            .cloned()
            .collect();
    }

    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    for command in commands {
        let name = command.command.to_ascii_lowercase();
        if name == filter {
            exact.push(command.clone());
        } else if name.starts_with(&filter) {
            prefix.push(command.clone());
        }
    }
    exact.into_iter().chain(prefix).collect()
}

fn slash_filter_token(input: &str) -> String {
    let first_line = input.lines().next().unwrap_or("").trim_start();
    let Some(rest) = first_line.strip_prefix('/') else {
        return String::new();
    };
    let token = rest.split_whitespace().next().unwrap_or("");
    token.to_owned()
}

fn normalize_command_name(name: impl Into<String>) -> String {
    name.into().trim().trim_start_matches('/').to_owned()
}

fn command_name_column_width(items: &[SlashCommandSpec], width: usize) -> usize {
    let max_name = items
        .iter()
        .map(|item| item.slash_name().width())
        .max()
        .unwrap_or(0);
    max_name.saturating_add(2).min((width * 6 / 10).max(8))
}

fn render_command_row(
    command: &SlashCommandSpec,
    selected: bool,
    name_col: usize,
    width: usize,
) -> Vec<String> {
    let marker = if selected { "› " } else { "  " };
    let name = command.slash_name();
    let left = format!("{marker}{}", pad_display(&name, name_col));
    let left_width = left.width();
    let desc_width = width.saturating_sub(left_width).max(1);
    let description = if command.available_during_task {
        command.description.clone()
    } else {
        format!("{} (idle only)", command.description)
    };
    let wrapped_desc = wrap_text(&description, desc_width);
    if wrapped_desc.is_empty() {
        return vec![fit_line(&left, width)];
    }
    wrapped_desc
        .into_iter()
        .enumerate()
        .map(|(idx, desc)| {
            if idx == 0 {
                fit_line(&format!("{left}{desc}"), width)
            } else {
                fit_line(&format!("{}{}", " ".repeat(left_width), desc), width)
            }
        })
        .collect()
}

fn pad_display(input: &str, width: usize) -> String {
    let input_width = input.width();
    if input_width >= width {
        return input.to_owned();
    }
    format!("{input}{}", " ".repeat(width - input_width))
}

fn fit_line(input: &str, width: usize) -> String {
    if input.width() <= width {
        return input.to_owned();
    }
    let mut out = String::new();
    let mut current = 0usize;
    let target = width.saturating_sub(1);
    for ch in input.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if current.saturating_add(ch_width) > target {
            break;
        }
        out.push(ch);
        current = current.saturating_add(ch_width);
    }
    out.push('…');
    out
}

fn wrap_text(input: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if input.is_empty() {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in input.split_whitespace() {
        let word_width = word.width();
        let separator = if current.is_empty() { 0 } else { 1 };
        if !current.is_empty() && current_width + separator + word_width > width {
            rows.push(std::mem::take(&mut current));
            current_width = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_width += 1;
        }
        if word_width <= width {
            current.push_str(word);
            current_width += word_width;
        } else {
            for ch in word.chars() {
                let ch_width = ch.width().unwrap_or(0);
                if !current.is_empty() && current_width + ch_width > width {
                    rows.push(std::mem::take(&mut current));
                    current_width = 0;
                }
                current.push(ch);
                current_width += ch_width;
            }
        }
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> Vec<SlashCommandSpec> {
        vec![
            SlashCommandSpec::new("model", "change model"),
            SlashCommandSpec::new("memory", "inspect memory"),
            SlashCommandSpec::new("clear", "clear terminal and start a new session"),
            SlashCommandSpec::new("new", "hidden clear alias").hidden_alias(true),
            SlashCommandSpec::new("sessions", "resume prior sessions"),
        ]
    }

    #[test]
    fn default_list_hides_aliases() {
        let popup = SlashCommandPopup::new(commands());
        let names = popup
            .filtered_items()
            .into_iter()
            .map(|item| item.command)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["model", "memory", "clear", "sessions"]);
    }

    #[test]
    fn prefix_filter_selects_model_for_mo() {
        let mut popup = SlashCommandPopup::new(commands());
        popup.update_filter("/mo");

        assert_eq!(popup.selected_command().as_deref(), Some("/model"));
        assert_eq!(popup.complete_with_tab().as_deref(), Some("/model "));
    }

    #[test]
    fn up_and_down_wrap_selection() {
        let mut popup = SlashCommandPopup::new(commands());
        popup.update_filter("/");

        popup.move_down();
        assert_eq!(popup.selected_command().as_deref(), Some("/memory"));
        popup.move_up();
        assert_eq!(popup.selected_command().as_deref(), Some("/model"));
        popup.move_up();
        assert_eq!(popup.selected_command().as_deref(), Some("/sessions"));
    }

    #[test]
    fn hidden_alias_appears_when_queried() {
        let mut popup = SlashCommandPopup::new(commands());
        popup.update_filter("/ne");

        assert_eq!(popup.selected_command().as_deref(), Some("/new"));

        popup.update_filter("/new");
        assert!(popup.select_exact_match());
        assert_eq!(popup.selected_command().as_deref(), Some("/new"));
    }

    #[test]
    fn render_uses_selected_marker_and_two_columns() {
        let mut popup = SlashCommandPopup::new(commands());
        popup.update_filter("/m");

        assert_eq!(
            popup.render_lines(60, 4),
            vec![
                "› /model   change model".to_owned(),
                "  /memory  inspect memory".to_owned(),
            ]
        );
    }

    #[test]
    fn render_wraps_description_on_narrow_width() {
        let mut popup = SlashCommandPopup::new(commands());
        popup.update_filter("/clear");
        let lines = popup.render_lines(28, 5);

        assert!(lines.len() > 1);
        assert!(lines[0].starts_with("› /clear"));
        assert!(lines.iter().all(|line| line.width() <= 28));
    }

    #[test]
    fn no_match_renders_empty_state() {
        let mut popup = SlashCommandPopup::new(commands());
        popup.update_filter("/zzzz");

        assert_eq!(popup.render_lines(80, 5), vec!["  no matching commands"]);
        assert_eq!(popup.selected_command(), None);

        let mut empty = SlashCommandPopup::new(Vec::new());
        empty.update_filter("/");
        assert_eq!(empty.render_lines(80, 5), vec!["  no matching commands"]);
        assert_eq!(empty.selected_command(), None);
    }

    #[test]
    fn page_navigation_updates_selection_and_scroll() {
        let mut popup = SlashCommandPopup::new(
            (0..12)
                .map(|index| SlashCommandSpec::new(format!("cmd{index}"), "command"))
                .collect(),
        );
        popup.update_filter("/");

        popup.page_down();
        assert_eq!(popup.state().selected_index(), 8);
        assert_eq!(popup.state().scroll_top, 1);

        popup.page_up();
        assert_eq!(popup.state().selected_index(), 0);
        assert_eq!(popup.state().scroll_top, 0);
    }
}
