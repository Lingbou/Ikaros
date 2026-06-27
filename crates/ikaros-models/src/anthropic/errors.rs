// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::redact_secrets;

pub(super) fn provider_http_error(status: u16, text: &str) -> String {
    format!(
        "Anthropic model provider returned HTTP {status}: {}",
        redact_secrets(text)
    )
}

pub(super) fn stream_http_error(status: u16, text: &str) -> String {
    format!(
        "Anthropic model stream returned HTTP {status}: {}",
        redact_secrets(text)
    )
}
