// SPDX-License-Identifier: GPL-3.0-only

use crate::{LocalExecutionEnv, NetworkEgress, NetworkEgressRequest, NetworkEgressResponse};
use ikaros_core::{IkarosError, Result, redact_secrets};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use tokio::net::lookup_host;
use url::Url;

const MAX_NETWORK_EGRESS_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

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
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(IkarosError::Message(format!(
                "network egress denied: unsupported URL scheme {}",
                redact_secrets(parsed.scheme())
            )));
        }
        let Some(host) = parsed.host_str() else {
            return Err(IkarosError::Message(
                "network egress denied: URL has no host".into(),
            ));
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if let Ok(ip) = host.parse::<IpAddr>()
            && is_restricted_egress_ip(ip)
            && !ip.is_loopback()
        {
            return Err(IkarosError::Message(format!(
                "network egress denied: restricted IP address {}",
                redact_secrets(&host)
            )));
        }
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
    timeout: Duration,
}

impl HttpNetworkEgress {
    pub fn new(timeout: Duration) -> Result<Self> {
        Ok(Self { timeout })
    }

    fn client_for_target(&self, target: &ResolvedEgressTarget) -> Result<reqwest::Client> {
        reqwest::Client::builder()
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(&target.host, &target.addresses)
            .build()
            .map_err(|source| {
                IkarosError::Message(format!("failed to build network egress client: {source}"))
            })
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
            let method = request.method.clone();
            let url = redact_secrets(&request.url);
            if let Err(error) = self.policy.allows(&request.url) {
                tracing::warn!(
                    event = "harness_network_egress_denied",
                    method = %method,
                    url = %url,
                    error = %redact_secrets(&error.to_string()),
                    "harness network egress denied by policy"
                );
                return Err(error);
            }
            tracing::info!(
                event = "harness_network_egress_allowed",
                method = %method,
                url = %url,
                "harness network egress allowed by policy"
            );
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
            let method = request.method.clone();
            let url = redact_secrets(&request.url);
            let header_count = request.headers.len();
            let request_body_bytes = request
                .body_bytes
                .as_ref()
                .map(Vec::len)
                .or_else(|| request.body.as_ref().map(String::len))
                .unwrap_or(0);
            tracing::info!(
                event = "harness_network_request_started",
                method = %method,
                url = %url,
                header_count,
                request_body_bytes,
                timeout_ms = self.timeout.as_millis() as u64,
                "harness network request started"
            );
            let result: Result<NetworkEgressResponse> = async {
                let target = resolve_egress_target(&request.url).await?;
                let client = self.client_for_target(&target)?;
                let method = request
                    .method
                    .parse::<reqwest::Method>()
                    .map_err(|source| {
                        IkarosError::Message(format!("invalid network egress method: {source}"))
                    })?;
                let mut builder = client.request(method, &request.url);
                for (name, value) in request.headers {
                    builder = builder.header(name, value);
                }
                if let Some(body) = request.body_bytes {
                    builder = builder.body(body);
                } else if let Some(body) = request.body {
                    builder = builder.body(body);
                }
                let response = builder.send().await.map_err(|source| {
                    IkarosError::Message(format!("network egress request failed: {source}"))
                })?;
                let status = response.status().as_u16();
                if let Some(length) = response.content_length()
                    && length > MAX_NETWORK_EGRESS_RESPONSE_BYTES
                {
                    return Err(IkarosError::Message(format!(
                        "network egress response too large: content_length={} max={}",
                        length, MAX_NETWORK_EGRESS_RESPONSE_BYTES
                    )));
                }
                let headers = response
                    .headers()
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.as_str().to_ascii_lowercase(),
                            value.to_str().unwrap_or("<non-utf8>").to_owned(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                let bytes = response.bytes().await.map_err(|source| {
                    IkarosError::Message(format!(
                        "failed to read network egress response: {source}"
                    ))
                })?;
                if bytes.len() as u64 > MAX_NETWORK_EGRESS_RESPONSE_BYTES {
                    return Err(IkarosError::Message(format!(
                        "network egress response too large: bytes={} max={}",
                        bytes.len(),
                        MAX_NETWORK_EGRESS_RESPONSE_BYTES
                    )));
                }
                let body = network_response_body_text(&headers, bytes.as_ref());
                Ok(NetworkEgressResponse {
                    status,
                    headers,
                    body,
                    body_bytes: Some(bytes.to_vec()),
                })
            }
            .await;
            match &result {
                Ok(response) => {
                    tracing::info!(
                        event = "harness_network_request_completed",
                        method = %method,
                        url = %url,
                        status = response.status,
                        response_header_count = response.headers.len(),
                        response_body_bytes = response
                            .body_bytes
                            .as_ref()
                            .map(Vec::len)
                            .unwrap_or_else(|| response.body.len()),
                        "harness network request completed"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        event = "harness_network_request_failed",
                        method = %method,
                        url = %url,
                        error = %redact_secrets(&error.to_string()),
                        "harness network request failed"
                    );
                }
            }
            result
        })
    }
}

