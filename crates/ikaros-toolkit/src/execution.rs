// SPDX-License-Identifier: GPL-3.0-only

use crate::{Skill, SkillContext, SkillOutput};
use ikaros_core::{Result, redact_json, redact_secrets};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileMetadata {
    pub is_file: bool,
    pub is_dir: bool,
    pub is_symlink: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessRequest {
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub use_shell: bool,
    #[serde(default, skip_serializing_if = "ProcessCwdScope::is_workspace")]
    pub cwd_scope: ProcessCwdScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl ProcessRequest {
    pub fn shell(command: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            use_shell: true,
            cwd_scope: ProcessCwdScope::Workspace,
            stdin: None,
            timeout_ms: None,
            max_output_bytes: None,
            env: BTreeMap::new(),
        }
    }

    pub fn program(program: impl Into<String>, args: Vec<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command: program.into(),
            args,
            cwd: cwd.into(),
            use_shell: false,
            cwd_scope: ProcessCwdScope::Workspace,
            stdin: None,
            timeout_ms: None,
            max_output_bytes: None,
            env: BTreeMap::new(),
        }
    }

    pub fn with_stdin(mut self, stdin: impl Into<String>) -> Self {
        self.stdin = Some(stdin.into());
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    pub fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = Some(max_output_bytes);
        self
    }

    pub fn with_env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(name.into(), value.into());
        self
    }

    pub fn with_plugin_cwd_scope(mut self) -> Self {
        self.cwd_scope = ProcessCwdScope::Plugin;
        self
    }
}

impl std::fmt::Debug for ProcessRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessRequest")
            .field("command", &redact_secrets(&self.command))
            .field("args", &redacted_process_args(&self.args))
            .field("cwd", &self.cwd)
            .field("use_shell", &self.use_shell)
            .field("cwd_scope", &self.cwd_scope)
            .field("stdin", &self.stdin.as_deref().map(redact_secrets))
            .field("timeout_ms", &self.timeout_ms)
            .field("max_output_bytes", &self.max_output_bytes)
            .field("env", &redacted_process_env(&self.env))
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcessCwdScope {
    #[default]
    Workspace,
    Plugin,
}

impl ProcessCwdScope {
    fn is_workspace(&self) -> bool {
        matches!(self, Self::Workspace)
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkEgressRequest {
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_bytes: Option<Vec<u8>>,
}

impl std::fmt::Debug for NetworkEgressRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkEgressRequest")
            .field("method", &self.method)
            .field("url", &redact_secrets(&self.url))
            .field("headers", &redacted_network_headers(&self.headers))
            .field("body", &self.body.as_deref().map(redacted_network_body))
            .field(
                "body_bytes",
                &self
                    .body_bytes
                    .as_ref()
                    .map(|body| redacted_network_body_bytes(body.as_slice())),
            )
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkEgressResponse {
    pub status: u16,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_bytes: Option<Vec<u8>>,
}

impl std::fmt::Debug for NetworkEgressResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkEgressResponse")
            .field("status", &self.status)
            .field("headers", &redacted_network_headers(&self.headers))
            .field("body", &redacted_network_body(&self.body))
            .field(
                "body_bytes",
                &self
                    .body_bytes
                    .as_ref()
                    .map(|body| redacted_network_body_bytes(body.as_slice())),
            )
            .finish()
    }
}

pub trait FileSystem: Send + Sync {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>>;

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>>;

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>>;

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

pub trait ProcessRunner: Send + Sync {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>>;
}

pub trait NetworkEgress: Send + Sync {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>>;
}

pub trait ExecutionEnv: FileSystem + ProcessRunner + NetworkEgress + Send + Sync {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>>;
}

pub fn redacted_process_args(args: &[String]) -> Vec<String> {
    args.iter().map(|arg| redact_secrets(arg)).collect()
}

pub fn redacted_process_env(env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    env.iter()
        .map(|(name, value)| {
            let redacted = if sensitive_env_name(name) {
                "[REDACTED_SECRET]".into()
            } else {
                redact_secrets(value)
            };
            (name.clone(), redacted)
        })
        .collect()
}

pub fn sensitive_env_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("key")
        || name.contains("token")
        || name.contains("secret")
        || name.contains("password")
        || name.contains("credential")
}

pub fn redacted_network_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let redacted = if sensitive_network_header(name) {
                "[REDACTED_SECRET]".into()
            } else {
                redact_secrets(value)
            };
            (name.clone(), redacted)
        })
        .collect()
}

pub fn sensitive_network_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "x-api-key" | "api-key" | "cookie" | "set-cookie"
    )
}

pub fn redacted_network_body(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        return redact_json(value).to_string();
    }
    redact_secrets(body)
}

pub fn redacted_network_body_bytes(body: &[u8]) -> String {
    format!("{} bytes", body.len())
}
