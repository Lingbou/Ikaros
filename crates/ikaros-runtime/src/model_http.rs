// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::Result;
use ikaros_harness::{NetworkEgress, NetworkEgressRequest};
use ikaros_models::{ModelHttpClient, ModelHttpRequest, ModelHttpResponse};
use std::{future::Future, pin::Pin, sync::Arc};

#[derive(Clone)]
pub struct EgressModelHttpClient {
    egress: Arc<dyn NetworkEgress>,
}

impl EgressModelHttpClient {
    pub fn new(egress: Arc<dyn NetworkEgress>) -> Self {
        Self { egress }
    }
}

impl ModelHttpClient for EgressModelHttpClient {
    fn send<'a>(
        &'a self,
        request: ModelHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ModelHttpResponse>> + Send + 'a>> {
        Box::pin(async move {
            let response = self
                .egress
                .send_network_request(NetworkEgressRequest {
                    method: request.method,
                    url: request.url,
                    headers: request.headers,
                    body: Some(request.body),
                    body_bytes: None,
                })
                .await?;
            Ok(ModelHttpResponse {
                status: response.status,
                headers: response.headers,
                body: response.body,
            })
        })
    }
}
