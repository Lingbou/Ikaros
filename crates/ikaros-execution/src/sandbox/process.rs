// SPDX-License-Identifier: GPL-3.0-only

use super::*;
impl ProcessRunner for LocalExecutionEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        Box::pin(async move {
            let command_name = redact_secrets(&request.command);
            let cwd = redact_secrets(&request.cwd.display().to_string());
            let args_count = request.args.len();
            let env_count = request.env.len();
            let stdin_bytes = request.stdin.as_ref().map(|stdin| stdin.len()).unwrap_or(0);
            let timeout_ms = request.timeout_ms;
            let max_output_bytes = request.max_output_bytes;
            tracing::info!(
                event = "harness_process_started",
                command = %command_name,
                args_count,
                cwd = %cwd,
                use_shell = request.use_shell,
                cwd_scope = ?request.cwd_scope,
                stdin_bytes,
                env_count,
                timeout_ms,
                max_output_bytes,
                "harness process started"
            );
            let result: Result<ProcessOutput> =
                async {
                    let mut command = command_from_request(&request);
                    command.kill_on_drop(true);
                    let max_output_bytes = request.max_output_bytes;
                    let mut child = command.spawn().map_err(|source| {
                        IkarosError::Message(format!("failed to start command: {source}"))
                    })?;
                    let stdout = child.stdout.take().ok_or_else(|| {
                        IkarosError::Message("failed to open command stdout".into())
                    })?;
                    let stderr = child.stderr.take().ok_or_else(|| {
                        IkarosError::Message("failed to open command stderr".into())
                    })?;
                    let mut stdout_task =
                        tokio::spawn(read_process_pipe(stdout, max_output_bytes, "stdout"));
                    let mut stderr_task =
                        tokio::spawn(read_process_pipe(stderr, max_output_bytes, "stderr"));
                    let deadline = request.timeout_ms.map(|timeout_ms| {
                        time::Instant::now() + time::Duration::from_millis(timeout_ms)
                    });
                    if let Some(stdin) = request.stdin {
                        let mut child_stdin = child.stdin.take().ok_or_else(|| {
                            IkarosError::Message("failed to open command stdin".into())
                        })?;
                        tokio::spawn(async move {
                            let _ = child_stdin.write_all(stdin.as_bytes()).await;
                        });
                    }
                    let mut stdout = None;
                    let mut stderr = None;
                    let status = loop {
                        tokio::select! {
                            result = wait_for_process(&mut child, deadline) => {
                                match result {
                                    Ok(status) => break status,
                                    Err(error) => {
                                        stdout_task.abort();
                                        stderr_task.abort();
                                        return Err(error);
                                    }
                                }
                            }
                            result = &mut stdout_task, if stdout.is_none() => {
                                match read_process_pipe_join_result(result, "stdout") {
                                    Ok(output) => stdout = Some(output),
                                    Err(error) => {
                                        stderr_task.abort();
                                        kill_process_group(&mut child).await;
                                        return Err(error);
                                    }
                                }
                            }
                            result = &mut stderr_task, if stderr.is_none() => {
                                match read_process_pipe_join_result(result, "stderr") {
                                    Ok(output) => stderr = Some(output),
                                    Err(error) => {
                                        stdout_task.abort();
                                        kill_process_group(&mut child).await;
                                        return Err(error);
                                    }
                                }
                            }
                        }
                    };
                    let stdout = match stdout {
                        Some(output) => output,
                        None => read_process_pipe_result(stdout_task, "stdout").await?,
                    };
                    let stderr = match stderr {
                        Some(output) => output,
                        None => read_process_pipe_result(stderr_task, "stderr").await?,
                    };
                    Ok(ProcessOutput {
                        status: status.code().unwrap_or(-1),
                        stdout: String::from_utf8_lossy(&stdout).to_string(),
                        stderr: String::from_utf8_lossy(&stderr).to_string(),
                    })
                }
                .await;
            match &result {
                Ok(output) => {
                    tracing::info!(
                        event = "harness_process_completed",
                        command = %command_name,
                        args_count,
                        cwd = %cwd,
                        status = output.status,
                        stdout_bytes = output.stdout.len(),
                        stderr_bytes = output.stderr.len(),
                        "harness process completed"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        event = "harness_process_failed",
                        command = %command_name,
                        args_count,
                        cwd = %cwd,
                        error = %redact_secrets(&error.to_string()),
                        "harness process failed"
                    );
                }
            }
            result
        })
    }
}

