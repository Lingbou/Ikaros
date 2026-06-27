// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::message) fn create_gateway_pairing(
    args: MessagePairingCreate,
    store: &LocalGatewayStore,
) -> Result<()> {
    let pairing = store.create_pairing(&args.source, args.account.as_deref(), &args.peer)?;
    println!("message_pairing_created: true");
    println!("message_pairing_code: {}", pairing.code);
    println!(
        "message_pairing_source: {}",
        redact_secrets(&pairing.source)
    );
    println!(
        "message_pairing_account: {}",
        pairing
            .account
            .as_deref()
            .map(redact_secrets)
            .unwrap_or_else(|| "none".into())
    );
    println!("message_pairing_peer: {}", redact_secrets(&pairing.peer));
    println!("gateway_pairings: {}", store.pairings_path().display());
    Ok(())
}

pub(in crate::message) fn print_gateway_pairings(store: &LocalGatewayStore) -> Result<()> {
    let pairings = store
        .pairings()?
        .into_iter()
        .map(|pairing| {
            serde_json::json!({
                "code": "[REDACTED_PAIRING_CODE]",
                "source": redact_secrets(&pairing.source),
                "account": pairing.account.as_deref().map(redact_secrets),
                "peer": redact_secrets(&pairing.peer),
                "status": pairing.status,
                "created_at": pairing.created_at,
                "paired_at": pairing.paired_at,
                "revoked_at": pairing.revoked_at,
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&pairings)?);
    Ok(())
}
