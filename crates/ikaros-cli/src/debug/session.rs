// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) fn debug_session(
    args: DebugSessionArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let page = args.page.max(1);
    let page_size = args.page_size.max(1);
    let (state_db, replay) = replay_session_page(
        paths,
        workspace,
        agent_override,
        &args.session_id,
        page,
        page_size,
    )?;
    let export = args
        .export
        .as_ref()
        .map(|path| {
            let store = SqliteSessionStore::from_file(&state_db);
            let session_id = SessionId::from(args.session_id.as_str());
            let full_replay = store
                .replay_session(&session_id)?
                .ok_or_else(|| anyhow!("session not found in state.db files: {session_id}"))?;
            write_session_export(path, &state_db, &full_replay)
        })
        .transpose()?;
    let entries = serde_json::to_value(&replay.entries)?;
    let agent_events = serde_json::to_value(&replay.agent_events)?;
    let approvals = serde_json::to_value(&replay.approvals)?;
    let turn_correlations = replay_page_turn_correlations(&args.session_id, &replay);
    let output = json!({
        "format": "ikaros-session-debug-v1",
        "session_id": args.session_id,
        "state_db": state_db.display().to_string(),
        "session": replay.session,
        "turn_correlations": turn_correlations,
        "counts": {
            "entries": replay.total_entries,
            "agent_events": replay.total_agent_events,
            "approvals": replay.total_approvals,
        },
        "pagination": {
            "page": page,
            "page_size": page_size,
            "entries": pagination_summary(replay.total_entries, page, page_size),
            "agent_events": pagination_summary(replay.total_agent_events, page, page_size),
            "approvals": pagination_summary(replay.total_approvals, page, page_size),
        },
        "entries": entries,
        "agent_events": agent_events,
        "approvals": approvals,
        "export": export,
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(in crate::debug) fn write_session_export(
    path: &Path,
    state_db: &Path,
    replay: &SessionReplay,
) -> Result<Value> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let turn_correlations = replay_turn_correlations(replay.session.session_id.as_str(), replay);
    let artifact = json!({
        "format": "ikaros-session-export-v1",
        "redacted": true,
        "state_db": state_db.display().to_string(),
        "exported_at": time::OffsetDateTime::now_utc(),
        "session": replay.session,
        "turn_correlations": turn_correlations,
        "entries": replay.entries,
        "agent_events": replay.agent_events,
        "approvals": replay.approvals,
    });
    fs::write(path, serde_json::to_vec_pretty(&redact_json(artifact))?)?;
    Ok(json!({
        "created": path.is_file(),
        "path": path.display().to_string(),
    }))
}

pub(in crate::debug) fn pagination_summary(total: usize, page: usize, page_size: usize) -> Value {
    let start = page_start(page, page_size);
    let end = start.saturating_add(page_size).min(total);
    json!({
        "total": total,
        "start": start,
        "end": end,
        "has_previous": page > 1 && total > 0,
        "has_next": end < total,
    })
}

pub(in crate::debug) fn page_start(page: usize, page_size: usize) -> usize {
    page.saturating_sub(1).saturating_mul(page_size)
}
pub(in crate::debug) fn replay_turn_correlations(
    session_id: &str,
    replay: &SessionReplay,
) -> BTreeMap<String, String> {
    let mut turn_ids = replay
        .agent_events
        .iter()
        .map(|event| event.turn_id.to_string())
        .collect::<BTreeSet<_>>();
    for entry in &replay.entries {
        if let Some(turn_id) = entry.turn_id.as_ref() {
            turn_ids.insert(turn_id.to_string());
        }
    }
    for approval in &replay.approvals {
        if let Some(turn_id) = approval.turn_id.as_ref() {
            turn_ids.insert(turn_id.to_string());
        }
    }
    turn_correlation_map(session_id, &turn_ids)
}

pub(in crate::debug) fn replay_page_turn_correlations(
    session_id: &str,
    replay: &SessionReplayPage,
) -> BTreeMap<String, String> {
    let mut turn_ids = replay
        .agent_events
        .iter()
        .map(|event| event.turn_id.to_string())
        .collect::<BTreeSet<_>>();
    for entry in &replay.entries {
        if let Some(turn_id) = entry.turn_id.as_ref() {
            turn_ids.insert(turn_id.to_string());
        }
    }
    for approval in &replay.approvals {
        if let Some(turn_id) = approval.turn_id.as_ref() {
            turn_ids.insert(turn_id.to_string());
        }
    }
    turn_correlation_map(session_id, &turn_ids)
}
pub(in crate::debug) fn replay_session(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: &str,
) -> Result<(PathBuf, SessionReplay)> {
    let session_id = SessionId::from(session_id);
    for state_db in state_db_candidates(paths, workspace, agent_override)? {
        let store = SqliteSessionStore::from_file(&state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            return Ok((state_db, replay));
        }
    }
    Err(anyhow!("session not found in state.db files: {session_id}"))
}

pub(in crate::debug) fn replay_session_page(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: &str,
    page: usize,
    page_size: usize,
) -> Result<(PathBuf, SessionReplayPage)> {
    let session_id = SessionId::from(session_id);
    for state_db in state_db_candidates(paths, workspace, agent_override)? {
        let store = SqliteSessionStore::from_file(&state_db);
        if let Some(replay) = store.replay_session_page(&session_id, page, page_size)? {
            return Ok((state_db, replay));
        }
    }
    Err(anyhow!("session not found in state.db files: {session_id}"))
}

pub(in crate::debug) fn state_db_candidates(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    push_state_db_candidate(&mut candidates, &mut seen, agent.state_dir.join("state.db"));
    if agent_override.is_none() {
        let agents_dir = paths.home.join("agents");
        if agents_dir.is_dir() {
            for entry in fs::read_dir(&agents_dir)? {
                let entry = entry?;
                let state_db = entry.path().join("state.db");
                push_state_db_candidate(&mut candidates, &mut seen, state_db);
            }
        }
    }
    Ok(candidates)
}

pub(in crate::debug) fn push_state_db_candidate(
    candidates: &mut Vec<PathBuf>,
    seen: &mut BTreeSet<PathBuf>,
    state_db: PathBuf,
) {
    if state_db.is_file() && seen.insert(state_db.clone()) {
        candidates.push(state_db);
    }
}

pub(in crate::debug) fn filter_turn_events<'a>(
    events: &'a [AgentEvent],
    session_id: &str,
    turn_id: Option<&str>,
) -> Result<Vec<&'a AgentEvent>> {
    let filtered = events
        .iter()
        .filter(|event| turn_id.is_none_or(|turn_id| event.turn_id.as_str() == turn_id))
        .collect::<Vec<_>>();
    if let Some(turn_id) = turn_id
        && filtered.is_empty()
    {
        return Err(anyhow!("turn not found in session {session_id}: {turn_id}"));
    }
    Ok(filtered)
}
