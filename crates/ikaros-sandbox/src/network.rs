// SPDX-License-Identifier: GPL-3.0-only

use super::*;

impl NetworkEgress for LocalExecutionEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        Box::pin(async move {
            tracing::warn!(
                event = "harness_network_egress_unconfigured",
                method = %request.method,
                url = %redact_secrets(&request.url),
                "harness network egress requested without backend"
            );
            Err(IkarosError::Message(format!(
                "no network backend is configured for {} {}",
                request.method, request.url
            )))
        })
    }
}
