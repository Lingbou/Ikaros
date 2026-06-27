// SPDX-License-Identifier: GPL-3.0-only

use std::collections::BTreeMap;

use crate::terminal_inline;

use super::matching::{
    slash_command_matches, slash_command_palette_matches, slash_command_query_matches,
};
use super::metadata::{command_metadata_list, slash_registry_permissions, slash_registry_surfaces};
use super::registry::{
    SLASH_COMMAND_REGISTRY_SCHEMA, SLASH_COMMAND_REGISTRY_SOURCE, SLASH_COMMAND_REGISTRY_VERSION,
    slash_commands,
};

pub fn slash_command_registry_summary() -> String {
    format!(
        "commands={} surfaces={} permissions={} command=/commands json=/commands --json markdown=/commands --markdown palette=/commands --palette help=/help",
        slash_commands().len(),
        command_metadata_list(slash_registry_surfaces().iter().copied()),
        command_metadata_list(slash_registry_permissions().iter().copied()),
    )
}

pub fn format_slash_command_help() -> String {
    let usages = slash_commands()
        .iter()
        .map(|command| command.usage)
        .collect::<Vec<_>>()
        .join(", ");
    format!("commands: {usages}")
}

pub fn print_slash_commands(args: &[&str]) {
    let options = SlashCommandQueryOptions::parse(args);
    let query = options.query.as_deref();
    if options.palette {
        println!("{}", slash_commands_palette_json_line(query));
        return;
    }
    if options.markdown {
        println!("{}", slash_command_registry_markdown(query));
        return;
    }
    if options.json_only {
        println!("{}", slash_commands_json_line(query));
        return;
    }
    if let Some(query) = query {
        println!("commands_query: {}", terminal_inline(query));
    } else {
        println!("commands_query: all");
    }
    println!("{}", slash_command_registry_line());
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| query.is_none_or(|query| slash_command_matches(*command, query)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| command.name);
    println!("commands_found: {}", matches.len());
    for command in matches {
        println!(
            "- {} usage={} args={} effect={} outputs={} permissions={} surfaces={} summary={}",
            terminal_inline(command.name),
            terminal_inline(command.usage),
            command.argument_model().as_str(),
            command.effect().as_str(),
            command_metadata_list(command.outputs().iter().copied()),
            command_metadata_list(
                command
                    .permissions
                    .iter()
                    .map(|permission| permission.as_str())
            ),
            command_metadata_list(command.surfaces.iter().map(|surface| surface.as_str())),
            terminal_inline(command.summary)
        );
    }
    println!("{}", slash_commands_json_line(query));
}

pub fn slash_commands_human_lines(args: &[&str]) -> Vec<String> {
    let options = SlashCommandQueryOptions::parse(args);
    let query = options.query.as_deref();
    let mut lines = vec!["* Slash commands".to_owned()];
    if let Some(query) = query {
        lines.push(format!("  query: {}", terminal_inline(query)));
    }
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| query.is_none_or(|query| slash_command_matches(*command, query)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| command.name);
    for command in matches.into_iter().take(12) {
        lines.push(format!(
            "  {:<12} {}",
            terminal_inline(command.name),
            terminal_inline(command.summary)
        ));
    }
    lines.push("  open picker: F5".to_owned());
    lines
}

pub fn print_slash_commands_for_human(args: &[&str]) {
    for line in slash_commands_human_lines(args) {
        println!("{line}");
    }
}

fn slash_command_registry_line() -> String {
    format!(
        "commands_registry: schema={} version={} source={} commands={} surfaces={} permissions={}",
        SLASH_COMMAND_REGISTRY_SCHEMA,
        SLASH_COMMAND_REGISTRY_VERSION,
        SLASH_COMMAND_REGISTRY_SOURCE,
        slash_commands().len(),
        slash_registry_surfaces().join(","),
        slash_registry_permissions().join(",")
    )
}

