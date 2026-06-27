// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::gateway) fn create_gateway_pairing(
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

pub(in crate::gateway) fn print_gateway_pairings(store: &LocalGatewayStore) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&store.redacted_pairing_reports()?)?
    );
    Ok(())
}
