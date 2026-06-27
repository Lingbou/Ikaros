// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{RagConfig, RemoteProviderConfig, RiskLevel, redact_secrets};
use ikaros_rag::embedding_provider_uses_network;
use ikaros_toolkit::PolicyRequest;
use serde_json::json;
use std::path::{Path, PathBuf};

pub(super) fn rag_risk_level(config: &RagConfig, writes_index: bool) -> RiskLevel {
    if embedding_provider_uses_network(&config.embedding_provider) {
        RiskLevel::Network
    } else if writes_index {
        RiskLevel::LocalWrite
    } else {
        RiskLevel::SafeRead
    }
}

pub(super) fn rag_path_policy_request(
    action: &str,
    risk: RiskLevel,
    input: &serde_json::Value,
    workspace_root: &Path,
    writes_index: bool,
) -> PolicyRequest {
    PolicyRequest {
        action: action.into(),
        risk,
        path: input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| resolve_policy_path(path, workspace_root)),
        command: None,
        is_write: writes_index,
    }
}

pub(super) fn rag_approval_context(
    action: &str,
    config: &RagConfig,
    provider_settings: &RemoteProviderConfig,
    input: &serde_json::Value,
    workspace_root: &Path,
    local_file_read: bool,
    writes_index: bool,
) -> serde_json::Value {
    let path = input
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(|path| resolve_policy_path(path, workspace_root));
    let rag_scope = input
        .get("scope")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("project");
    json!({
        "action": action,
        "operations": {
            "provider_call": embedding_provider_uses_network(&config.embedding_provider),
            "local_file_read": local_file_read,
            "rag_index_write": writes_index,
        },
        "provider": {
            "embedding_provider": &config.embedding_provider,
            "embedding_model": &config.embedding_model,
            "base_url_configured": !provider_settings.base_url.trim().is_empty(),
            "base_url": redact_secrets(provider_settings.base_url.trim()),
        },
        "scope": {
            "path": path,
            "rag_scope": rag_scope,
        },
    })
}

fn resolve_policy_path(path: &str, workspace_root: &Path) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}
