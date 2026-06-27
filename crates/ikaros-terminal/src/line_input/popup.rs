// SPDX-License-Identifier: GPL-3.0-only

use super::{
    INLINE_COMPOSER_HINT_LIMIT,
    inline_composer::{InlineComposerLine, InlineComposerLineStyle},
};
use crate::text::{pad_terminal_text, terminal_display_width};
use crate::{
    SlashCommandPaletteItem, SlashCommandPopup, SlashCommandSpec, slash_command_palette_items,
    terminal_inline,
};
pub(super) fn inline_completion_popup_lines(
    query: &str,
    completions: Vec<String>,
    selected_completion: Option<&str>,
    width: usize,
) -> Vec<InlineComposerLine> {
    let popup_width = inline_popup_width(width);
    let specs = slash_completion_popup_specs(&completions);
    let mut popup = SlashCommandPopup::new(specs);
    popup.update_filter(query);
    if let Some(selected) = selected_completion {
        let selected = selected.trim_start_matches('/');
        if let Some(index) = popup
            .filtered_items()
            .iter()
            .position(|item| item.command_name() == selected)
        {
            while popup.state().selected_index() < index {
                popup.move_down();
            }
            while popup.state().selected_index() > index {
                popup.move_up();
            }
        } else {
            popup.select_exact_match();
        }
    } else {
        popup.select_exact_match();
    }
    let mut lines = vec![InlineComposerLine {
        text: codex_popup_border('+', '+', "Slash Commands", popup_width),
        style: InlineComposerLineStyle::Hint,
    }];
    for rendered in popup.render_lines(popup_width.saturating_sub(4), INLINE_COMPOSER_HINT_LIMIT) {
        let selected = rendered.trim_start().starts_with('>');
        lines.push(InlineComposerLine {
            text: codex_popup_item_rendered_line(&rendered, popup_width),
            style: if selected {
                InlineComposerLineStyle::SelectedHint
            } else {
                InlineComposerLineStyle::Hint
            },
        });
    }
    lines.push(InlineComposerLine {
        text: codex_popup_border('+', '+', "", popup_width),
        style: InlineComposerLineStyle::Hint,
    });
    lines
}

pub(super) fn slash_completion_popup_specs(completions: &[String]) -> Vec<SlashCommandSpec> {
    if completions.is_empty() {
        return slash_command_palette_items(None, usize::MAX)
            .into_iter()
            .map(slash_palette_item_spec)
            .collect();
    }
    completions
        .iter()
        .map(|completion| slash_completion_spec(completion))
        .collect()
}

pub(super) fn slash_completion_spec(command: &str) -> SlashCommandSpec {
    let command = terminal_inline(command);
    let normalized = command.trim_start_matches('/');
    let item = slash_command_palette_items(Some(&command), 1)
        .into_iter()
        .find(|item| item.name.trim_start_matches('/') == normalized);
    if let Some(item) = item {
        slash_palette_item_spec(item)
    } else {
        SlashCommandSpec::new(command, "")
    }
}

pub(super) fn slash_palette_item_spec(item: SlashCommandPaletteItem) -> SlashCommandSpec {
    let hidden_alias = item.tags.split(',').any(|tag| tag.trim() == "alias");
    SlashCommandSpec::new(item.name, terminal_inline(item.summary))
        .category(item.effect)
        .hidden_alias(hidden_alias)
        .available_during_task(item.effect != "session-mutation")
}

pub(super) fn inline_popup_width(width: usize) -> usize {
    let target = width.saturating_sub(2).max(1);
    target.max(20.min(width.max(1)))
}

pub(super) fn codex_popup_border(left: char, right: char, title: &str, width: usize) -> String {
    let width = width.max(4);
    let label = if title.trim().is_empty() {
        String::new()
    } else {
        format!("-{}-", title.trim())
    };
    let label_width = terminal_display_width(&label);
    let rule_width = width.saturating_sub(2 + label_width);
    format!("{left}{label}{}{right}", "-".repeat(rule_width))
}

pub(super) fn codex_popup_item_rendered_line(line: &str, width: usize) -> String {
    let inner_width = width.saturating_sub(4).max(1);
    format!("| {} |", pad_terminal_text(line, inner_width))
}
