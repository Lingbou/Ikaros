// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{PolicyDecision, Result, now_rfc3339, redact_json, redact_secrets};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEvent {
    pub id: String,
    pub at: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub decision: Option<PolicyDecision>,
    pub message: String,
    pub data: serde_json::Value,
}

impl AuditEvent {
    pub fn new(
        kind: impl Into<String>,
        decision: Option<PolicyDecision>,
        message: impl Into<String>,
        data: serde_json::Value,
    ) -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            at: now_rfc3339()?,
            kind: kind.into(),
            correlation_id: None,
            decision,
            message: redact_secrets(&message.into()),
            data: redact_json(data),
        })
    }

    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        let correlation_id = redact_secrets(&correlation_id.into());
        if !correlation_id.trim().is_empty() {
            self.correlation_id = Some(correlation_id);
        }
        self
    }
}
