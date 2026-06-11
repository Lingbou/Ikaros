// SPDX-License-Identifier: GPL-3.0-only

use super::types::{AgentHandoffReport, AgentPoolItemReport};
use ikaros_core::{Result, redact_secrets};

pub(super) fn pool_item_from_result(
    index: usize,
    task_text: String,
    requested_profile: Option<String>,
    result: Result<AgentHandoffReport>,
) -> AgentPoolItemReport {
    match result {
        Ok(report) => AgentPoolItemReport {
            index,
            task: redact_secrets(&task_text),
            profile: Some(report.agent.clone()),
            ok: true,
            state: Some(report.report.state.clone()),
            report: Some(report),
            error: None,
        },
        Err(error) => AgentPoolItemReport {
            index,
            task: redact_secrets(&task_text),
            profile: requested_profile,
            ok: false,
            state: None,
            report: None,
            error: Some(redact_secrets(&error.to_string())),
        },
    }
}
