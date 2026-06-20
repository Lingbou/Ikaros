// SPDX-License-Identifier: GPL-3.0-only

use crate::{Skill, SkillContext, SkillOutput, session::ExecutionSession};
use ikaros_core::{IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    path::Component,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    time,
};

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
        session: &'a ExecutionSession,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LocalExecutionEnv;

#[derive(Clone)]
pub struct WorkspaceExecutionEnv {
    workspace_root: PathBuf,
    inner: Arc<dyn ExecutionEnv>,
}

#[derive(Clone)]
pub struct NetworkedExecutionEnv {
    inner: Arc<dyn ExecutionEnv>,
    network: Arc<dyn NetworkEgress>,
}

#[derive(Clone)]
pub struct DryRunExecutionEnv {
    inner: Arc<dyn ExecutionEnv>,
}

impl DryRunExecutionEnv {
    pub fn new(inner: Arc<dyn ExecutionEnv>) -> Self {
        Self { inner }
    }
}

impl NetworkedExecutionEnv {
    pub fn new(inner: Arc<dyn ExecutionEnv>, network: Arc<dyn NetworkEgress>) -> Self {
        Self { inner, network }
    }
}

impl WorkspaceExecutionEnv {
    pub fn new(workspace_root: impl Into<PathBuf>, inner: Arc<dyn ExecutionEnv>) -> Self {
        Self {
            workspace_root: normalize_path(&absolute_path(workspace_root.into())),
            inner,
        }
    }

    pub fn local(workspace_root: impl Into<PathBuf>) -> Self {
        Self::new(workspace_root, Arc::new(LocalExecutionEnv))
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    fn resolve_path(&self, path: &Path) -> PathBuf {
        normalize_path(&if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        })
    }

    fn ensure_lexically_in_workspace(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.resolve_path(path);
        if resolved.starts_with(&self.workspace_root) {
            return Ok(resolved);
        }
        #[cfg(windows)]
        {
            let canonical_workspace = fs::canonicalize(&self.workspace_root)
                .unwrap_or_else(|_| self.workspace_root.clone());
            if let Ok(canonical) = fs::canonicalize(&resolved)
                && canonical.starts_with(&canonical_workspace)
            {
                return Ok(resolved);
            }
        }
        Err(IkarosError::OutOfScope(resolved))
    }

    fn ensure_write_path(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.ensure_lexically_in_workspace(path)?;
        if let Ok(canonical) = fs::canonicalize(&resolved) {
            self.ensure_canonical_in_workspace(&resolved, &canonical)?;
            return Ok(resolved);
        }
        let parent = resolved
            .parent()
            .ok_or_else(|| IkarosError::OutOfScope(resolved.clone()))?;
        self.ensure_existing_anchor_in_workspace(parent)?;
        Ok(resolved)
    }

    fn ensure_create_dir_path(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.ensure_lexically_in_workspace(path)?;
        if let Ok(canonical) = fs::canonicalize(&resolved) {
            self.ensure_canonical_in_workspace(&resolved, &canonical)?;
            return Ok(resolved);
        }
        self.ensure_existing_anchor_in_workspace(&resolved)?;
        Ok(resolved)
    }

    fn ensure_existing_workspace_path(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.ensure_lexically_in_workspace(path)?;
        let canonical =
            fs::canonicalize(&resolved).map_err(|source| IkarosError::io(&resolved, source))?;
        self.ensure_canonical_in_workspace(&resolved, &canonical)?;
        Ok(resolved)
    }

    fn ensure_existing_anchor_in_workspace(&self, path: &Path) -> Result<()> {
        if let Ok(canonical) = fs::canonicalize(path) {
            self.ensure_canonical_in_workspace(path, &canonical)?;
            return Ok(());
        }
        let mut ancestor = path;
        while let Some(parent) = ancestor.parent() {
            if parent == self.workspace_root {
                return Ok(());
            }
            if parent.starts_with(&self.workspace_root)
                && let Ok(canonical) = fs::canonicalize(parent)
            {
                self.ensure_canonical_in_workspace(parent, &canonical)?;
                return Ok(());
            }
            ancestor = parent;
        }
        Err(IkarosError::OutOfScope(path.to_path_buf()))
    }

