// SPDX-License-Identifier: GPL-3.0-only

use crate::GatewayRoute;
use ikaros_core::redact_secrets;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default)]
pub struct MessageWebhookAcl {
    allowed_sources: BTreeSet<String>,
    allowed_accounts: BTreeSet<String>,
    allowed_peers: BTreeSet<String>,
    allowed_threads: BTreeSet<String>,
}

impl MessageWebhookAcl {
    pub fn from_allow_lists(
        sources: &[String],
        accounts: &[String],
        peers: &[String],
        threads: &[String],
    ) -> Option<Self> {
        let acl = Self {
            allowed_sources: cleaned_set(sources, Normalize::Lowercase),
            allowed_accounts: cleaned_set(accounts, Normalize::Preserve),
            allowed_peers: cleaned_set(peers, Normalize::Preserve),
            allowed_threads: cleaned_set(threads, Normalize::Preserve),
        };
        (!acl.is_empty()).then_some(acl)
    }

    pub fn validate_route(&self, route: &GatewayRoute) -> Result<(), &'static str> {
        if !self.allowed_sources.is_empty()
            && !self
                .allowed_sources
                .contains(&route.source.trim().to_ascii_lowercase())
        {
            return Err("source");
        }
        if !self.allowed_accounts.is_empty()
            && !self
                .allowed_accounts
                .contains(route_account(route).unwrap_or_default())
        {
            return Err("account");
        }
        if !self.allowed_peers.is_empty()
            && !self
                .allowed_peers
                .contains(route_peer(route).unwrap_or_default())
        {
            return Err("peer");
        }
        if !self.allowed_threads.is_empty()
            && !self
                .allowed_threads
                .contains(route_thread(route).unwrap_or_default())
        {
            return Err("thread");
        }
        Ok(())
    }

    fn is_empty(&self) -> bool {
        self.allowed_sources.is_empty()
            && self.allowed_accounts.is_empty()
            && self.allowed_peers.is_empty()
            && self.allowed_threads.is_empty()
    }
}

#[derive(Debug, Clone, Copy)]
enum Normalize {
    Preserve,
    Lowercase,
}

fn cleaned_set(values: &[String], normalize: Normalize) -> BTreeSet<String> {
    values
        .iter()
        .map(|value| redact_secrets(value.trim()))
        .map(|value| match normalize {
            Normalize::Preserve => value,
            Normalize::Lowercase => value.to_ascii_lowercase(),
        })
        .filter(|value| !value.is_empty())
        .collect()
}

fn route_account(route: &GatewayRoute) -> Option<&str> {
    route.session_source.as_ref()?.account.as_deref()
}

fn route_peer(route: &GatewayRoute) -> Option<&str> {
    route.session_source.as_ref()?.peer.as_deref()
}

fn route_thread(route: &GatewayRoute) -> Option<&str> {
    route.session_source.as_ref()?.thread.as_deref()
}
