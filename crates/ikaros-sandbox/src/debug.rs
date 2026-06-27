// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxDebugReport {
    pub level: SandboxIsolationLevel,
    pub backend: String,
    pub configured_image: Option<String>,
    pub cwd_enforced: bool,
    pub env_allowlist: bool,
    pub timeout_capable: bool,
    pub process_timeout_strategy: String,
    pub output_capable: bool,
    pub file_write_scope: String,
    pub network_egress: String,
    pub allow_provider_hosts: bool,
    pub configured_allowed_host_count: usize,
    pub effective_allowed_host_count: usize,
    pub host_allowlist_mode: String,
    pub restricted_ip_literal_block: bool,
    pub dns_rebind_block: bool,
    pub loopback_exception: String,
    pub process_network_isolation: String,
    pub workspace_mount: Option<String>,
    pub plugin_mount: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxIsolationLevel {
    NoOp,
    DryRun,
    WorkspaceOnly,
    NetworkRestricted,
    Container,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxIsolationMatrixEntry {
    pub level: SandboxIsolationLevel,
    pub status: SandboxIsolationStatus,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxIsolationStatus {
    Available,
    Planned,
    Unsupported,
}

pub fn sandbox_isolation_matrix() -> Vec<SandboxIsolationMatrixEntry> {
    vec![
        SandboxIsolationMatrixEntry {
            level: SandboxIsolationLevel::NoOp,
            status: SandboxIsolationStatus::Unsupported,
            summary: "raw host execution is not exposed as a runtime backend".into(),
        },
        SandboxIsolationMatrixEntry {
            level: SandboxIsolationLevel::DryRun,
            status: SandboxIsolationStatus::Available,
            summary: "skips file/process/network side effects while preserving reports".into(),
        },
        SandboxIsolationMatrixEntry {
            level: SandboxIsolationLevel::WorkspaceOnly,
            status: SandboxIsolationStatus::Available,
            summary: "enforces workspace file scope, process cwd scope, env allowlist, timeout, and output caps".into(),
        },
        SandboxIsolationMatrixEntry {
            level: SandboxIsolationLevel::NetworkRestricted,
            status: SandboxIsolationStatus::Available,
            summary: "routes network egress through governed allowlist or deny-by-default policy".into(),
        },
        SandboxIsolationMatrixEntry {
            level: SandboxIsolationLevel::Container,
            status: SandboxIsolationStatus::Available,
            summary: "runs process execution through a Docker container backend when configured".into(),
        },
    ]
}

pub fn local_sandbox_debug_report(
    backend: impl AsRef<str>,
    network_enabled: bool,
    configured_image: Option<&str>,
) -> SandboxDebugReport {
    let backend = backend.as_ref().to_ascii_lowercase();
    let level = match backend.as_str() {
        "dry-run" => SandboxIsolationLevel::DryRun,
        "docker" => SandboxIsolationLevel::Container,
        "local" if network_enabled => SandboxIsolationLevel::NetworkRestricted,
        "local" => SandboxIsolationLevel::WorkspaceOnly,
        _ => SandboxIsolationLevel::NoOp,
    };
    let network_egress = if network_enabled {
        "governed"
    } else {
        "deny_by_default"
    };
    let mut notes = vec![
        "process cwd is constrained before spawn".into(),
        "process environment is cleared and rebuilt from a small allowlist plus explicit request env".into(),
        "process timeout and output caps kill the spawned process group on Unix".into(),
        "file APIs reject workspace escapes, symlink write escapes, and final-path symlink swaps on Unix".into(),
    ];
    if !network_enabled {
        notes.push("network egress transport is deny-by-default".into());
    }
    if backend == "dry-run" {
        notes.push("dry-run backend skips process and file side effects".into());
    }
    if backend == "docker" {
        notes.push("process execution runs through docker run with the workspace bind-mounted at /workspace".into());
        notes.push("command-backed plugin execution mounts plugin roots at /plugin when the request uses plugin cwd scope".into());
        notes.push("container process network is disabled with --network none".into());
    }
    let configured_image = (backend == "docker").then(|| {
        configured_image
            .map(str::trim)
            .filter(|image| !image.is_empty())
            .unwrap_or("unknown")
            .to_string()
    });
    SandboxDebugReport {
        level,
        backend,
        configured_image,
        cwd_enforced: true,
        env_allowlist: true,
        timeout_capable: true,
        process_timeout_strategy: process_timeout_strategy().into(),
        output_capable: true,
        file_write_scope: "workspace_only".into(),
        network_egress: network_egress.into(),
        allow_provider_hosts: false,
        configured_allowed_host_count: 0,
        effective_allowed_host_count: 0,
        host_allowlist_mode: if network_enabled {
            "configured_hosts_only".into()
        } else {
            "deny_by_default".into()
        },
        restricted_ip_literal_block: true,
        dns_rebind_block: true,
        loopback_exception: "explicit_loopback_hosts_only".into(),
        process_network_isolation: if level == SandboxIsolationLevel::Container {
            "docker_network_none".into()
        } else {
            "not_enforced_without_container_backend".into()
        },
        workspace_mount: (level == SandboxIsolationLevel::Container).then(|| "/workspace".into()),
        plugin_mount: (level == SandboxIsolationLevel::Container).then(|| "/plugin".into()),
        notes,
    }
}

#[cfg(unix)]
fn process_timeout_strategy() -> &'static str {
    "process_group_unix"
}

#[cfg(not(unix))]
fn process_timeout_strategy() -> &'static str {
    "direct_child_kill"
}
