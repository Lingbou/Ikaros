// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{RagConfig, RiskLevel};
use ikaros_harness::PolicyRequest;
use ikaros_rag::embedding_provider_uses_network;
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

fn resolve_policy_path(path: &str, workspace_root: &Path) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}
