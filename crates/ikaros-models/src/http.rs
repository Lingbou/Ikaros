// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use std::{collections::BTreeMap, future::Future, pin::Pin, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHttpResponse {
    pub status: u16,
    pub body: String,
}

pub trait ModelHttpClient: Send + Sync {
    fn send<'a>(
        &'a self,
        request: ModelHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ModelHttpResponse>> + Send + 'a>>;
}

#[derive(Clone)]
pub struct ReqwestModelHttpClient {
    client: reqwest::Client,
}

impl ReqwestModelHttpClient {
    pub fn new(timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|source| {
                IkarosError::Message(format!("failed to build model HTTP client: {source}"))
            })?;
        Ok(Self { client })
    }
}

impl ModelHttpClient for ReqwestModelHttpClient {
    fn send<'a>(
        &'a self,
        request: ModelHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ModelHttpResponse>> + Send + 'a>> {
        Box::pin(async move {
            let method = request
                .method
                .parse::<reqwest::Method>()
                .map_err(|source| IkarosError::Message(format!("invalid HTTP method: {source}")))?;
            let mut builder = self.client.request(method, &request.url);
            for (name, value) in request.headers {
                builder = builder.header(name, value);
            }
            let response = builder.body(request.body).send().await.map_err(|source| {
                IkarosError::Message(format!("model request failed: {source}"))
            })?;
            let status = response.status().as_u16();
            let body = response.text().await.map_err(|source| {
                IkarosError::Message(format!("failed to read model response: {source}"))
            })?;
            Ok(ModelHttpResponse { status, body })
        })
    }
}
