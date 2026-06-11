// SPDX-License-Identifier: GPL-3.0-only

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
    #[arg(long, hide = true)]
    once: bool,
}

#[cfg(test)]
mod tests;
