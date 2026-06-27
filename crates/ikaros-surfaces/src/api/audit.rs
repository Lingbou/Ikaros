// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::api) fn audit_api_request(
    paths: &IkarosPaths,
    peer_addr: Option<SocketAddr>,
    method: &str,
    route: &str,
    response: &ApiHttpResponse,
    headers: Option<&ApiHeaders>,
) {
    let audit = AuditLog::new(&paths.audit_dir);
    let event = match AuditEvent::new(
        "api_request",
        None,
        format!("api {method} {route} -> {}", response.status_code),
        json!({
            "method": method,
            "route": route,
            "status_code": response.status_code,
            "reason": response.reason,
            "peer_addr": peer_addr.map(|addr| addr.to_string()),
            "authorization_present": headers.and_then(|headers| headers.authorization.as_ref()).is_some(),
            "client_id": headers.and_then(|headers| headers.client_id.as_deref()).map(api_header_preview),
            "session_id": response.session.as_ref().map(|session| &session.session_id),
            "turn_id": response.session.as_ref().map(|session| &session.turn_id),
            "correlation_id": response.session.as_ref().map(ApiSessionIds::correlation_id),
        }),
    ) {
        Ok(event) => event,
        Err(error) => {
            eprintln!("failed to build API audit event: {error}");
            return;
        }
    };
    if let Err(error) = audit.append(event) {
        eprintln!("failed to append API audit event: {error}");
    }
    append_api_request_trace(paths, peer_addr, method, route, response, headers);
}

pub(in crate::api) fn append_api_request_trace(
    paths: &IkarosPaths,
    peer_addr: Option<SocketAddr>,
    method: &str,
    route: &str,
    response: &ApiHttpResponse,
    headers: Option<&ApiHeaders>,
) {
    let mut event = match StructuredTraceEvent::new(
        if response.status_code >= 500 {
            "ERROR"
        } else if response.status_code >= 400 {
            "WARN"
        } else {
            "INFO"
        },
        "ikaros_cli::api",
        "api_request",
        format!("api {method} {route} -> {}", response.status_code),
        json!({
            "method": method,
            "route": route,
            "status_code": response.status_code,
            "reason": response.reason,
            "peer_addr": peer_addr.map(|addr| addr.to_string()),
            "authorization_present": headers.and_then(|headers| headers.authorization.as_ref()).is_some(),
            "client_id": headers.and_then(|headers| headers.client_id.as_deref()).map(api_header_preview),
            "content_type": headers.and_then(|headers| headers.content_type.as_deref()).map(api_header_preview),
            "session_id": response.session.as_ref().map(|session| &session.session_id),
            "turn_id": response.session.as_ref().map(|session| &session.turn_id),
            "correlation_id": response.session.as_ref().map(ApiSessionIds::correlation_id),
        }),
    ) {
        Ok(event) => event,
        Err(error) => {
            eprintln!("failed to build API trace event: {error}");
            return;
        }
    };
    if let Some(session) = &response.session {
        event = event
            .with_session_turn(session.session_id.clone(), session.turn_id.clone())
            .with_correlation_id(session.correlation_id());
    }
    if let Err(error) = StructuredTraceLog::new(&paths.logs_dir).append(event) {
        eprintln!("failed to append API trace event: {error}");
    }
}

pub(in crate::api) fn api_header_preview(value: &str) -> String {
    let redacted = redact_secrets(value);
    let mut preview = redacted.chars().take(128).collect::<String>();
    if redacted.chars().count() > 128 {
        preview.push_str("...");
    }
    preview.replace(['\r', '\n', '\t'], " ")
}
