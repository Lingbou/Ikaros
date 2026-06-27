// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_surfaces::gateway::{
    GatewayAdapterEnqueueRequest, GatewayAdapterRenderDeliveryRequest,
    enqueue_gateway_adapter_message as surface_enqueue_gateway_adapter_message,
    render_gateway_adapter_delivery as surface_render_gateway_adapter_delivery,
};
use std::path::Path;

pub(in crate::gateway) fn print_gateway_adapters() -> Result<()> {
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

pub(in crate::gateway) fn enqueue_gateway_adapter_message(
    args: MessageAdapterEnqueue,
    gateway_dir: &Path,
) -> Result<()> {
    let result = surface_enqueue_gateway_adapter_message(
        gateway_dir,
        GatewayAdapterEnqueueRequest {
            platform: args.platform,
            content: args.content,
            kind: args.kind.into(),
            agent: args.agent,
            account: args.account,
            peer: args.peer,
            thread: args.thread,
            message_id: args.message_id,
            idempotency_key: args.idempotency_key,
            safe_tools: args.safe_tools,
        },
    )?;
    println!("message_adapter_enqueue: true");
    println!("message_adapter_platform: {}", result.platform.as_str());
    println!("message_adapter_safe_tools: {}", result.safe_tools);
    println!("enqueued: {}", result.message.id);
    println!("{}", serde_json::to_string_pretty(&result.message)?);
    let store = LocalGatewayStore::new(gateway_dir);
    println!("gateway_inbox: {}", store.inbox_path().display());
    Ok(())
}

pub(in crate::gateway) fn render_gateway_adapter_delivery(
    args: MessageAdapterRenderDelivery,
    gateway_dir: &Path,
) -> Result<()> {
    let result = surface_render_gateway_adapter_delivery(
        gateway_dir,
        GatewayAdapterRenderDeliveryRequest {
            platform: args.platform,
            id: args.id,
            message_id: args.message_id,
        },
    )?;
    println!("message_adapter_render_delivery: true");
    println!("message_adapter_platform: {}", result.platform.as_str());
    println!("{}", serde_json::to_string_pretty(&result.envelope)?);
    Ok(())
}
