// SPDX-License-Identifier: GPL-3.0-only

mod webhook;

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use ikaros_core::IkarosPaths;
use ikaros_gateway::{GatewayMessageKind, GatewayMessageStatus, GatewayRoute, LocalGatewayStore};
use ikaros_runtime::{drain_gateway_messages, run_gateway_worker_tick};
use std::{path::Path, time::Duration};
use tokio::time::sleep;
use webhook::{MessageWebhook, serve_message_webhook};

#[derive(Debug, Subcommand)]
pub(crate) enum MessageCommand {
    Send(MessageSend),
    List {
        #[arg(long)]
        all: bool,
    },
    Drain {
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        dry_run: bool,
    },
    Worker(MessageWorker),
    Outbox,
    Delete {
        id: String,
    },
    Webhook(MessageWebhook),
}

#[derive(Debug, Args)]
pub(crate) struct MessageSend {
    content: String,
    #[arg(long, value_enum, default_value = "chat")]
    kind: MessageKindArg,
    #[arg(long, default_value = "cli")]
    source: String,
    #[arg(long, value_name = "PROFILE")]
    profile: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct MessageWorker {
    #[arg(long = "interval-seconds", default_value_t = 60)]
    interval_seconds: u64,
    #[arg(long, default_value_t = 10)]
    limit: usize,
    #[arg(long)]
    once: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum MessageKindArg {
    Chat,
    Task,
}

impl From<MessageKindArg> for GatewayMessageKind {
    fn from(value: MessageKindArg) -> Self {
        match value {
            MessageKindArg::Chat => Self::Chat,
            MessageKindArg::Task => Self::Task,
        }
    }
}

pub(crate) async fn message_command(
    command: MessageCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    match command {
        MessageCommand::Send(args) => {
            let agent = args
                .profile
                .or_else(|| agent_override.map(ToOwned::to_owned));
            let message = store.enqueue(GatewayRoute::new(
                args.source,
                args.kind.into(),
                args.content,
                agent,
            ))?;
            println!("enqueued: {}", message.id);
            println!("{}", serde_json::to_string_pretty(&message)?);
            println!("gateway_inbox: {}", store.inbox_path().display());
        }
        MessageCommand::List { all } => {
            let messages = store
                .list()?
                .into_iter()
                .filter(|message| all || message.status == GatewayMessageStatus::Pending)
                .collect::<Vec<_>>();
            println!("{}", serde_json::to_string_pretty(&messages)?);
            println!("gateway_inbox: {}", store.inbox_path().display());
        }
        MessageCommand::Drain { limit, dry_run } => {
            if dry_run {
                let messages = store.pending(limit)?;
                println!("{}", serde_json::to_string_pretty(&messages)?);
                println!("gateway_inbox: {}", store.inbox_path().display());
                println!("gateway_outbox: {}", store.outbox_path().display());
                return Ok(());
            }
            let messages = store.claim_pending(limit)?;
            let reports =
                drain_gateway_messages(messages, &store, paths, workspace, agent_override).await?;
            println!("{}", serde_json::to_string_pretty(&reports)?);
            println!("gateway_inbox: {}", store.inbox_path().display());
            println!("gateway_outbox: {}", store.outbox_path().display());
        }
        MessageCommand::Worker(args) => {
            run_message_worker(args, &store, paths, workspace, agent_override).await?;
        }
        MessageCommand::Outbox => {
            println!("{}", serde_json::to_string_pretty(&store.deliveries()?)?);
            println!("gateway_outbox: {}", store.outbox_path().display());
        }
        MessageCommand::Delete { id } => {
            let deleted = store.delete(&id)?;
            println!("deleted: {deleted}");
            println!("gateway_inbox: {}", store.inbox_path().display());
        }
        MessageCommand::Webhook(args) => serve_message_webhook(args, paths)?,
    }
    Ok(())
}

async fn run_message_worker(
    args: MessageWorker,
    store: &LocalGatewayStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    if args.interval_seconds == 0 {
        anyhow::bail!("message worker interval must be greater than zero");
    }
    if args.limit == 0 {
        anyhow::bail!("message worker limit must be greater than zero");
    }
    println!("message_worker: started");
    println!("interval_seconds: {}", args.interval_seconds);
    println!("limit: {}", args.limit);
    println!("gateway_inbox: {}", store.inbox_path().display());
    println!("gateway_outbox: {}", store.outbox_path().display());
    loop {
        let report =
            run_gateway_worker_tick(store, args.limit, paths, workspace, agent_override).await?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if args.once {
            break;
        }
        sleep(Duration::from_secs(args.interval_seconds)).await;
    }
    Ok(())
}
