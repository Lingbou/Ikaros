// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::{
    history::{
        chat_history_context_lines_with_summary, chat_history_records_from_session_replay,
        session_replay_hides_chat_history,
    },
    types::{ChatContext, ChatRunOptions},
};
use ikaros_core::Result;
use ikaros_session::{SessionId, SessionStore, SqliteSessionStore};

pub(super) fn assemble_history_context(
    context: &mut ChatContext,
    options: &ChatRunOptions,
) -> Result<()> {
    if options.history_context_limit == 0 {
        return Ok(());
    }
    if let (Some(session_id), Some(state_db)) = (
        options.session_id.as_deref(),
        options.session_state_db.as_ref(),
    ) {
        let session_store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = session_store.replay_session(&SessionId::from(session_id))? {
            if session_replay_hides_chat_history(&replay) {
                return Ok(());
            }
            let records = chat_history_records_from_session_replay(&replay);
            if !records.is_empty() {
                context.history = chat_history_context_lines_with_summary(
                    &records,
                    options.history_context_limit,
                    options.history_summary_limit,
                );
                return Ok(());
            }
        }
    }
    Ok(())
}
