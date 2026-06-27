// SPDX-License-Identifier: GPL-3.0-only
//! Loopback HTTP webhook adapter for enqueueing gateway messages.

mod acl;
mod http;
mod payload;
mod response;
mod server;

pub use acl::MessageWebhookAcl;
pub use http::{
    HttpHeaders, parse_http_request_line, read_http_headers, write_webhook_http_response,
};
pub use payload::{parse_webhook_pairing_code, parse_webhook_payload};
pub use response::{
    MessageWebhookHttpResponse, MessageWebhookIngressPolicy, is_message_route,
    webhook_http_response,
};
pub use server::{
    MessageWebhookServerConfig, handle_webhook_stream, require_loopback_host,
    serve_message_webhook, verify_webhook_signature,
};

#[cfg(test)]
mod tests;
