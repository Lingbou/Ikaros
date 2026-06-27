// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn run_gateway_daemon_workbench_command(
    args: &[&str],
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    match args {
        [] | ["status"] => {
            print_message_daemon_status(&store);
            print_gateway_status(&store)?;
        }
        ["start", rest @ ..] => {
            let start = parse_message_daemon_start_args(rest)?;
            start_message_daemon(start, paths, workspace, agent_override)?;
        }
        ["stop", rest @ ..] => {
            let stop = parse_message_daemon_stop_args(rest)?;
            request_message_daemon_stop(stop, paths)?;
        }
        ["restart", rest @ ..] => {
            let start = parse_message_daemon_start_args(rest)?;
            restart_message_daemon(start, paths, workspace, agent_override)?;
        }
        ["help"] | ["--help"] => print_gateway_daemon_workbench_usage(),
        _ => print_gateway_daemon_workbench_usage(),
    }
    Ok(())
}

pub(crate) fn run_gateway_adapter_workbench_command(
    args: &[&str],
    paths: &IkarosPaths,
) -> Result<()> {
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    match args {
        [] | ["list"] | ["status"] => print_gateway_adapters()?,
        ["enqueue", rest @ ..] => {
            let enqueue = parse_message_adapter_enqueue_args(rest)?;
            enqueue_gateway_adapter_message(enqueue, &store)?;
        }
        ["render-delivery" | "render_delivery", rest @ ..] => {
            let render = parse_message_adapter_render_delivery_args(rest)?;
            render_gateway_adapter_delivery(render, &store)?;
        }
        ["help"] | ["--help"] => print_gateway_adapter_workbench_usage(),
        _ => print_gateway_adapter_workbench_usage(),
    }
    Ok(())
}

pub(in crate::message) fn parse_message_daemon_start_args(
    args: &[&str],
) -> Result<MessageDaemonStart> {
    let mut start = MessageDaemonStart {
        interval_seconds: 60,
        limit: 10,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--interval-seconds" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow::anyhow!(
                        "usage: /gateway daemon start [--interval-seconds N] [--limit N]"
                    )
                })?;
                start.interval_seconds = value
                    .parse::<u64>()
                    .with_context(|| "--interval-seconds must be a positive integer")?;
                index += 2;
            }
            "--limit" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("usage: /gateway daemon start [--limit N]"))?;
                start.limit = value
                    .parse::<usize>()
                    .with_context(|| "--limit must be a positive integer")?;
                index += 2;
            }
            "help" | "--help" => {
                print_gateway_daemon_workbench_usage();
                index += 1;
            }
            value => {
                anyhow::bail!("unsupported /gateway daemon start option: {value}");
            }
        }
    }
    Ok(start)
}

pub(in crate::message) fn parse_message_daemon_stop_args(
    args: &[&str],
) -> Result<MessageDaemonStop> {
    let mut stop = MessageDaemonStop {
        reason: "operator requested stop".into(),
    };
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--reason" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow::anyhow!("usage: /gateway daemon stop [--reason TEXT]")
                })?;
                stop.reason = (*value).to_owned();
                index += 2;
            }
            "help" | "--help" => {
                print_gateway_daemon_workbench_usage();
                index += 1;
            }
            value => {
                anyhow::bail!("unsupported /gateway daemon stop option: {value}");
            }
        }
    }
    Ok(stop)
}

pub(in crate::message) fn print_gateway_daemon_workbench_usage() {
    println!(
        "gateway_daemon_usage: /gateway daemon [status|start|stop|restart] [--interval-seconds N] [--limit N] [--reason TEXT]"
    );
}

