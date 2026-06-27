// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::message) fn cancel_gateway_message(
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

pub(in crate::message) fn claim_gateway_deliveries(
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

pub(in crate::message) fn ack_gateway_delivery(
    args: MessageDeliveryAck,
    store: &LocalGatewayStore,
) -> Result<()> {
    let claim = find_gateway_delivery_claim(store, &args.id, &args.lease_owner)?;
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

pub(in crate::message) fn fail_gateway_delivery(
    args: MessageDeliveryFail,
    store: &LocalGatewayStore,
) -> Result<()> {
    if args.max_attempts == 0 {
        anyhow::bail!("delivery max-attempts must be greater than zero");
    }
    let claim = find_gateway_delivery_claim(store, &args.id, &args.lease_owner)?;
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

pub(in crate::message) fn find_gateway_delivery_claim(
    store: &LocalGatewayStore,
    id: &str,
    lease_owner: &str,
) -> Result<GatewayDelivery> {
    let expected_owner = redact_secrets(lease_owner);
    let Some(delivery) = store
        .deliveries()?
        .into_iter()
        .find(|delivery| delivery.id == id)
    else {
        anyhow::bail!("delivery not found: {}", redact_secrets(id));
    };
    if delivery.status != GatewayDeliveryStatus::Processing {
        anyhow::bail!(
            "delivery is not processing: id={} status={:?}",
            redact_secrets(&delivery.id),
            delivery.status
        );
    }
    if delivery.lease_owner.as_deref() != Some(expected_owner.as_str()) {
        anyhow::bail!(
            "delivery lease owner mismatch: id={} expected_owner={}",
            redact_secrets(&delivery.id),
            expected_owner
        );
    }
    Ok(delivery)
}