pub(super) fn slash_commands_json_line(query: Option<&str>) -> String {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let matches = slash_command_query_matches(query);
    let commands = matches
        .iter()
        .map(|command| {
            serde_json::json!({
                "name": command.name,
                "usage": command.usage,
                "summary": command.summary,
                "tags": command.tags,
                "argument_model": command.argument_model().as_str(),
                "effect": command.effect().as_str(),
                "outputs": command.outputs(),
                "permissions": command.permissions.iter().map(|permission| permission.as_str()).collect::<Vec<_>>(),
                "surfaces": command.surfaces.iter().map(|surface| surface.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    format!(
        "commands_json: {}",
        serde_json::json!({
            "schema": SLASH_COMMAND_REGISTRY_SCHEMA,
            "registry_version": SLASH_COMMAND_REGISTRY_VERSION,
            "source": SLASH_COMMAND_REGISTRY_SOURCE,
            "query": query.unwrap_or("all"),
            "command_count": commands.len(),
            "total_commands": slash_commands().len(),
            "surfaces": slash_registry_surfaces(),
            "permissions": slash_registry_permissions(),
            "commands": commands,
        })
    )
}

fn slash_commands_palette_json_line(query: Option<&str>) -> String {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let matches = slash_command_palette_matches(query);

    let mut by_effect: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
    let mut by_permission: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
    let mut by_surface: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
    for command in &matches {
        by_effect
            .entry(command.effect().as_str())
            .or_default()
            .push(command.name);
        for permission in command.permissions {
            by_permission
                .entry(permission.as_str())
                .or_default()
                .push(command.name);
        }
        for surface in command.surfaces {
            by_surface
                .entry(surface.as_str())
                .or_default()
                .push(command.name);
        }
    }

    let items = matches
        .iter()
        .map(|command| {
            serde_json::json!({
                "name": command.name,
                "usage": command.usage,
                "summary": command.summary,
                "argument_model": command.argument_model().as_str(),
                "effect": command.effect().as_str(),
                "permissions": command.permissions.iter().map(|permission| permission.as_str()).collect::<Vec<_>>(),
                "surfaces": command.surfaces.iter().map(|surface| surface.as_str()).collect::<Vec<_>>(),
                "tags": command.tags,
                "action": command.name,
                "detail_action": format!("/commands {}", command.name),
            })
        })
        .collect::<Vec<_>>();

    format!(
        "commands_palette_json: {}",
        serde_json::json!({
            "schema": "ikaros-workbench-command-palette-v1",
            "version": 1,
            "query": query.unwrap_or("all"),
            "command_count": items.len(),
            "total_commands": slash_commands().len(),
            "groups": {
                "effect": by_effect,
                "permission": by_permission,
                "surface": by_surface,
            },
            "items": items,
        })
    )
}

fn slash_command_registry_markdown(query: Option<&str>) -> String {
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| query.is_none_or(|query| slash_command_matches(*command, query)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| command.name);
    let mut output = String::new();
    output.push_str("commands_markdown:\n");
    output.push_str("# Ikaros Slash Commands\n\n");
    output.push_str(&format!(
        "- schema: `{}`\n- version: `{}`\n- source: `{}`\n- query: `{}`\n- commands: `{}`\n\n",
        SLASH_COMMAND_REGISTRY_SCHEMA,
        SLASH_COMMAND_REGISTRY_VERSION,
        SLASH_COMMAND_REGISTRY_SOURCE,
        query.unwrap_or("all"),
        matches.len()
    ));
    output.push_str("| Command | Usage | Effect | Permissions | Surfaces | Summary |\n");
    output.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for command in matches {
        output.push_str(&format!(
            "| `{}` | `{}` | `{}` | `{}` | `{}` | {} |\n",
            terminal_inline(command.name),
            terminal_inline(command.usage),
            command.effect().as_str(),
            command_metadata_list(
                command
                    .permissions
                    .iter()
                    .map(|permission| permission.as_str())
            ),
            command_metadata_list(command.surfaces.iter().map(|surface| surface.as_str())),
            terminal_inline(command.summary)
        ));
    }
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SlashCommandQueryOptions {
    query: Option<String>,
    json_only: bool,
    markdown: bool,
    palette: bool,
}

impl SlashCommandQueryOptions {
    fn parse(args: &[&str]) -> Self {
        let mut query = Vec::new();
        let mut json_only = false;
        let mut markdown = false;
        let mut palette = false;
        for arg in args {
            match *arg {
                "--json" | "json" => json_only = true,
                "--markdown" | "--md" | "markdown" => markdown = true,
                "--palette" | "palette" => palette = true,
                value => query.push(value),
            }
        }
        Self {
            query: (!query.is_empty()).then(|| query.join(" ")),
            json_only,
            markdown,
            palette,
        }
    }
}