pub(super) async fn wait_for_process(
    child: &mut tokio::process::Child,
    deadline: Option<time::Instant>,
) -> Result<std::process::ExitStatus> {
    if let Some(deadline) = deadline {
        let now = time::Instant::now();
        if deadline <= now {
            kill_process_group(child).await;
            return Err(IkarosError::Message("command timed out".into()));
        }
        match time::timeout(deadline.duration_since(now), child.wait()).await {
            Ok(result) => result
                .map_err(|source| IkarosError::Message(format!("failed to run command: {source}"))),
            Err(_) => {
                kill_process_group(child).await;
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

#[cfg(unix)]
pub(super) async fn kill_process_group(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        if let Some(process_group) = rustix::process::Pid::from_raw(pid as rustix::process::RawPid)
        {
            let _ =
                rustix::process::kill_process_group(process_group, rustix::process::Signal::KILL);
        }
    } else {
        let _ = child.start_kill();
    }
    let _ = child.wait().await;
}

#[cfg(not(unix))]
pub(super) async fn kill_process_group(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

pub(super) async fn read_process_pipe<R>(
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

pub(super) async fn read_process_pipe_result(
    task: tokio::task::JoinHandle<Result<Vec<u8>>>,
    label: &str,
) -> Result<Vec<u8>> {
    read_process_pipe_join_result(task.await, label)
}

pub(super) fn read_process_pipe_join_result(
    result: std::result::Result<Result<Vec<u8>>, tokio::task::JoinError>,
    label: &str,
) -> Result<Vec<u8>> {
    result.map_err(|source| {
        IkarosError::Message(format!("failed to join command {label} reader: {source}"))
    })?
}

pub(super) fn command_from_request(request: &ProcessRequest) -> Command {
    if request.use_shell {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(&request.command);
            configure_process_stdio(&mut cmd, request);
            configure_process_group(&mut cmd);
            return cmd;
        }

        #[cfg(not(windows))]
        {
            let mut cmd = Command::new("sh");
            cmd.arg("-lc").arg(&request.command);
            configure_process_stdio(&mut cmd, request);
            configure_process_group(&mut cmd);
            return cmd;
        }
    }

    let mut cmd = Command::new(&request.command);
    cmd.args(&request.args);
    configure_process_stdio(&mut cmd, request);
    configure_process_group(&mut cmd);
    cmd
}

#[cfg(unix)]
pub(super) fn configure_process_group(cmd: &mut Command) {
    cmd.process_group(0);
}

#[cfg(not(unix))]
pub(super) fn configure_process_group(_cmd: &mut Command) {}

pub(super) fn configure_process_stdio(cmd: &mut Command, request: &ProcessRequest) {
    cmd.current_dir(&request.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    configure_process_environment(cmd, request);
}

pub(super) fn configure_process_environment(cmd: &mut Command, request: &ProcessRequest) {
    cmd.env_clear();
    for (name, value) in baseline_process_environment() {
        cmd.env(name, value);
    }
    for (name, value) in &request.env {
        cmd.env(name, value);
    }
}

pub(super) fn baseline_process_environment() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for name in baseline_process_environment_names() {
        if let Ok(value) = std::env::var(name)
            && !value.is_empty()
        {
            env.insert((*name).to_owned(), value);
        }
    }
    env
}

#[cfg(windows)]
pub(super) fn baseline_process_environment_names() -> &'static [&'static str] {
    &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
    ]
}

#[cfg(not(windows))]
pub(super) fn baseline_process_environment_names() -> &'static [&'static str] {
    &["PATH", "TMPDIR", "TEMP", "TMP"]
}

pub(super) fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub(super) fn normalize_path(path: &Path) -> PathBuf {
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
