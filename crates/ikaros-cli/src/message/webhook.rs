// SPDX-License-Identifier: GPL-3.0-only

mod acl;
mod http;
mod payload;
mod response;
mod server;

use clap::Args;

pub(crate) use server::serve_message_webhook;

#[derive(Debug, Args)]
pub(crate) struct MessageWebhook {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8002)]
    port: u16,
    #[arg(long = "max-body-bytes", default_value_t = 65536)]
    max_body_bytes: usize,
    #[arg(long = "hmac-secret", value_name = "SECRET")]
    hmac_secret: Option<String>,
    #[arg(long = "allow-source", value_name = "SOURCE")]
    allow_sources: Vec<String>,
    #[arg(long = "allow-account", value_name = "ACCOUNT")]
    allow_accounts: Vec<String>,
    #[arg(long = "allow-peer", value_name = "PEER")]
    allow_peers: Vec<String>,
    #[arg(long = "allow-thread", value_name = "THREAD")]
    allow_threads: Vec<String>,
    #[arg(long = "require-pairing")]
    require_pairing: bool,
    #[arg(long = "unsafe-tools")]
    unsafe_tools: bool,
    #[arg(long, hide = true)]
    once: bool,
}

#[cfg(test)]
mod tests;
