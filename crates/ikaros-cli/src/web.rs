// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_approval_hint, print_skill_result, session_and_registry};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use ikaros_core::IkarosPaths;
use serde_json::json;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum WebCommand {
    Search(WebSearchArgs),
    Extract(WebExtractArgs),
}

#[derive(Debug, Args)]
pub(crate) struct WebSearchArgs {
    query: Vec<String>,
    #[arg(long, default_value_t = 5)]
    max_results: usize,
    #[arg(long, default_value = "duckduckgo-html")]
    provider: String,
    #[arg(long)]
    endpoint: Option<String>,
    #[arg(long)]
    api_key: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct WebExtractArgs {
    url: String,
    #[arg(long)]
    max_bytes: Option<usize>,
    #[arg(long)]
    max_chars: Option<usize>,
}

pub(crate) async fn web_command(
    command: WebCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = match command {
        WebCommand::Search(args) => {
            let query = args.query.join(" ");
            if query.trim().is_empty() {
                anyhow::bail!("web search query must not be empty");
            }
            let mut input = json!({
                "query": query,
                "max_results": args.max_results,
                "provider": args.provider,
            });
            if let Some(endpoint) = args.endpoint {
                input["endpoint"] = json!(endpoint);
            }
            if let Some(api_key) = args.api_key {
                input["api_key"] = json!(api_key);
            }
            session
                .execute_skill(&registry, "web_search", input)
                .await
                .with_context(|| "web_search failed")?
        }
        WebCommand::Extract(args) => {
            let mut input = json!({
                "url": args.url,
            });
            if let Some(max_bytes) = args.max_bytes {
                input["max_bytes"] = json!(max_bytes);
            }
            if let Some(max_chars) = args.max_chars {
                input["max_chars"] = json!(max_chars);
            }
            session
                .execute_skill(&registry, "web_extract", input)
                .await
                .with_context(|| "web_extract failed")?
        }
    };
    print_skill_result(&result)?;
    print_approval_hint(&result);
    Ok(())
}
