// SPDX-License-Identifier: GPL-3.0-only

use super::model::SlashCommandDescriptor;
use super::registry::slash_commands;

pub fn suggest_slash_command(input: &str) -> Option<&'static str> {
    let command = input.split_whitespace().next().unwrap_or(input).trim();
    if command.is_empty() {
        return None;
    }
    slash_commands()
        .iter()
        .filter_map(|candidate| {
            let score = edit_distance(command, candidate.name);
            (score <= 2).then_some((score, candidate.name))
        })
        .min_by_key(|(score, name)| (*score, *name))
        .map(|(_, name)| name)
}

pub(super) fn slash_command_matches(command: SlashCommandDescriptor, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    let mut terms = query.split_whitespace();
    if let (Some(first), Some(second)) = (terms.next(), terms.next()) {
        return slash_command_matches(command, first)
            && slash_command_matches(command, second)
            && terms.all(|term| slash_command_matches(command, term));
    }
    let name = command.name.to_ascii_lowercase();
    let usage = command.usage.to_ascii_lowercase();
    let summary = command.summary.to_ascii_lowercase();
    name.contains(&query)
        || usage.contains(&query)
        || summary.contains(&query)
        || command.tags.iter().any(|tag| tag.contains(&query))
        || command
            .permissions
            .iter()
            .any(|permission| permission.as_str().contains(&query))
        || command
            .surfaces
            .iter()
            .any(|surface| surface.as_str().contains(&query))
        || command.argument_model().as_str().contains(&query)
        || command.effect().as_str().contains(&query)
        || command
            .outputs()
            .iter()
            .any(|output| output.contains(&query))
        || fuzzy_subsequence(&name, &query)
}

pub(super) fn slash_command_query_matches(query: Option<&str>) -> Vec<SlashCommandDescriptor> {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| query.is_none_or(|query| slash_command_matches(*command, query)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| command.name);
    matches
}

pub(super) fn slash_command_palette_matches(query: Option<&str>) -> Vec<SlashCommandDescriptor> {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| {
            slash_command_visible_in_picker(*command, query)
                && query.is_none_or(|query| slash_command_matches(*command, query))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| {
        (
            command.effect().as_str(),
            command.argument_model().as_str(),
            command.name,
        )
    });
    matches
}

pub(super) fn slash_command_visible_in_picker(
    command: SlashCommandDescriptor,
    query: Option<&str>,
) -> bool {
    if !slash_command_is_alias(command) {
        return true;
    }
    let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) else {
        return false;
    };
    let query = query.trim_start_matches('/');
    if query.len() < 2 {
        return false;
    }
    command.name.trim_start_matches('/').starts_with(query)
}

fn slash_command_is_alias(command: SlashCommandDescriptor) -> bool {
    command.tags.contains(&"alias")
}

fn fuzzy_subsequence(value: &str, query: &str) -> bool {
    let mut query_chars = query.chars();
    let Some(mut next) = query_chars.next() else {
        return true;
    };
    for ch in value.chars() {
        if ch == next {
            let Some(candidate) = query_chars.next() else {
                return true;
            };
            next = candidate;
        }
    }
    false
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right_len = right.chars().count();
    let mut previous = (0..=right_len).collect::<Vec<_>>();
    let mut current = vec![0; right_len + 1];
    for (left_index, left_ch) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_ch) in right.chars().enumerate() {
            let insert = current[right_index] + 1;
            let delete = previous[right_index + 1] + 1;
            let replace = previous[right_index] + usize::from(left_ch != right_ch);
            current[right_index + 1] = insert.min(delete).min(replace);
        }
        previous.clone_from(&current);
    }
    previous[right_len]
}
