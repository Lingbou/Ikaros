// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::message) fn print_gateway_adapters() -> Result<()> {
    let adapters = builtin_gateway_adapters()
        .into_iter()
        .map(|adapter| {
            serde_json::json!({
                "id": adapter.id,
                "platform": adapter.platform,
                "display_name": adapter.display_name,
                "inbound": adapter.inbound,
                "outbound": adapter.outbound,
                "requires_pairing": adapter.requires_pairing,
                "supports_hmac": adapter.supports_hmac,
                "safe_tools_default": adapter.safe_tools_default,
                "capabilities": adapter.capabilities,
                "commands": {
                    "enqueue": format!(
                        "ikaros message adapter enqueue --platform {} <content>",
                        adapter.platform.as_str()
                    ),
                    "render_delivery": format!(
                        "ikaros message adapter render-delivery --platform {} <delivery-id>",
                        adapter.platform.as_str()
                    ),
                },
            })
        })
        .collect::<Vec<_>>();
    println!("message_adapters: {}", adapters.len());
    println!("{}", serde_json::to_string_pretty(&adapters)?);
    Ok(())
}

pub(in crate::message) fn enqueue_gateway_adapter_message(
    args: MessageAdapterEnqueue,
    store: &LocalGatewayStore,
) -> Result<()> {
    let platform = GatewayPlatform::parse(&args.platform)?;
    let descriptor = builtin_gateway_adapters()
        .into_iter()
        .find(|adapter| adapter.platform == platform);
    let safe_tools = args.safe_tools
        || descriptor
            .as_ref()
            .is_some_and(|adapter| adapter.safe_tools_default);
    let envelope = GatewayInboundEnvelope {
        platform,
        content: args.content,
        kind: args.kind.into(),
        agent: args.agent,
        account: args.account,
        peer: args.peer,
        thread: args.thread,
        message_id: args.message_id,
        idempotency_key: args.idempotency_key,
        safe_tools,
    };
    let route = envelope.to_route();
    let message = store.enqueue(route)?;
    println!("message_adapter_enqueue: true");
    println!("message_adapter_platform: {}", platform.as_str());
    println!("message_adapter_safe_tools: {safe_tools}");
    println!("enqueued: {}", message.id);
    println!("{}", serde_json::to_string_pretty(&message)?);
    println!("gateway_inbox: {}", store.inbox_path().display());
    Ok(())
}

pub(in crate::message) fn render_gateway_adapter_delivery(
    args: MessageAdapterRenderDelivery,
    store: &LocalGatewayStore,
) -> Result<()> {
    let platform = GatewayPlatform::parse(&args.platform)?;
    let deliveries = store.deliveries()?;
    let Some(delivery) = deliveries.iter().find(|delivery| delivery.id == args.id) else {
        anyhow::bail!("delivery not found: {}", redact_secrets(&args.id));
    };
    let mut source = if let Some(message_id) = args.message_id.as_deref() {
        gateway_message_source_by_id(store, message_id)?
    } else {
        None
    };
    if source.is_none() {
        source = gateway_message_source_by_id(store, &delivery.message_id)?;
    }
    let envelope = GatewayOutboundEnvelope::from_delivery(platform, delivery, source.as_ref());
    println!("message_adapter_render_delivery: true");
    println!("message_adapter_platform: {}", platform.as_str());
    println!("{}", serde_json::to_string_pretty(&envelope)?);
    Ok(())
}

pub(in crate::message) fn gateway_message_source_by_id(
    store: &LocalGatewayStore,
    message_id: &str,
) -> Result<Option<GatewaySessionSource>> {
    Ok(store
        .list()?
        .into_iter()
        .find(|message| message.id == message_id)
        .and_then(|message| message.session_source))
}
