// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_state::session::{
    SessionContinuationStatus, SessionId, SessionStore, SqliteSessionStore,
};

use super::super::{InteractiveChatRuntime, terminal_inline};
use super::continuation_status_count;
use super::output::continuations_json_line;

pub(in crate::chat::interactive) fn print_workbench_continuation_status(
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let continuations = store.continuations(&session_id)?;
    let queued = continuation_status_count(&continuations, SessionContinuationStatus::Queued);
    let running = continuation_status_count(&continuations, SessionContinuationStatus::Running);
    let completed = continuation_status_count(&continuations, SessionContinuationStatus::Completed);
    let failed = continuation_status_count(&continuations, SessionContinuationStatus::Failed);
    let cancelled = continuation_status_count(&continuations, SessionContinuationStatus::Cancelled);
    println!(
        "debug_continuations: session={} total={} queued={} running={} completed={} failed={} cancelled={}",
        terminal_inline(session_id.as_str()),
        continuations.len(),
        queued,
        running,
        completed,
        failed,
        cancelled,
    );
    println!("{}", continuations_json_line(&continuations));
    Ok(())
}
