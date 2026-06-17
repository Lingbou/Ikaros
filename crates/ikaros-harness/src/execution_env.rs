// SPDX-License-Identifier: GPL-3.0-only

use crate::{Skill, SkillContext, SkillOutput, session::ExecutionSession};
use ikaros_core::{IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    future::Future,
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
    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkEgressResponse {
    pub status: u16,
    pub body: String,
}

impl FileSystem for LocalExecutionEnv {
    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(
            async move { fs::read_to_string(path).map_err(|source| IkarosError::io(path, source)) },
        )
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
