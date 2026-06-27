// SPDX-License-Identifier: GPL-3.0-only

use crate::terminal_inline;

use super::model::{SlashCommandPermission, SlashCommandSurface};
use super::registry::slash_commands;

pub(super) fn command_metadata_list<'a>(values: impl Iterator<Item = &'a str>) -> String {
    values.map(terminal_inline).collect::<Vec<_>>().join(",")
}

pub(super) fn slash_registry_surfaces() -> Vec<&'static str> {
    let mut surfaces = slash_commands()
        .iter()
        .flat_map(|command| command.surfaces.iter().copied())
        .map(SlashCommandSurface::as_str)
        .collect::<Vec<_>>();
    surfaces.sort();
    surfaces.dedup();
    surfaces
}

pub(super) fn slash_registry_permissions() -> Vec<&'static str> {
    let mut permissions = slash_commands()
        .iter()
        .flat_map(|command| command.permissions.iter().copied())
        .map(SlashCommandPermission::as_str)
        .collect::<Vec<_>>();
    permissions.sort();
    permissions.dedup();
    permissions
}
