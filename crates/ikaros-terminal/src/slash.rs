// SPDX-License-Identifier: GPL-3.0-only

mod completion;
mod matching;
mod metadata;
mod model;
mod output;
mod registry;

#[cfg(test)]
mod tests;

pub use completion::{
    slash_command_completion_candidates, slash_command_palette_items,
    slash_command_palette_summary, slash_completion_query,
};
pub use matching::suggest_slash_command;
pub use model::{SlashCommandCompletion, SlashCommandPaletteItem, SlashCommandPaletteSummary};
pub use output::{
    format_slash_command_help, print_slash_commands, print_slash_commands_for_human,
    slash_command_registry_summary, slash_commands_human_lines,
};
