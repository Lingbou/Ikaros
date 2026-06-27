// SPDX-License-Identifier: GPL-3.0-only

use crate::terminal_inline;
use ikaros_core::redact_secrets;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchProgressSnapshot {
    pub kind: String,
    pub status: String,
    pub elapsed_ms: Option<u128>,
    pub detail: String,
    pub error_kind: Option<String>,
}

impl WorkbenchProgressSnapshot {
    pub fn new(
        kind: &str,
        status: &str,
        elapsed_ms: Option<u128>,
        detail: Option<&str>,
        error_kind: Option<&str>,
    ) -> Self {
        Self {
            kind: terminal_inline(kind),
            status: terminal_inline(status),
            elapsed_ms,
            detail: detail.map(progress_detail).unwrap_or_else(|| "none".into()),
            error_kind: error_kind.map(terminal_inline),
        }
    }

    pub fn phase(&self) -> &'static str {
        progress_phase(&self.status)
    }

    pub fn spinner(&self) -> &'static str {
        progress_spinner(self.elapsed_ms, &self.status)
    }

    pub fn progress_bar(&self) -> &'static str {
        progress_bar(&self.status)
    }
}

fn progress_detail(input: &str) -> String {
    const MAX_CHARS: usize = 160;
    let redacted = redact_secrets(input).replace(['\n', '\r'], " ");
    let mut output = String::new();
    for (index, ch) in redacted.chars().enumerate() {
        if index >= MAX_CHARS {
            output.push_str("...");
            return output;
        }
        output.push(ch);
    }
    output
}

pub fn progress_phase(status: &str) -> &'static str {
    match status {
        "running" => "active",
        "queued" => "queued",
        "approval_pending" => "waiting_approval",
        "failed" => "recoverable",
        "completed" => "done",
        "cancelled" => "cancelled",
        _ => "idle",
    }
}

pub fn progress_spinner(elapsed_ms: Option<u128>, status: &str) -> &'static str {
    if !matches!(status, "running" | "queued" | "approval_pending") {
        return "-";
    }
    match elapsed_ms.unwrap_or_default() / 250 % 4 {
        0 => "|",
        1 => "/",
        2 => "-",
        _ => "\\",
    }
}

pub fn progress_bar(status: &str) -> &'static str {
    match status {
        "running" => "[###-------]",
        "queued" => "[#---------]",
        "approval_pending" => "[#####-----]",
        "completed" => "[##########]",
        "failed" => "[!!!-------]",
        "cancelled" => "[xxx-------]",
        _ => "[----------]",
    }
}
