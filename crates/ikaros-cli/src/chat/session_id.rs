// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_agent::chat::{chat_history_records_from_session_replay, new_chat_session_id};
use ikaros_core::AgentInstance;
use ikaros_state::session::{SessionSource, SessionStore, SqliteSessionStore};

pub(in crate::chat) fn interactive_chat_session_id(
    agent: &AgentInstance,
    explicit_session_id: Option<&str>,
) -> Result<String> {
    if let Some(session_id) = explicit_session_id {
        return Ok(session_id.to_owned());
    }
    Ok(recent_interactive_chat_session_id(agent)?.unwrap_or_else(new_chat_session_id))
}

fn recent_interactive_chat_session_id(agent: &AgentInstance) -> Result<Option<String>> {
    let store = SqliteSessionStore::new(&agent.state_dir);
    if !store.path().is_file() {
        return Ok(None);
    }
    let mut latest: Option<ikaros_state::session::SessionRecord> = None;
    for session in store.session_records()? {
        if session.ended_at.is_some()
            || !matches!(session.source, SessionSource::Cli)
            || session.agent_id.as_deref() != Some(agent.agent_id.as_str())
            || session.workspace.as_deref() != Some(agent.workspace.as_path())
        {
            continue;
        }
        let Some(replay) = store.replay_session(&session.session_id)? else {
            continue;
        };
        if chat_history_records_from_session_replay(&replay).is_empty() {
            continue;
        }
        if latest.as_ref().is_none_or(|current| {
            session
                .started_at
                .cmp(&current.started_at)
                .then_with(|| session.session_id.as_str().cmp(current.session_id.as_str()))
                .is_gt()
        }) {
            latest = Some(session);
        }
    }
    Ok(latest.map(|session| session.session_id.to_string()))
}