pub(in crate::message) fn parse_message_adapter_enqueue_args(
    args: &[&str],
) -> Result<MessageAdapterEnqueue> {
    let mut content = Vec::new();
    let mut platform = "generic".to_owned();
    let mut kind = MessageKindArg::Chat;
    let mut account = None;
    let mut peer = None;
    let mut thread = None;
    let mut message_id = None;
    let mut idempotency_key = None;
    let mut agent = None;
    let mut safe_tools = false;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--platform" => {
                platform = next_workbench_arg(args, index, "--platform")?.to_owned();
                index += 2;
            }
            "--kind" => {
                kind = parse_message_kind_arg(next_workbench_arg(args, index, "--kind")?)?;
                index += 2;
            }
            "--account" => {
                account = Some(next_workbench_arg(args, index, "--account")?.to_owned());
                index += 2;
            }
            "--peer" => {
                peer = Some(next_workbench_arg(args, index, "--peer")?.to_owned());
                index += 2;
            }
            "--thread" => {
                thread = Some(next_workbench_arg(args, index, "--thread")?.to_owned());
                index += 2;
            }
            "--message-id" => {
                message_id = Some(next_workbench_arg(args, index, "--message-id")?.to_owned());
                index += 2;
            }
            "--idempotency-key" => {
                idempotency_key =
                    Some(next_workbench_arg(args, index, "--idempotency-key")?.to_owned());
                index += 2;
            }
            "--profile" | "--agent" => {
                agent = Some(next_workbench_arg(args, index, args[index])?.to_owned());
                index += 2;
            }
            "--safe-tools" => {
                safe_tools = true;
                index += 1;
            }
            "help" | "--help" => {
                print_gateway_adapter_workbench_usage();
                index += 1;
            }
            value if value.starts_with("--") => {
                anyhow::bail!("unsupported /gateway adapter enqueue option: {value}");
            }
            value => {
                content.push(value);
                index += 1;
            }
        }
    }
    let content = content.join(" ");
    if content.trim().is_empty() {
        anyhow::bail!(
            "usage: /gateway adapter enqueue <content> [--platform generic|webhook|telegram|discord|slack] [--kind chat|task]"
        );
    }
    Ok(MessageAdapterEnqueue {
        content,
        platform,
        kind,
        account,
        peer,
        thread,
        message_id,
        idempotency_key,
        agent,
        safe_tools,
    })
}

pub(in crate::message) fn parse_message_adapter_render_delivery_args(
    args: &[&str],
) -> Result<MessageAdapterRenderDelivery> {
    let mut id = None;
    let mut platform = "generic".to_owned();
    let mut message_id = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--platform" => {
                platform = next_workbench_arg(args, index, "--platform")?.to_owned();
                index += 2;
            }
            "--message-id" => {
                message_id = Some(next_workbench_arg(args, index, "--message-id")?.to_owned());
                index += 2;
            }
            "help" | "--help" => {
                print_gateway_adapter_workbench_usage();
                index += 1;
            }
            value if value.starts_with("--") => {
                anyhow::bail!("unsupported /gateway adapter render-delivery option: {value}");
            }
            value if id.is_none() => {
                id = Some(value.to_owned());
                index += 1;
            }
            value => {
                anyhow::bail!("unexpected /gateway adapter render-delivery argument: {value}");
            }
        }
    }
    Ok(MessageAdapterRenderDelivery {
        id: id.ok_or_else(|| {
            anyhow::anyhow!("usage: /gateway adapter render-delivery <delivery-id>")
        })?,
        platform,
        message_id,
    })
}

pub(in crate::message) fn next_workbench_arg<'a>(
    args: &'a [&str],
    index: usize,
    option: &str,
) -> Result<&'a str> {
    args.get(index + 1)
        .copied()
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| anyhow::anyhow!("missing value for {option}"))
}

pub(in crate::message) fn parse_message_kind_arg(value: &str) -> Result<MessageKindArg> {
    match value.trim().to_ascii_lowercase().as_str() {
        "chat" => Ok(MessageKindArg::Chat),
        "task" => Ok(MessageKindArg::Task),
        _ => anyhow::bail!("message kind must be chat or task"),
    }
}

pub(in crate::message) fn print_gateway_adapter_workbench_usage() {
    println!("gateway_adapter_usage: /gateway adapter list");
    println!(
        "gateway_adapter_usage: /gateway adapter enqueue <content> [--platform generic|webhook|telegram|discord|slack] [--kind chat|task] [--account ID] [--peer ID] [--thread ID] [--message-id ID] [--idempotency-key KEY] [--profile AGENT] [--safe-tools]"
    );
    println!(
        "gateway_adapter_usage: /gateway adapter render-delivery <delivery-id> [--platform generic|webhook|telegram|discord|slack] [--message-id MESSAGE_ID]"
    );
}
