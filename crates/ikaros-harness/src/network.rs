// SPDX-License-Identifier: GPL-3.0-only

use crate::{LocalExecutionEnv, NetworkEgress, NetworkEgressRequest, NetworkEgressResponse};
use ikaros_core::{IkarosError, Result, redact_secrets};
use std::{collections::BTreeSet, future::Future, pin::Pin, sync::Arc, time::Duration};
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct NetworkEgressPolicy {
    allowed_hosts: BTreeSet<String>,
}

impl NetworkEgressPolicy {
    pub fn deny_by_default() -> Self {
        Self::default()
    }

    pub fn allow_hosts(hosts: impl IntoIterator<Item = String>) -> Self {
        Self {
            allowed_hosts: hosts
                .into_iter()
                .map(|host| host.trim().trim_end_matches('.').to_ascii_lowercase())
                .filter(|host| !host.is_empty())
                .collect(),
        }
    }

    pub fn allows(&self, url: &str) -> Result<()> {
        let parsed = Url::parse(url)
            .map_err(|_| IkarosError::Message("network egress denied: invalid URL".into()))?;
        let Some(host) = parsed.host_str() else {
            return Err(IkarosError::Message(
                "network egress denied: URL has no host".into(),
            ));
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if self.allowed_hosts.contains(&host) {
            return Ok(());
        }
        Err(IkarosError::Message(format!(
            "network egress denied for host {}",
            redact_secrets(&host)
        )))
    }
}

#[derive(Clone)]
pub struct GovernedNetworkEgress {
    policy: NetworkEgressPolicy,
    transport: Arc<dyn NetworkEgress>,
}

#[derive(Clone)]
pub struct HttpNetworkEgress {
    client: reqwest::Client,
}

impl HttpNetworkEgress {
    pub fn new(timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|source| {
                IkarosError::Message(format!("failed to build network egress client: {source}"))
            })?;
        Ok(Self { client })
    }
}

impl GovernedNetworkEgress {
    pub fn deny_by_default() -> Self {
        Self::new(
            NetworkEgressPolicy::deny_by_default(),
            Arc::new(LocalExecutionEnv),
        )
    }

    pub fn new(policy: NetworkEgressPolicy, transport: Arc<dyn NetworkEgress>) -> Self {
        Self { policy, transport }
    }

    pub fn policy(&self) -> &NetworkEgressPolicy {
        &self.policy
    }
}

impl NetworkEgress for GovernedNetworkEgress {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        Box::pin(async move {
            self.policy.allows(&request.url)?;
            self.transport.send_network_request(request).await
        })
    }
}

impl NetworkEgress for HttpNetworkEgress {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        Box::pin(async move {
            let method = request
                .method
                .parse::<reqwest::Method>()
                .map_err(|source| {
                    IkarosError::Message(format!("invalid network egress method: {source}"))
                })?;
            let mut builder = self.client.request(method, &request.url);
            for (name, value) in request.headers {
                builder = builder.header(name, value);
            }
            if let Some(body) = request.body {
                builder = builder.body(body);
            }
            let response = builder.send().await.map_err(|source| {
                IkarosError::Message(format!("network egress request failed: {source}"))
            })?;
            let status = response.status().as_u16();
            let body = response.text().await.map_err(|source| {
                IkarosError::Message(format!("failed to read network egress response: {source}"))
            })?;
            Ok(NetworkEgressResponse { status, body })
        })
    }
}