    fn ensure_canonical_in_workspace(&self, requested: &Path, canonical: &Path) -> Result<()> {
        let canonical_workspace =
            fs::canonicalize(&self.workspace_root).unwrap_or_else(|_| self.workspace_root.clone());
        if !canonical.starts_with(&canonical_workspace) {
            return Err(IkarosError::OutOfScope(requested.to_path_buf()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileMetadata {
    pub is_file: bool,
    pub is_dir: bool,
    pub is_symlink: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
}

impl FileSystem for WorkspaceExecutionEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_existing_workspace_path(path)?;
            self.inner.path_metadata(&resolved).await
        })
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_existing_workspace_path(path)?;
            self.inner.read_to_string(&resolved).await
        })
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_existing_workspace_path(path)?;
            self.inner.read_bytes(&resolved).await
        })
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_write_path(path)?;
            self.inner.write_string(&resolved, content).await
        })
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_write_path(path)?;
            self.inner.write_bytes(&resolved, content).await
        })
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_create_dir_path(path)?;
            self.inner.create_dir_all(&resolved).await
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_existing_workspace_path(path)?;
            self.inner.read_dir(&resolved).await
        })
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_existing_workspace_path(path)?;
            self.inner.remove_file(&resolved).await
        })
    }
}

impl ProcessRunner for WorkspaceExecutionEnv {
    fn run_process<'a>(
        &'a self,
        mut request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        Box::pin(async move {
            request.cwd = self.ensure_existing_workspace_path(&request.cwd)?;
            self.inner.run_process(request).await
        })
    }
}

impl NetworkEgress for WorkspaceExecutionEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        self.inner.send_network_request(request)
    }
}

impl ExecutionEnv for WorkspaceExecutionEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        session: &'a ExecutionSession,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        self.inner.execute_skill(skill, input, session)
    }
}

impl FileSystem for NetworkedExecutionEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        self.inner.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        self.inner.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        self.inner.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        self.inner.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.remove_file(path)
    }
}

impl ProcessRunner for NetworkedExecutionEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        self.inner.run_process(request)
    }
}

impl NetworkEgress for NetworkedExecutionEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        self.network.send_network_request(request)
    }
}

impl ExecutionEnv for NetworkedExecutionEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        session: &'a ExecutionSession,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        self.inner.execute_skill(skill, input, session)
    }
}

impl FileSystem for DryRunExecutionEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        self.inner.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        self.inner.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        self.inner.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        _content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let path = path.to_path_buf();
        Box::pin(async move {
            let _ = path;
            Ok(())
        })
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        _content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let path = path.to_path_buf();
        Box::pin(async move {
            let _ = path;
            Ok(())
        })
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let path = path.to_path_buf();
        Box::pin(async move {
            let _ = path;
            Ok(())
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        self.inner.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let path = path.to_path_buf();
        Box::pin(async move {
            let _ = path;
            Ok(())
        })
    }
}

impl ProcessRunner for DryRunExecutionEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        Box::pin(async move {
            Ok(ProcessOutput {
                status: 0,
                stdout: format!("dry-run: skipped command {}", request.command),
                stderr: String::new(),
            })
        })
    }
}

impl NetworkEgress for DryRunExecutionEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        Box::pin(async move {
            Ok(NetworkEgressResponse {
                status: 200,
                body: format!(
                    "{{\"dry_run\":true,\"url\":{}}}",
                    serde_json::to_string(&request.url)?
                ),
            })
        })
    }
}

impl ExecutionEnv for DryRunExecutionEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        session: &'a ExecutionSession,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        self.inner.execute_skill(skill, input, session)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessRequest {
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub use_shell: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_bytes: Option<usize>,
}

impl ProcessRequest {
    pub fn shell(command: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            use_shell: true,
            stdin: None,
            timeout_ms: None,
            max_output_bytes: None,
        }
    }

    pub fn program(program: impl Into<String>, args: Vec<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command: program.into(),
            args,
            cwd: cwd.into(),
            use_shell: false,
            stdin: None,
            timeout_ms: None,
            max_output_bytes: None,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkEgressRequest {
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkEgressResponse {
    pub status: u16,
    pub body: String,
}

impl FileSystem for LocalExecutionEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        Box::pin(async move {
            let metadata =
                fs::symlink_metadata(path).map_err(|source| IkarosError::io(path, source))?;
            let file_type = metadata.file_type();
            Ok(FileMetadata {
                is_file: metadata.is_file(),
                is_dir: metadata.is_dir(),
                is_symlink: file_type.is_symlink(),
                modified_at: metadata.modified().ok().and_then(system_time_to_rfc3339),
            })
        })
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(
            async move { fs::read_to_string(path).map_err(|source| IkarosError::io(path, source)) },
        )
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        Box::pin(async move { fs::read(path).map_err(|source| IkarosError::io(path, source)) })
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
            }
            fs::write(path, content).map_err(|source| IkarosError::io(path, source))
        })
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
            }
            fs::write(path, content).map_err(|source| IkarosError::io(path, source))
        })
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(
            async move { fs::create_dir_all(path).map_err(|source| IkarosError::io(path, source)) },
        )
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        Box::pin(async move {
            let mut entries = Vec::new();
            for entry in fs::read_dir(path).map_err(|source| IkarosError::io(path, source))? {
                let entry = entry.map_err(|source| IkarosError::io(path, source))?;
                entries.push(entry.file_name().to_string_lossy().to_string());
            }
            entries.sort();
            Ok(entries)
        })
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(
            async move { fs::remove_file(path).map_err(|source| IkarosError::io(path, source)) },
        )
    }
}

