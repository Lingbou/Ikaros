// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::api) struct ApiHttpHeader {
    pub(in crate::api) name: &'static str,
    pub(in crate::api) value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::api) struct ApiHttpResponse {
    pub(in crate::api) status_code: u16,
    pub(in crate::api) reason: &'static str,
    pub(in crate::api) content_type: &'static str,
    pub(in crate::api) body: String,
    pub(in crate::api) body_bytes: Option<Vec<u8>>,
    pub(in crate::api) allow: Option<&'static str>,
    pub(in crate::api) extra_headers: Vec<ApiHttpHeader>,
    pub(in crate::api) session: Option<ApiSessionIds>,
}

impl ApiHttpResponse {
    pub(in crate::api) fn json(
        status_code: u16,
        reason: &'static str,
        body: Value,
    ) -> Result<Self> {
        Ok(Self {
            status_code,
            reason,
            content_type: "application/json; charset=utf-8",
            body: serde_json::to_string(&body)?,
            body_bytes: None,
            allow: None,
            extra_headers: Vec::new(),
            session: None,
        })
    }

    pub(in crate::api) fn json_error(
        status_code: u16,
        reason: &'static str,
        message: impl AsRef<str>,
    ) -> Self {
        let body = json!({
            "error": {
                "message": redact_secrets(message.as_ref()),
                "type": "ikaros_api_error",
                "code": status_code,
            }
        });
        Self {
            status_code,
            reason,
            content_type: "application/json; charset=utf-8",
            body: serde_json::to_string(&body).unwrap_or_else(|_| {
                "{\"error\":{\"message\":\"api error\",\"type\":\"ikaros_api_error\"}}".into()
            }),
            body_bytes: None,
            allow: None,
            extra_headers: Vec::new(),
            session: None,
        }
    }

    pub(in crate::api) fn method_not_allowed(allow: &'static str) -> Self {
        let mut response = Self::json_error(405, "Method Not Allowed", "method not allowed");
        response.allow = Some(allow);
        response
    }

    pub(in crate::api) fn unauthorized() -> Self {
        let mut response = Self::json_error(
            401,
            "Unauthorized",
            "missing or invalid bearer token for Ikaros API route",
        );
        response.extra_headers.push(ApiHttpHeader {
            name: "WWW-Authenticate",
            value: "Bearer realm=\"ikaros\"".to_owned(),
        });
        response
    }

    pub(in crate::api) fn rate_limited(retry_after_seconds: u64) -> Self {
        let mut response = Self::json_error(429, "Too Many Requests", "rate limit exceeded");
        response.extra_headers.push(ApiHttpHeader {
            name: "Retry-After",
            value: retry_after_seconds.to_string(),
        });
        response
    }

    pub(in crate::api) fn internal_error(error: anyhow::Error) -> Self {
        Self::json_error(
            500,
            "Internal Server Error",
            format!("api request failed: {error:#}"),
        )
    }

    pub(in crate::api) fn event_stream(body: String) -> Self {
        Self {
            status_code: 200,
            reason: "OK",
            content_type: "text/event-stream; charset=utf-8",
            body,
            body_bytes: None,
            allow: None,
            extra_headers: Vec::new(),
            session: None,
        }
    }

    pub(in crate::api) fn binary(
        status_code: u16,
        reason: &'static str,
        content_type: &'static str,
        body: Vec<u8>,
    ) -> Self {
        Self {
            status_code,
            reason,
            content_type,
            body: String::new(),
            body_bytes: Some(body),
            allow: None,
            extra_headers: Vec::new(),
            session: None,
        }
    }

    pub(in crate::api) fn with_session(mut self, session: ApiSessionIds) -> Self {
        self.session = Some(session);
        self
    }

    pub(in crate::api) fn observability_headers(&self) -> Vec<ApiHttpHeader> {
        let Some(session) = &self.session else {
            return Vec::new();
        };
        vec![
            ApiHttpHeader {
                name: "X-Ikaros-Session-Id",
                value: session.session_id.clone(),
            },
            ApiHttpHeader {
                name: "X-Ikaros-Turn-Id",
                value: session.turn_id.clone(),
            },
            ApiHttpHeader {
                name: "X-Ikaros-Correlation-Id",
                value: session.correlation_id(),
            },
        ]
    }

    pub(in crate::api) fn wire_body(&self) -> &[u8] {
        self.body_bytes.as_deref().unwrap_or(self.body.as_bytes())
    }
}
pub(in crate::api) fn write_api_http_response(
    stream: &mut TcpStream,
    response: &ApiHttpResponse,
    send_body: bool,
) -> Result<()> {
    let body = response.wire_body();
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        response.status_code,
        response.reason,
        response.content_type,
        body.len()
    )
    .with_context(|| "failed to write API response headers")?;
    if let Some(allow) = response.allow {
        write!(stream, "Allow: {allow}\r\n").with_context(|| "failed to write API Allow header")?;
    }
    let observability_headers = response.observability_headers();
    for header in response
        .extra_headers
        .iter()
        .chain(observability_headers.iter())
    {
        write!(
            stream,
            "{}: {}\r\n",
            header.name,
            safe_header_value(&header.value)
        )
        .with_context(|| format!("failed to write API {} header", header.name))?;
    }
    write!(stream, "\r\n").with_context(|| "failed to finish API response headers")?;
    if send_body {
        stream
            .write_all(body)
            .with_context(|| "failed to write API response body")?;
    }
    stream
        .flush()
        .with_context(|| "failed to flush API response")
}

pub(in crate::api) fn safe_header_value(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}
