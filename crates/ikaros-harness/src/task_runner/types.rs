// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

use crate::guardrails::GuardrailConfig;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionOptions {
    pub timeout_ms: Option<u64>,
    pub max_retries: u8,
    pub retry_delay_ms: u64,
    pub guardrails: GuardrailConfig,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            timeout_ms: Some(30_000),
            max_retries: 0,
            retry_delay_ms: 250,
            guardrails: GuardrailConfig::default(),
        }
    }
}
