// SPDX-License-Identifier: GPL-3.0-only

mod webhook;

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use ikaros_core::{IkarosPaths, redact_secrets};
use ikaros_gateway::{
    GatewayMessage, GatewayMessageKind, GatewayMessageStatus, GatewayRoute, GatewaySessionSource,
    LocalGatewayStore,
};
use ikaros_runtime::{drain_gateway_messages, gateway_session_id, run_gateway_worker_tick};
use std::{path::Path, time::Duration};
use tokio::time::sleep;
use webhook::{MessageWebhook, serve_message_webhook};

#[derive(Debug, Subcommand)]
pub(crate) enum MessageCommand {
    Send(MessageSend),
    Status,
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
    #[arg(long)]
    account: Option<String>,
    #[arg(long)]
    peer: Option<String>,
    #[arg(long)]
    thread: Option<String>,
    #[arg(long = "message-id")]
    message_id: Option<String>,
    #[arg(long = "idempotency-key")]
    idempotency_key: Option<String>,
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
            let mut route =
                GatewayRoute::new(args.source.clone(), args.kind.into(), args.content, agent);
            if args.account.is_some()
                || args.peer.is_some()
                || args.thread.is_some()
                || args.message_id.is_some()
            {
                route = route.with_session_source(GatewaySessionSource {
                    channel: args.source,
                    account: args.account,
                    peer: args.peer,
                    thread: args.thread,
                    message_id: args.message_id,
                });
            }
            if let Some(key) = args.idempotency_key {
                route = route.with_idempotency_key(key);
            }
            let message = store.enqueue(route)?;
            println!("enqueued: {}", message.id);
            println!("{}", serde_json::to_string_pretty(&message)?);
            println!("gateway_inbox: {}", store.inbox_path().display());
        }
        MessageCommand::Status => {
            print_gateway_status(&store)?;
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

fn print_gateway_status(store: &LocalGatewayStore) -> Result<()> {
    let messages = store.list()?;
    let deliveries = store.deliveries()?;
    let pending = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Pending)
        .count();
    let processing = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Processing)
        .count();
    let processed = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Processed)
        .count();
    let failed = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Failed)
        .count();
    println!("gateway_status:");
    println!("gateway_inbox: {}", store.inbox_path().display());
    println!("gateway_outbox: {}", store.outbox_path().display());
    println!("gateway_pending: {pending}");
    println!("gateway_processing: {processing}");
    println!("gateway_processed: {processed}");
    println!("gateway_failed: {failed}");
    println!("gateway_deliveries: {}", deliveries.len());
    print_gateway_sessions(&messages);
    Ok(())
}

fn print_gateway_sessions(messages: &[GatewayMessage]) {
    let mut sessions = messages
        .iter()
        .map(|message| {
            let session_id = gateway_session_id(message);
            (
                session_id.to_string(),
                message.source.as_str(),
                message
                    .session_source
                    .as_ref()
                    .and_then(|source| source.thread.as_deref())
                    .unwrap_or(message.id.as_str()),
                message,
            )
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.0.cmp(&right.0));
    sessions.dedup_by(|left, right| left.0 == right.0);
    println!("gateway_sessions: {}", sessions.len());
    for (session_id, source, thread, message) in sessions.into_iter().rev().take(5) {
        println!(
            "gateway_session: session={} source={} thread={} last_status={:?}",
            redact_secrets(&session_id),
            redact_secrets(source),
            redact_secrets(thread),
            message.status
        );
        println!(
            "  resume: ikaros chat --chat-session {} --message \"...\"",
            redact_secrets(&session_id)
        );
    }
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
