// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::Result;
use ikaros_harness::CancellationToken;
use ikaros_session::{
    ContinuationId, SessionContinuation, SessionContinuationStatus, SessionId, SessionStore,
};

const DURABLE_CONTINUATION_CANCEL_POLL_MS: u64 = 25;

pub(super) async fn poll_durable_continuation_cancel(
    store: &dyn SessionStore,
    session_id: SessionId,
    continuation_id: ContinuationId,
    token: CancellationToken,
) -> Result<()> {
    loop {
        if token.is_cancelled() {
            return Ok(());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(
            DURABLE_CONTINUATION_CANCEL_POLL_MS,
        ))
        .await;
        let Some(continuation) = continuation_for_id(store, &session_id, &continuation_id)? else {
            return Ok(());
        };
        match continuation.status {
            SessionContinuationStatus::Cancelled => {
                token.cancel();
                return Ok(());
            }
            SessionContinuationStatus::Completed | SessionContinuationStatus::Failed => {
                return Ok(());
            }
            SessionContinuationStatus::Queued | SessionContinuationStatus::Running => {}
        }
    }
}

pub(super) fn ensure_continuation_cancelled(
    store: &dyn SessionStore,
    session_id: &SessionId,
    continuation_id: &ContinuationId,
    reason: &str,
) -> Result<Option<SessionContinuation>> {
    if let Some(existing) = continuation_for_id(store, session_id, continuation_id)?
        && existing.status == SessionContinuationStatus::Cancelled
    {
        return Ok(Some(existing));
    }
    store.cancel_continuation(continuation_id, reason)
}

fn continuation_for_id(
    store: &dyn SessionStore,
    session_id: &SessionId,
    continuation_id: &ContinuationId,
) -> Result<Option<SessionContinuation>> {
    Ok(store
        .continuations(session_id)?
        .into_iter()
        .find(|continuation| &continuation.continuation_id == continuation_id))
}
