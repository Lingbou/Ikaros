// SPDX-License-Identifier: GPL-3.0-only

use futures_util::{Stream, StreamExt, stream};
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
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

pub struct ModelHttpStreamResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Pin<Box<dyn Stream<Item = Result<Vec<u8>>> + Send>>,
}

pub trait ModelHttpClient: Send + Sync {
    fn send<'a>(
        &'a self,
        request: ModelHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ModelHttpResponse>> + Send + 'a>>;

    fn send_stream<'a>(
        &'a self,
        request: ModelHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ModelHttpStreamResponse>> + Send + 'a>> {
        Box::pin(async move {
            let response = self.send(request).await?;
            Ok(ModelHttpStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(stream::once(async move { Ok(response.body.into_bytes()) })),
            })
        })
    }
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
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
                })
                .collect();
            let body = response.text().await.map_err(|source| {
                IkarosError::Message(format!("failed to read model response: {source}"))
            })?;
            Ok(ModelHttpResponse {
                status,
                headers,
                body,
            })
        })
    }

    fn send_stream<'a>(
        &'a self,
        request: ModelHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ModelHttpStreamResponse>> + Send + 'a>> {
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
                IkarosError::Message(format!("model stream request failed: {source}"))
            })?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
                })
                .collect();
            let body = response.bytes_stream().map(|chunk| {
                chunk.map(|bytes| bytes.to_vec()).map_err(|source| {
                    IkarosError::Message(format!("failed to read model stream response: {source}"))
                })
            });
            Ok(ModelHttpStreamResponse {
                status,
                headers,
                body: Box::pin(body),
            })
        })
    }
}