impl ProcessRunner for LocalExecutionEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        Box::pin(async move {
            let mut command = command_from_request(&request);
            command.kill_on_drop(true);
            let max_output_bytes = request.max_output_bytes;
            let mut child = command.spawn().map_err(|source| {
                IkarosError::Message(format!("failed to start command: {source}"))
            })?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| IkarosError::Message("failed to open command stdout".into()))?;
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| IkarosError::Message("failed to open command stderr".into()))?;
            let stdout_task = tokio::spawn(read_process_pipe(stdout, max_output_bytes, "stdout"));
            let stderr_task = tokio::spawn(read_process_pipe(stderr, max_output_bytes, "stderr"));
            if let Some(stdin) = request.stdin {
                let mut child_stdin = child
                    .stdin
                    .take()
                    .ok_or_else(|| IkarosError::Message("failed to open command stdin".into()))?;
                tokio::spawn(async move {
                    let _ = child_stdin.write_all(stdin.as_bytes()).await;
                });
            }
            let status = match wait_for_process(&mut child, request.timeout_ms).await {
                Ok(status) => status,
                Err(error) => {
                    stdout_task.abort();
                    stderr_task.abort();
                    return Err(error);
                }
            };
            let stdout = read_process_pipe_result(stdout_task, "stdout").await?;
            let stderr = read_process_pipe_result(stderr_task, "stderr").await?;
            Ok(ProcessOutput {
                status: status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&stdout).to_string(),
                stderr: String::from_utf8_lossy(&stderr).to_string(),
            })
        })
    }
}

async fn wait_for_process(
    child: &mut tokio::process::Child,
    timeout_ms: Option<u64>,
) -> Result<std::process::ExitStatus> {
    if let Some(timeout_ms) = timeout_ms {
        match time::timeout(time::Duration::from_millis(timeout_ms), child.wait()).await {
            Ok(result) => result
                .map_err(|source| IkarosError::Message(format!("failed to run command: {source}"))),
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                Err(IkarosError::Message("command timed out".into()))
            }
        }
    } else {
        child
            .wait()
            .await
            .map_err(|source| IkarosError::Message(format!("failed to run command: {source}")))
    }
}

async fn read_process_pipe<R>(
    mut pipe: R,
    max_output_bytes: Option<usize>,
    label: &'static str,
) -> Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = pipe.read(&mut buffer).await.map_err(|source| {
            IkarosError::Message(format!("failed to read command {label}: {source}"))
        })?;
        if read == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..read]);
        if let Some(max_output_bytes) = max_output_bytes
            && output.len() > max_output_bytes
        {
            return Err(IkarosError::Message(format!(
                "command {label} exceeded {max_output_bytes} bytes"
            )));
        }
    }
    Ok(output)
}

async fn read_process_pipe_result(
    task: tokio::task::JoinHandle<Result<Vec<u8>>>,
    label: &str,
) -> Result<Vec<u8>> {
    task.await.map_err(|source| {
        IkarosError::Message(format!("failed to join command {label} reader: {source}"))
    })?
}

fn command_from_request(request: &ProcessRequest) -> Command {
    if request.use_shell {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(&request.command);
            configure_process_stdio(&mut cmd, request);
            return cmd;
        }

        #[cfg(not(windows))]
        {
            let mut cmd = Command::new("sh");
            cmd.arg("-lc").arg(&request.command);
            configure_process_stdio(&mut cmd, request);
            return cmd;
        }
    }

    let mut cmd = Command::new(&request.command);
    cmd.args(&request.args);
    configure_process_stdio(&mut cmd, request);
    cmd
}

fn configure_process_stdio(cmd: &mut Command, request: &ProcessRequest) {
    cmd.current_dir(&request.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
}

fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

impl NetworkEgress for LocalExecutionEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        Box::pin(async move {
            Err(IkarosError::Message(format!(
                "no network backend is configured for {} {}",
                request.method, request.url
            )))
        })
    }
}

impl ExecutionEnv for LocalExecutionEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        session: &'a ExecutionSession,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        Box::pin(async move {
            skill
                .execute(
                    input,
                    SkillContext {
                        session: session.clone(),
                    },
                )
                .await
        })
    }
}

fn system_time_to_rfc3339(time: std::time::SystemTime) -> Option<String> {
    let duration = time.duration_since(std::time::UNIX_EPOCH).ok()?;
    let datetime = ::time::OffsetDateTime::from_unix_timestamp(duration.as_secs() as i64).ok()?;
    datetime
        .format(&::time::format_description::well_known::Rfc3339)
        .ok()
}
