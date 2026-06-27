// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::Result;
use ikaros_execution::harness::{ExecutionSession, NetworkEgressRequest};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct BrowserCdpHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
    pub body_bytes: Option<Vec<u8>>,
}

pub async fn send_browser_cdp_http_request(
    session: &ExecutionSession,
    method: &str,
    url: &str,
) -> Result<BrowserCdpHttpResponse> {
    let response = session
        .env
        .send_network_request(NetworkEgressRequest {
            method: method.into(),
            url: url.into(),
            headers: BTreeMap::new(),
            body: None,
            body_bytes: None,
        })
        .await?;
    Ok(BrowserCdpHttpResponse {
        status: response.status,
        headers: response.headers,
        body: response.body,
        body_bytes: response.body_bytes,
    })
}
