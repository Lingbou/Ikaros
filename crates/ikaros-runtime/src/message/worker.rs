// SPDX-License-Identifier: GPL-3.0-only

use super::{drain::drain_gateway_messages, types::GatewayWorkerTickReport};
use ikaros_core::{IkarosError, IkarosPaths, Result};
use ikaros_gateway::LocalGatewayStore;
use std::path::Path;

pub async fn run_gateway_worker_tick(
    store: &LocalGatewayStore,
    limit: usize,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<GatewayWorkerTickReport> {
    if limit == 0 {
        return Err(IkarosError::Message(
            "message worker limit must be greater than zero".into(),
        ));
    }
    let messages = store.claim_pending(limit)?;
    let pending = messages.len();
    let reports = drain_gateway_messages(messages, store, paths, workspace, agent_override).await?;
    Ok(GatewayWorkerTickReport {
        kind: "gateway_worker_tick".into(),
        pending,
        drained: reports.len(),
        reports,
    })
}
