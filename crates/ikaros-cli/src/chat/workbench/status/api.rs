// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::IkarosConfig;

use super::super::{WorkbenchCell, WorkbenchCellKind, terminal_inline};

const DEFAULT_API_HOST: &str = "127.0.0.1";
const DEFAULT_API_PORT: u16 = 8003;
const DEFAULT_RATE_LIMIT_PER_MINUTE: u32 = 120;

pub(in crate::chat) fn print_api_status(config: &IkarosConfig) {
    println!("api_surface: openai-compatible");
    println!("api_bind_policy: loopback_only");
    println!("api_default_host: {DEFAULT_API_HOST}");
    println!("api_default_port: {DEFAULT_API_PORT}");
    println!("api_auth: optional_bearer_token");
    println!("api_rate_limit_per_minute_default: {DEFAULT_RATE_LIMIT_PER_MINUTE}");
    println!(
        "api_model_provider: {}",
        terminal_inline(&config.model.default.provider)
    );
    println!(
        "api_model_id: {}",
        terminal_inline(&config.model.default.model)
    );
    println!(
        "api_embedding_provider: {}",
        terminal_inline(&config.rag.embedding_provider)
    );
    println!(
        "api_embedding_model: {}",
        terminal_inline(&config.rag.embedding_model)
    );
    println!(
        "api_endpoints: /v1/models,/v1/chat/completions,/v1/responses,/v1/embeddings,/v1/images/generations,/v1/audio/speech,/v1/audio/transcriptions,/v1/ikaros/protocol"
    );
    println!("{}", api_status_json_line(config));
}

pub(in crate::chat) fn print_api_status_for_human(config: &IkarosConfig) {
    for line in api_status_human_lines(config) {
        println!("{line}");
    }
}

pub(in crate::chat) fn api_status_human_lines(config: &IkarosConfig) -> Vec<String> {
    vec![
        "• API".to_owned(),
        "  surface: OpenAI-compatible loopback".to_owned(),
        format!("  bind: {DEFAULT_API_HOST}:{DEFAULT_API_PORT}"),
        "  auth: optional bearer token".to_owned(),
        format!(
            "  model: {} ({})",
            terminal_inline(&config.model.default.model),
            terminal_inline(&config.model.default.provider)
        ),
        format!(
            "  embedding: {} ({})",
            terminal_inline(&config.rag.embedding_model),
            terminal_inline(&config.rag.embedding_provider)
        ),
        "  endpoints: models, chat, responses, embeddings, images, audio".to_owned(),
    ]
}

pub(super) fn screen_api_cell(config: &IkarosConfig) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "api".into(),
        detail: format!(
            "surface=openai-compatible bind=loopback endpoints=chat,responses,embeddings,images,audio,protocol model={} embedding={} command=/api status",
            terminal_inline(&config.model.default.model),
            terminal_inline(&config.rag.embedding_model),
        ),
    }
}

fn api_status_json_line(config: &IkarosConfig) -> String {
    serde_json::to_string(&serde_json::json!({
        "schema": "ikaros-workbench-api-status-v1",
        "version": 1,
        "surface": "openai-compatible",
        "bind_policy": "loopback_only",
        "default_host": DEFAULT_API_HOST,
        "default_port": DEFAULT_API_PORT,
        "auth": {
            "bearer_token": "optional",
            "health_routes_open": true,
        },
        "rate_limit": {
            "per_process": true,
            "default_per_minute": DEFAULT_RATE_LIMIT_PER_MINUTE,
        },
        "endpoints": [
            {"method": "GET", "path": "/v1/models", "capabilities": ["chat.completions", "responses", "embeddings", "images.generations", "audio.speech", "audio.transcriptions"]},
            {"method": "GET", "path": "/v1/ikaros/protocol", "capabilities": ["ikaros.protocol"]},
            {"method": "POST", "path": "/v1/chat/completions", "session_evidence": true},
            {"method": "POST", "path": "/v1/responses", "session_evidence": true},
            {"method": "POST", "path": "/v1/embeddings", "session_evidence": true},
            {"method": "POST", "path": "/v1/images/generations", "session_evidence": true},
            {"method": "POST", "path": "/v1/audio/speech", "session_evidence": true},
            {"method": "POST", "path": "/v1/audio/transcriptions", "session_evidence": true},
            {"method": "GET", "path": "/healthz", "session_evidence": false},
            {"method": "GET", "path": "/health", "session_evidence": false},
            {"method": "GET", "path": "/ready", "session_evidence": false},
        ],
        "model": {
            "provider": terminal_inline(&config.model.default.provider),
            "model": terminal_inline(&config.model.default.model),
        },
        "embedding": {
            "provider": terminal_inline(&config.rag.embedding_provider),
            "model": terminal_inline(&config.rag.embedding_model),
        },
        "network_egress": {
            "model": true,
            "remote_embedding": true,
        },
        "limitations": [
            "local loopback API server only",
            "no persistent API credential lifecycle",
            "no server-side execution of API-supplied tools",
            "responses endpoint is a first API slice",
        ],
    }))
    .unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-api-status-v1","version":1,"error":"serialization_failed"}"#
            .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_status_json_lists_responses_without_secret_material() {
        let mut config = IkarosConfig::default();
        config.model.default.model = "mock-ikaros".into();
        config.providers.model.api_key = "sk-secret-value".into();

        let line = api_status_json_line(&config);
        assert!(line.contains("/v1/responses"));
        assert!(line.contains("chat.completions"));
        assert!(!line.contains("sk-secret-value"));
    }
}
