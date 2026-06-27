// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::api) fn api_http_response(
    method: &str,
    route: &str,
    body: &[u8],
    headers: &ApiHeaders,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ApiHttpResponse> {
    match (method, route) {
        ("GET" | "HEAD", "/healthz" | "/health") => {
            ApiHttpResponse::json(200, "OK", json!({"status": "ok"}))
        }
        ("GET" | "HEAD", "/ready") => ready_response(paths, workspace, agent_override),
        ("GET" | "HEAD", "/v1/models") => models_response(paths, workspace, agent_override),
        ("GET" | "HEAD", "/v1/ikaros/protocol") => protocol_response(),
        ("POST", "/v1/chat/completions") => {
            chat_completion_response(body, paths, workspace, agent_override)
        }
        ("POST", "/v1/responses") => responses_response(body, paths, workspace, agent_override),
        ("POST", "/v1/embeddings") => embedding_response(body, paths, workspace, agent_override),
        ("POST", "/v1/images/generations") => {
            image_generation_response(body, paths, workspace, agent_override)
        }
        ("POST", "/v1/audio/speech") => {
            audio_speech_response(body, paths, workspace, agent_override)
        }
        ("POST", "/v1/audio/transcriptions") => {
            audio_transcription_response(body, headers, paths, workspace, agent_override)
        }
        ("GET" | "HEAD", "/v1/chat/completions") => Ok(ApiHttpResponse::method_not_allowed("POST")),
        ("GET" | "HEAD", "/v1/responses") => Ok(ApiHttpResponse::method_not_allowed("POST")),
        ("GET" | "HEAD", "/v1/embeddings") => Ok(ApiHttpResponse::method_not_allowed("POST")),
        ("GET" | "HEAD", "/v1/images/generations") => {
            Ok(ApiHttpResponse::method_not_allowed("POST"))
        }
        ("GET" | "HEAD", "/v1/audio/speech" | "/v1/audio/transcriptions") => {
            Ok(ApiHttpResponse::method_not_allowed("POST"))
        }
        ("POST", _) => Ok(ApiHttpResponse::json_error(404, "Not Found", "not found")),
        (_, "/healthz" | "/health" | "/ready" | "/v1/models" | "/v1/ikaros/protocol") => {
            Ok(ApiHttpResponse::method_not_allowed("GET, HEAD"))
        }
        _ => Ok(ApiHttpResponse::json_error(404, "Not Found", "not found")),
    }
}
