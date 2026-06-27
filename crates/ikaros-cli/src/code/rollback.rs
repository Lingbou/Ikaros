// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result};
use ikaros_core::IkarosPaths;
use ikaros_host::host_agent_context;
use ikaros_state::session::{AgentEventKind, SessionId, SessionStore, SqliteSessionStore, TurnId};
use std::path::Path;

pub(super) fn rollback_diff_for_coding_turn(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: &str,
    turn_id: &str,
) -> Result<String> {
    let host = host_agent_context(paths, workspace, agent_override)?;
    let store = SqliteSessionStore::new(&host.agent_instance.state_dir);
    let session_id = SessionId::from(session_id.to_owned());
    let turn_id = TurnId::from(turn_id.to_owned());
    let replay = store
        .replay_session(&session_id)?
        .with_context(|| format!("coding session not found: {}", session_id.as_str()))?;
    let diff = replay
        .agent_events
        .iter()
        .filter(|event| event.turn_id == turn_id)
        .filter(|event| matches!(event.kind, AgentEventKind::CodingTurn))
        .filter(|event| event.payload["kind"] == "diff_updated")
        .filter_map(|event| event.payload["payload"]["unified_diff"].as_str())
        .next_back()
        .with_context(|| {
            format!(
                "coding turn {} has no rollbackable diff_updated event",
                turn_id.as_str()
            )
        })?;
    reverse_unified_diff(diff)
}

pub(super) fn reverse_unified_diff(diff: &str) -> Result<String> {
    let mut reversed = Vec::new();
    let mut pending_source_header: Option<String> = None;
    let mut change_group = Vec::<String>::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            let mut parts = rest.split_whitespace();
            let Some(left) = parts.next() else {
                anyhow::bail!("invalid diff --git header");
            };
            let Some(right) = parts.next() else {
                anyhow::bail!("invalid diff --git header");
            };
            reversed.push(format!("diff --git {right} {left}"));
            continue;
        }
        if let Some(path) = line.strip_prefix("rename from ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            reversed.push(format!("rename to {path}"));
            continue;
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            reversed.push(format!("rename from {path}"));
            continue;
        }
        if line.starts_with("--- ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            pending_source_header = Some(line.replacen("--- ", "+++ ", 1));
            continue;
        }
        if line.starts_with("+++ ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            reversed.push(line.replacen("+++ ", "--- ", 1));
            if let Some(source) = pending_source_header.take() {
                reversed.push(source);
            }
            continue;
        }
        if line.starts_with("@@ ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            reversed.push(reverse_hunk_header(line)?);
            continue;
        }
        if line.starts_with('+') || line.starts_with('-') {
            change_group.push(line.to_owned());
        } else {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            reversed.push(line.to_owned());
        }
    }
    flush_reversed_change_group(&mut reversed, &mut change_group);
    if pending_source_header.is_some() {
        anyhow::bail!("invalid unified diff: missing +++ header");
    }
    Ok(format!("{}\n", reversed.join("\n")))
}

fn flush_reversed_change_group(output: &mut Vec<String>, group: &mut Vec<String>) {
    if group.is_empty() {
        return;
    }
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for line in group.drain(..) {
        if let Some(rest) = line.strip_prefix('+') {
            added.push(rest.to_owned());
        } else if let Some(rest) = line.strip_prefix('-') {
            removed.push(rest.to_owned());
        }
    }
    output.extend(added.into_iter().map(|line| format!("-{line}")));
    output.extend(removed.into_iter().map(|line| format!("+{line}")));
}

fn reverse_hunk_header(line: &str) -> Result<String> {
    let Some(rest) = line.strip_prefix("@@ ") else {
        anyhow::bail!("invalid hunk header");
    };
    let Some((ranges, suffix)) = rest.split_once(" @@") else {
        anyhow::bail!("invalid hunk header");
    };
    let mut parts = ranges.split_whitespace();
    let Some(old_range) = parts.next() else {
        anyhow::bail!("invalid hunk header");
    };
    let Some(new_range) = parts.next() else {
        anyhow::bail!("invalid hunk header");
    };
    if parts.next().is_some() || !old_range.starts_with('-') || !new_range.starts_with('+') {
        anyhow::bail!("invalid hunk header ranges");
    }
    Ok(format!(
        "@@ -{} +{} @@{}",
        &new_range[1..],
        &old_range[1..],
        suffix
    ))
}