fn network_response_body_text(headers: &BTreeMap<String, String>, bytes: &[u8]) -> String {
    let content_type = headers
        .get("content-type")
        .or_else(|| headers.get("Content-Type"))
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let text_like = content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("yaml")
        || content_type.contains("javascript");
    if text_like || std::str::from_utf8(bytes).is_ok() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    format!("<{} bytes binary response>", bytes.len())
}

#[derive(Debug)]
struct ResolvedEgressTarget {
    host: String,
    addresses: Vec<SocketAddr>,
}

async fn resolve_egress_target(url: &str) -> Result<ResolvedEgressTarget> {
    let parsed = Url::parse(url)
        .map_err(|_| IkarosError::Message("network egress denied: invalid URL".into()))?;
    let Some(host) = parsed.host_str() else {
        return Err(IkarosError::Message(
            "network egress denied: URL has no host".into(),
        ));
    };
    let port = parsed.port_or_known_default().ok_or_else(|| {
        IkarosError::Message("network egress denied: URL has no known port".into())
    })?;
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let host_allows_loopback = is_loopback_host(&host);
    let addresses = lookup_host((host.as_str(), port))
        .await
        .map_err(|source| {
            IkarosError::Message(format!(
                "network egress DNS resolution failed for {}: {}",
                redact_secrets(&host),
                source
            ))
        })?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(IkarosError::Message(format!(
            "network egress DNS resolution returned no addresses for {}",
            redact_secrets(&host)
        )));
    }
    for address in &addresses {
        let ip = address.ip();
        if is_restricted_egress_ip(ip) && !(host_allows_loopback && ip.is_loopback()) {
            return Err(IkarosError::Message(format!(
                "network egress denied: host {} resolved to restricted address {}",
                redact_secrets(&host),
                ip
            )));
        }
    }
    Ok(ResolvedEgressTarget { host, addresses })
}

pub(crate) fn is_restricted_egress_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_restricted_ipv4(ip),
        IpAddr::V6(ip) => is_restricted_ipv6(ip),
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn is_restricted_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_loopback()
}

fn is_restricted_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn network_egress_target_allows_explicit_loopback_hosts() {
        let target = resolve_egress_target("http://127.0.0.1:11434/api")
            .await
            .expect("loopback target");
        assert_eq!(target.host, "127.0.0.1");
        assert!(target.addresses.iter().all(|addr| addr.ip().is_loopback()));
    }

    #[tokio::test]
    async fn network_egress_target_rejects_restricted_private_literals() {
        let error = resolve_egress_target("https://10.0.0.1/v1")
            .await
            .expect_err("private address rejected");
        assert!(error.to_string().contains("restricted address"));
    }

    #[tokio::test]
    async fn network_egress_target_preserves_verified_addresses_for_pinning() {
        let target = resolve_egress_target("https://93.184.216.34/")
            .await
            .expect("public address target");
        assert_eq!(target.host, "93.184.216.34");
        assert_eq!(target.addresses.len(), 1);
        assert_eq!(target.addresses[0].ip().to_string(), "93.184.216.34");
    }
}
