// SPDX-License-Identifier: GPL-3.0-only

use super::*;

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
    WorkerStop(MessageWorkerStop),
    Daemon {
        #[command(subcommand)]
        command: MessageDaemonCommand,
    },
    Delivery {
        #[command(subcommand)]
        command: MessageDeliveryCommand,
    },
    Pairing {
        #[command(subcommand)]
        command: MessagePairingCommand,
    },
    Adapter {
        #[command(subcommand)]
        command: MessageAdapterCommand,
    },
    Outbox,
    Cancel(MessageCancel),
    Delete {
        id: String,
    },
    Webhook(MessageWebhook),
}

#[derive(Debug, Args)]
pub(crate) struct MessageSend {
    pub(in crate::gateway) content: String,
    #[arg(long, value_enum, default_value = "chat")]
    pub(in crate::gateway) kind: MessageKindArg,
    #[arg(long, default_value = "cli")]
    pub(in crate::gateway) source: String,
    #[arg(long)]
    pub(in crate::gateway) account: Option<String>,
    #[arg(long)]
    pub(in crate::gateway) peer: Option<String>,
    #[arg(long)]
    pub(in crate::gateway) thread: Option<String>,
    #[arg(long = "message-id")]
    pub(in crate::gateway) message_id: Option<String>,
    #[arg(long = "idempotency-key")]
    pub(in crate::gateway) idempotency_key: Option<String>,
    #[arg(long, value_name = "PROFILE")]
    pub(in crate::gateway) profile: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct MessageWorker {
    #[arg(long = "interval-seconds", default_value_t = 60)]
    pub(in crate::gateway) interval_seconds: u64,
    #[arg(long, default_value_t = 10)]
    pub(in crate::gateway) limit: usize,
    #[arg(long)]
    pub(in crate::gateway) once: bool,
}

#[derive(Debug, Args)]
pub(crate) struct MessageWorkerStop {
    #[arg(long, default_value = "operator requested stop")]
    pub(in crate::gateway) reason: String,
}

#[derive(Debug, Subcommand)]
pub(crate) enum MessageDaemonCommand {
    Start(MessageDaemonStart),
    Stop(MessageDaemonStop),
    Restart(MessageDaemonStart),
    Status,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct MessageDaemonStart {
    #[arg(long = "interval-seconds", default_value_t = 60)]
    pub(in crate::gateway) interval_seconds: u64,
    #[arg(long, default_value_t = 10)]
    pub(in crate::gateway) limit: usize,
}

#[derive(Debug, Args)]
pub(crate) struct MessageDaemonStop {
    #[arg(long, default_value = "operator requested stop")]
    pub(in crate::gateway) reason: String,
}

#[derive(Debug, Args)]
pub(crate) struct MessageCancel {
    pub(in crate::gateway) id: String,
    #[arg(long, default_value = "operator requested cancel")]
    pub(in crate::gateway) reason: String,
}

#[derive(Debug, Subcommand)]
pub(crate) enum MessageDeliveryCommand {
    Claim(MessageDeliveryClaim),
    Ack(MessageDeliveryAck),
    Fail(MessageDeliveryFail),
}

#[derive(Debug, Args)]
pub(crate) struct MessageDeliveryClaim {
    #[arg(long, default_value_t = 10)]
    pub(in crate::gateway) limit: usize,
    #[arg(long, default_value = "local-delivery-adapter")]
    pub(in crate::gateway) owner: String,
}

#[derive(Debug, Args)]
pub(crate) struct MessageDeliveryAck {
    pub(in crate::gateway) id: String,
    #[arg(long = "lease-owner")]
    pub(in crate::gateway) lease_owner: String,
    #[arg(long, default_value = "delivered")]
    pub(in crate::gateway) summary: String,
}

#[derive(Debug, Args)]
pub(crate) struct MessageDeliveryFail {
    pub(in crate::gateway) id: String,
    #[arg(long = "lease-owner")]
    pub(in crate::gateway) lease_owner: String,
    #[arg(long)]
    pub(in crate::gateway) reason: String,
    #[arg(long = "max-attempts", default_value_t = 3)]
    pub(in crate::gateway) max_attempts: u32,
    #[arg(long = "backoff-seconds", default_value_t = 60)]
    pub(in crate::gateway) backoff_seconds: u64,
}

#[derive(Debug, Subcommand)]
pub(crate) enum MessagePairingCommand {
    Create(MessagePairingCreate),
    List,
}

#[derive(Debug, Args)]
pub(crate) struct MessagePairingCreate {
    #[arg(long)]
    pub(in crate::gateway) source: String,
    #[arg(long)]
    pub(in crate::gateway) account: Option<String>,
    #[arg(long)]
    pub(in crate::gateway) peer: String,
}

#[derive(Debug, Subcommand)]
pub(crate) enum MessageAdapterCommand {
    List,
    Enqueue(MessageAdapterEnqueue),
    RenderDelivery(MessageAdapterRenderDelivery),
}

#[derive(Debug, Args)]
pub(crate) struct MessageAdapterEnqueue {
    pub(in crate::gateway) content: String,
    #[arg(long, default_value = "generic")]
    pub(in crate::gateway) platform: String,
    #[arg(long, value_enum, default_value = "chat")]
    pub(in crate::gateway) kind: MessageKindArg,
    #[arg(long)]
    pub(in crate::gateway) account: Option<String>,
    #[arg(long)]
    pub(in crate::gateway) peer: Option<String>,
    #[arg(long)]
    pub(in crate::gateway) thread: Option<String>,
    #[arg(long = "message-id")]
    pub(in crate::gateway) message_id: Option<String>,
    #[arg(long = "idempotency-key")]
    pub(in crate::gateway) idempotency_key: Option<String>,
    #[arg(long = "profile")]
    pub(in crate::gateway) agent: Option<String>,
    #[arg(long)]
    pub(in crate::gateway) safe_tools: bool,
}

#[derive(Debug, Args)]
pub(crate) struct MessageAdapterRenderDelivery {
    pub(in crate::gateway) id: String,
    #[arg(long, default_value = "generic")]
    pub(in crate::gateway) platform: String,
    #[arg(long)]
    pub(in crate::gateway) message_id: Option<String>,
}

#[derive(Debug, Clone, ValueEnum)]
pub(crate) enum MessageKindArg {
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
        MessageCommand::WorkerStop(args) => {
            request_message_worker_stop(args, paths)?;
        }
        MessageCommand::Daemon { command } => match command {
            MessageDaemonCommand::Start(args) => {
                start_message_daemon(args, paths, workspace, agent_override)?;
            }
            MessageDaemonCommand::Stop(args) => {
                request_message_daemon_stop(args, paths)?;
            }
            MessageDaemonCommand::Restart(args) => {
                restart_message_daemon(args, paths, workspace, agent_override)?;
            }
            MessageDaemonCommand::Status => {
                print_message_daemon_status(&store);
                print_gateway_status(&store)?;
            }
        },
        MessageCommand::Delivery { command } => match command {
            MessageDeliveryCommand::Claim(args) => claim_gateway_deliveries(args, &store)?,
            MessageDeliveryCommand::Ack(args) => ack_gateway_delivery(args, &store)?,
            MessageDeliveryCommand::Fail(args) => fail_gateway_delivery(args, &store)?,
        },
        MessageCommand::Pairing { command } => match command {
            MessagePairingCommand::Create(args) => create_gateway_pairing(args, &store)?,
            MessagePairingCommand::List => {
                print_gateway_pairings(&store)?;
                println!("gateway_pairings: {}", store.pairings_path().display());
            }
        },
        MessageCommand::Adapter { command } => match command {
            MessageAdapterCommand::List => print_gateway_adapters()?,
            MessageAdapterCommand::Enqueue(args) => enqueue_gateway_adapter_message(args, &store)?,
            MessageAdapterCommand::RenderDelivery(args) => {
                render_gateway_adapter_delivery(args, &store)?
            }
        },
        MessageCommand::Outbox => {
            println!("{}", serde_json::to_string_pretty(&store.deliveries()?)?);
            println!("gateway_outbox: {}", store.outbox_path().display());
        }
        MessageCommand::Cancel(args) => {
            cancel_gateway_message(args, &store)?;
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
