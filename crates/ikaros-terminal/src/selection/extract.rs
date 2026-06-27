// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn extract_assignment_commands(input: &str, key: &str) -> Vec<String> {
    let parts = input.split_whitespace().collect::<Vec<_>>();
    let mut commands = Vec::new();
    let mut index = 0;
    while index < parts.len() {
        let Some(command) = parts[index].strip_prefix(key) else {
            index += 1;
            continue;
        };
        if command.starts_with('/') {
            let mut command_parts = vec![command];
            let mut next = index + 1;
            while let Some(argument) = parts.get(next) {
                if argument.contains('=') {
                    break;
                }
                command_parts.push(argument);
                next += 1;
            }
            commands.push(terminal_inline(&command_parts.join(" ")));
            index = next;
            continue;
        }
        commands.push(terminal_inline(command));
        index += 1;
    }
    commands
}

pub(crate) fn extract_assignment_display(input: &str, key: &str, default: &str) -> String {
    extract_assignment_commands(input, key)
        .into_iter()
        .next()
        .or_else(|| extract_token_after(input, key))
        .unwrap_or_else(|| default.into())
}

pub(crate) fn extract_assignment_span(
    input: &str,
    key: &str,
    terminators: &[&str],
) -> Option<String> {
    let start = input.find(key)? + key.len();
    let tail = &input[start..];
    let end = terminators
        .iter()
        .filter_map(|terminator| tail.find(terminator))
        .min()
        .unwrap_or(tail.len());
    Some(terminal_inline(tail[..end].trim()))
}

pub(crate) fn extract_token_after(input: &str, key: &str) -> Option<String> {
    input
        .split_whitespace()
        .find_map(|part| part.strip_prefix(key))
        .map(terminal_inline)
}

pub(crate) fn default_cell_command(kind: WorkbenchCellKind) -> &'static str {
    match kind {
        WorkbenchCellKind::Session => "/timeline",
        WorkbenchCellKind::Model => "/model",
        WorkbenchCellKind::Tool => "/tools",
        WorkbenchCellKind::Context => "/context",
        WorkbenchCellKind::Memory => "/memory",
        WorkbenchCellKind::Coding => "/code plan",
        WorkbenchCellKind::Audit => "/debug",
        WorkbenchCellKind::Continuation => "/debug continuations",
        WorkbenchCellKind::Approval => "/approval",
        WorkbenchCellKind::Error => "/trace --failed",
    }
}
