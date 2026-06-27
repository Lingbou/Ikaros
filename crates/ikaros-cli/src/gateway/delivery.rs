// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::gateway) fn cancel_gateway_message(
    args: MessageCancel,
    store: &LocalGatewayStore,
) -> Result<()> {
    let reason = redact_secrets(&args.reason);
    let update = store.cancel(&args.id, &reason)?;
    match update {
        Some(message) => {
            println!(
                "message_cancelled: true id={} status={:?} reason={}",
                redact_secrets(&message.id),
                message.status,
                message
                    .summary
                    .as_deref()
                    .map(redact_secrets)
                    .unwrap_or_else(|| reason.clone())
            );
        }
        None => {
            println!(
                "message_cancelled: false id={} reason={}",
                redact_secrets(&args.id),
                reason
            );
        }
    }
    println!("gateway_inbox: {}", store.inbox_path().display());
    Ok(())
}

pub(in crate::gateway) fn claim_gateway_deliveries(
    args: MessageDeliveryClaim,
    store: &LocalGatewayStore,
) -> Result<()> {
    if args.limit == 0 {
        anyhow::bail!("delivery claim limit must be greater than zero");
    }
    let owner = redact_secrets(&args.owner);
    let deliveries = store.claim_pending_deliveries_with_owner(args.limit, owner)?;
    println!("message_delivery_claimed: {}", deliveries.len());
    println!("{}", serde_json::to_string_pretty(&deliveries)?);
    println!("gateway_outbox: {}", store.outbox_path().display());
    Ok(())
}

pub(in crate::gateway) fn ack_gateway_delivery(
    args: MessageDeliveryAck,
    store: &LocalGatewayStore,
) -> Result<()> {
    let claim = store.delivery_claim_by_owner(&args.id, &args.lease_owner)?;
    let update = store.record_delivery_success_for_claim(&claim, &args.summary)?;
    match update {
        Some(delivery) => println!(
            "message_delivery_delivered: true id={} status={:?} summary={}",
            redact_secrets(&delivery.id),
            delivery.status,
            delivery
                .summary
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| redact_secrets(&args.summary))
        ),
        None => println!(
            "message_delivery_delivered: false id={} status=unchanged summary=stale_claim",
            redact_secrets(&args.id)
        ),
    }
    println!("gateway_outbox: {}", store.outbox_path().display());
    Ok(())
}

pub(in crate::gateway) fn fail_gateway_delivery(
    args: MessageDeliveryFail,
    store: &LocalGatewayStore,
) -> Result<()> {
    if args.max_attempts == 0 {
        anyhow::bail!("delivery max-attempts must be greater than zero");
    }
    let claim = store.delivery_claim_by_owner(&args.id, &args.lease_owner)?;
    let update = store.record_delivery_failure_for_claim(
        &claim,
        &args.reason,
        args.max_attempts,
        args.backoff_seconds,
    )?;
    match update {
        Some(delivery) => println!(
            "message_delivery_failed: true id={} status={:?} attempts={} last_error={}",
            redact_secrets(&delivery.id),
            delivery.status,
            delivery.attempt_count,
            delivery
                .last_error
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| redact_secrets(&args.reason))
        ),
        None => println!(
            "message_delivery_failed: false id={} status=unchanged last_error=stale_claim",
            redact_secrets(&args.id)
        ),
    }
    println!("gateway_outbox: {}", store.outbox_path().display());
    Ok(())
}
