// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_api::{ApiServeOptions, serve_api};
use ikaros_core::IkarosPaths;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum ApiCommand {
    /// Serve a local OpenAI-compatible API surface.
    Serve(ApiServe),
}

#[derive(Debug, Args)]
pub(crate) struct ApiServe {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8003)]
    port: u16,
    #[arg(long, default_value_t = 64 * 1024)]
    max_body_bytes: usize,
    /// Optional bearer token required for /v1/* routes. Repeat to allow key rotation.
    #[arg(long, value_name = "TOKEN")]
    bearer_token: Vec<String>,
    /// Per-process request limit per minute. Use 0 to disable.
    #[arg(long, default_value_t = 120)]
    rate_limit_per_minute: u32,
    #[arg(long)]
    once: bool,
}

impl From<ApiServe> for ApiServeOptions {
    fn from(args: ApiServe) -> Self {
        Self {
            host: args.host,
            port: args.port,
            max_body_bytes: args.max_body_bytes,
            bearer_tokens: args.bearer_token,
            rate_limit_per_minute: args.rate_limit_per_minute,
            once: args.once,
        }
    }
}

pub(crate) async fn api_command(
    command: ApiCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        ApiCommand::Serve(args) => serve_api(args.into(), paths, workspace, agent_override),
    }
}
