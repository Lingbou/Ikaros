// SPDX-License-Identifier: GPL-3.0-only

use super::matching::{
    slash_command_matches, slash_command_palette_matches, slash_command_visible_in_picker,
};
use super::metadata::command_metadata_list;
use super::model::{SlashCommandCompletion, SlashCommandPaletteItem, SlashCommandPaletteSummary};
use super::registry::slash_commands;

pub fn slash_command_completion_candidates(
    input: &str,
    limit: usize,
) -> Vec<SlashCommandCompletion> {
    let query = slash_completion_query(input);
    if query.is_empty() {
        return Vec::new();
    }
    let mut prefix_matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| {
            command.name.starts_with(query)
                && slash_command_visible_in_picker(*command, Some(query))
        })
        .collect::<Vec<_>>();
    prefix_matches.sort_by_key(|command| command.name);

    let mut fuzzy_matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| {
            !command.name.starts_with(query)
                && slash_command_visible_in_picker(*command, Some(query))
                && slash_command_matches(*command, query)
        })
        .collect::<Vec<_>>();
    fuzzy_matches.sort_by_key(|command| command.name);

    prefix_matches
        .into_iter()
        .chain(fuzzy_matches)
        .take(limit.max(1))
        .map(|command| SlashCommandCompletion {
            name: command.name,
            usage: command.usage,
            summary: command.summary,
            argument_model: command.argument_model().as_str(),
            effect: command.effect().as_str(),
        })
        .collect()
}

pub fn slash_completion_query(input: &str) -> &str {
    let input = input.trim_start();
    if !input.starts_with('/') {
        return "";
    }
    input.split_whitespace().next().unwrap_or(input)
}

pub fn slash_command_palette_summary(query: Option<&str>) -> SlashCommandPaletteSummary {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let matches = slash_command_palette_matches(query);
    let mut effects = matches
        .iter()
        .map(|command| command.effect().as_str())
        .collect::<Vec<_>>();
    effects.sort();
    effects.dedup();
    let mut permissions = matches
        .iter()
        .flat_map(|command| {
            command
                .permissions
                .iter()
                .map(|permission| permission.as_str())
        })
        .collect::<Vec<_>>();
    permissions.sort();
    permissions.dedup();
    let mut surfaces = matches
        .iter()
        .flat_map(|command| command.surfaces.iter().map(|surface| surface.as_str()))
        .collect::<Vec<_>>();
    surfaces.sort();
    surfaces.dedup();
    SlashCommandPaletteSummary {
        query: query.unwrap_or("all").to_owned(),
        command_count: matches.len(),
        total_commands: slash_commands()
            .iter()
            .copied()
            .filter(|command| slash_command_visible_in_picker(*command, None))
            .count(),
        effects: command_metadata_list(effects.into_iter()),
        permissions: command_metadata_list(permissions.into_iter()),
        surfaces: command_metadata_list(surfaces.into_iter()),
    }
}

pub fn slash_command_palette_items(
    query: Option<&str>,
    limit: usize,
) -> Vec<SlashCommandPaletteItem> {
    slash_command_palette_matches(query)
        .into_iter()
        .take(limit.max(1))
        .map(|command| SlashCommandPaletteItem {
            name: command.name,
            usage: command.usage,
            summary: command.summary,
            argument_model: command.argument_model().as_str(),
            effect: command.effect().as_str(),
            permissions: command_metadata_list(
                command
                    .permissions
                    .iter()
                    .map(|permission| permission.as_str()),
            ),
            surfaces: command_metadata_list(
                command.surfaces.iter().map(|surface| surface.as_str()),
            ),
            tags: command_metadata_list(command.tags.iter().copied()),
        })
        .collect()
}
