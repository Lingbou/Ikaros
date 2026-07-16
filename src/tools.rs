use crate::domain::ToolCall;
use anyhow::{Context, Result, bail};
use atomic_write_file::AtomicWriteFile;
use command_group::AsyncCommandGroup;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    env, fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::mpsc,
    task::JoinHandle,
    time::{Instant, sleep_until, timeout, timeout_at},
};

const MAX_READ_BYTES: u64 = 512 * 1024;
const MAX_WRITE_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_LIST_ENTRIES: usize = 512;
const MAX_LIST_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_COMMAND_SECONDS: u64 = 60;

#[derive(Debug, Clone)]
pub struct ToolBox {
    workspace: PathBuf,
}

impl ToolBox {
    pub fn new(workspace: &Path) -> Result<Self> {
        let workspace = workspace
            .canonicalize()
            .with_context(|| format!("failed to resolve workspace {}", workspace.display()))?;
        if !workspace.is_dir() {
            bail!("workspace is not a directory: {}", workspace.display());
        }
        Ok(Self { workspace })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn definitions(&self) -> Vec<Value> {
        vec![
            function_tool(
                "list_dir",
                "List one directory inside the workspace.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "default": ".", "description": "Workspace-relative directory"}
                    },
                    "additionalProperties": false
                }),
            ),
            function_tool(
                "read_file",
                "Read UTF-8 text from a workspace file with line limits.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "offset": {"type": "integer", "minimum": 0, "default": 0},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200}
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
            ),
            function_tool(
                "write_file",
                "Replace one UTF-8 workspace file. The user must approve the exact path and content.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            ),
            function_tool(
                "run_command",
                "Run one program directly in the workspace. This is not a shell and does not provide OS sandboxing.",
                json!({
                    "type": "object",
                    "properties": {
                        "program": {"type": "string"},
                        "args": {"type": "array", "items": {"type": "string"}, "default": []},
                        "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 60, "default": 30}
                    },
                    "required": ["program"],
                    "additionalProperties": false
                }),
            ),
        ]
    }

    pub async fn execute(&self, call: &ToolCall) -> Result<String> {
        match call.name.as_str() {
            "list_dir" => self.list_dir(serde_json::from_value(call.arguments.clone())?),
            "read_file" => self.read_file(serde_json::from_value(call.arguments.clone())?),
            "write_file" => self.write_file(serde_json::from_value(call.arguments.clone())?),
            "run_command" => {
                self.run_command(serde_json::from_value(call.arguments.clone())?)
                    .await
            }
            other => bail!("unknown tool: {other}"),
        }
    }

    fn list_dir(&self, args: ListDirArgs) -> Result<String> {
        let path = args.path;
        let directory = self.resolve_existing(&path, true)?;
        if !directory.is_dir() {
            bail!("not a directory: {path}");
        }
        let mut entries = Vec::new();
        let mut output_bytes = 0;
        for entry in fs::read_dir(&directory).with_context(|| format!("failed to list {path}"))? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let suffix = if kind.is_dir() {
                "/"
            } else if kind.is_symlink() {
                "@"
            } else {
                ""
            };
            let rendered = format!("{}{}", entry.file_name().to_string_lossy(), suffix);
            push_directory_entry(&mut entries, &mut output_bytes, rendered)?;
        }
        entries.sort_unstable();
        Ok(entries.join("\n"))
    }

    fn read_file(&self, args: ReadFileArgs) -> Result<String> {
        let path = self.resolve_existing(&args.path, false)?;
        if !path.is_file() {
            bail!("not a file: {}", args.path);
        }
        let metadata = fs::metadata(&path)?;
        if metadata.len() > MAX_READ_BYTES {
            bail!(
                "file is too large: {} bytes (limit {})",
                metadata.len(),
                MAX_READ_BYTES
            );
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {} as UTF-8", args.path))?;
        if !(1..=1000).contains(&args.limit) {
            bail!("limit must be between 1 and 1000");
        }
        let offset = args.offset;
        let limit = args.limit;
        let lines = text
            .lines()
            .enumerate()
            .skip(offset)
            .take(limit)
            .map(|(index, line)| format!("{}: {line}", index + 1))
            .collect::<Vec<_>>();
        Ok(lines.join("\n"))
    }

    fn write_file(&self, args: WriteFileArgs) -> Result<String> {
        if args.content.len() > MAX_WRITE_BYTES {
            bail!(
                "content is too large: {} bytes (limit {})",
                args.content.len(),
                MAX_WRITE_BYTES
            );
        }
        let path = self.resolve_for_write(&args.path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
            let canonical_parent = parent
                .canonicalize()
                .with_context(|| format!("failed to resolve {}", parent.display()))?;
            self.ensure_inside_workspace(&canonical_parent)?;
            let file_name = path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("path has no file name: {}", args.path))?;
            let destination = canonical_parent.join(file_name);
            if fs::symlink_metadata(&destination)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                bail!("refusing to replace a symbolic link: {}", args.path);
            }
            if destination.is_dir() {
                bail!("cannot replace a directory: {}", args.path);
            }
            replace_file(&destination, args.content.as_bytes())
                .with_context(|| format!("failed to replace {}", args.path))?;
        }
        Ok(format!(
            "wrote {} bytes to {}",
            args.content.len(),
            args.path
        ))
    }

    async fn run_command(&self, args: RunCommandArgs) -> Result<String> {
        if args.program.trim().is_empty() {
            bail!("program is empty");
        }
        if !(1..=MAX_COMMAND_SECONDS).contains(&args.timeout_seconds) {
            bail!("timeout_seconds must be between 1 and {MAX_COMMAND_SECONDS}");
        }
        let timeout_seconds = args.timeout_seconds;
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        let mut command = Command::new(&args.program);
        command
            .args(&args.args)
            .current_dir(&self.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_command_environment(&mut command);
        let mut child = command
            .group()
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to start program {}", args.program))?;
        let mut process_guard = ProcessGroupDropGuard::new(child.id());
        let stdout = child
            .inner()
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to capture command stdout"))?;
        let stderr = child
            .inner()
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to capture command stderr"))?;
        let total = Arc::new(AtomicUsize::new(0));
        let (overflow_tx, mut overflow_rx) = mpsc::channel(1);
        let mut stdout_task =
            tokio::spawn(capture_stream(stdout, total.clone(), overflow_tx.clone()));
        let mut stderr_task =
            tokio::spawn(capture_stream(stderr, total.clone(), overflow_tx.clone()));
        let _overflow_guard = overflow_tx;
        let end = tokio::select! {
            status = child.wait() => CommandEnd::Exited(status?),
            signal = overflow_rx.recv() => {
                debug_assert!(signal.is_some());
                CommandEnd::OutputLimit
            }
            _ = sleep_until(deadline) => CommandEnd::TimedOut,
        };
        let status = match end {
            CommandEnd::Exited(status) => status,
            CommandEnd::OutputLimit => {
                terminate_process_group(&mut child).await;
                stdout_task.abort();
                stderr_task.abort();
                bail!("command exceeded the {MAX_OUTPUT_BYTES}-byte output limit");
            }
            CommandEnd::TimedOut => {
                terminate_process_group(&mut child).await;
                stdout_task.abort();
                stderr_task.abort();
                bail!("command timed out after {timeout_seconds} seconds");
            }
        };
        let captured = timeout_at(deadline, async {
            let stdout = join_capture(&mut stdout_task).await?;
            let stderr = join_capture(&mut stderr_task).await?;
            Ok::<_, anyhow::Error>((stdout, stderr))
        })
        .await;
        let (stdout, stderr) = match captured {
            Ok(result) => result?,
            Err(_) => {
                terminate_process_group(&mut child).await;
                stdout_task.abort();
                stderr_task.abort();
                bail!("command timed out after {timeout_seconds} seconds");
            }
        };
        if total.load(Ordering::Relaxed) > MAX_OUTPUT_BYTES {
            bail!("command exceeded the {MAX_OUTPUT_BYTES}-byte output limit");
        }
        process_guard.disarm();
        Ok(format!(
            "exit_code: {}\nstdout:\n{}\nstderr:\n{}",
            status
                .code()
                .map_or_else(|| "terminated".to_owned(), |code| code.to_string()),
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        ))
    }

    fn resolve_existing(&self, input: &str, allow_root: bool) -> Result<PathBuf> {
        let relative = safe_relative_path(input)?;
        if relative.as_os_str().is_empty() && !allow_root {
            bail!("path must name a file inside the workspace");
        }
        let path = self.workspace.join(relative);
        let canonical = path
            .canonicalize()
            .with_context(|| format!("path does not exist: {input}"))?;
        self.ensure_inside_workspace(&canonical)?;
        Ok(canonical)
    }

    fn resolve_for_write(&self, input: &str) -> Result<PathBuf> {
        let relative = safe_relative_path(input)?;
        if relative.as_os_str().is_empty() {
            bail!("path must name a file inside the workspace");
        }
        let target = self.workspace.join(relative);
        if target.exists() {
            let canonical = target
                .canonicalize()
                .with_context(|| format!("failed to resolve {input}"))?;
            self.ensure_inside_workspace(&canonical)?;
            return Ok(target);
        }

        let mut ancestor = target.parent();
        while let Some(path) = ancestor {
            if path.exists() {
                let canonical = path
                    .canonicalize()
                    .with_context(|| format!("failed to resolve {}", path.display()))?;
                self.ensure_inside_workspace(&canonical)?;
                return Ok(target);
            }
            ancestor = path.parent();
        }
        bail!("could not resolve a parent directory for {input}")
    }

    fn ensure_inside_workspace(&self, path: &Path) -> Result<()> {
        if !path.starts_with(&self.workspace) {
            bail!("path escapes the workspace: {}", path.display());
        }
        Ok(())
    }
}

fn function_tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters
        }
    })
}

fn push_directory_entry(
    entries: &mut Vec<String>,
    output_bytes: &mut usize,
    entry: String,
) -> Result<()> {
    if entries.len() >= MAX_LIST_ENTRIES {
        bail!("directory has more than {MAX_LIST_ENTRIES} entries");
    }
    let separator_bytes = usize::from(!entries.is_empty());
    let next_bytes = output_bytes
        .saturating_add(separator_bytes)
        .saturating_add(entry.len());
    if next_bytes > MAX_LIST_OUTPUT_BYTES {
        bail!("directory listing exceeds the {MAX_LIST_OUTPUT_BYTES}-byte output limit");
    }
    *output_bytes = next_bytes;
    entries.push(entry);
    Ok(())
}

fn safe_relative_path(input: &str) -> Result<PathBuf> {
    if input.trim() != input {
        bail!("path must not have leading or trailing whitespace: {input:?}");
    }
    let path = Path::new(input);
    if path.is_absolute() {
        bail!("absolute paths are not allowed: {input}");
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("path traversal is not allowed: {input}")
            }
        }
    }
    Ok(relative)
}

fn configure_command_environment(command: &mut Command) {
    const ALLOWED_NAMES: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "HOME",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "LANG",
        "LC_ALL",
        "TERM",
        "NO_COLOR",
    ];
    let values = ALLOWED_NAMES
        .iter()
        .filter_map(|name| env::var_os(name).map(|value| (*name, value)))
        .collect::<Vec<_>>();
    command.env_clear();
    for (name, value) in values {
        command.env(name, value);
    }
}

fn replace_file(path: &Path, content: &[u8]) -> Result<()> {
    let mut file = AtomicWriteFile::open(path)
        .with_context(|| format!("failed to open atomic writer for {}", path.display()))?;
    file.write_all(content)?;
    file.commit()
        .context("failed to atomically install replacement file")
}

enum CommandEnd {
    Exited(std::process::ExitStatus),
    OutputLimit,
    TimedOut,
}

struct ProcessGroupDropGuard {
    #[cfg(unix)]
    pgid: Option<nix::unistd::Pid>,
}

impl ProcessGroupDropGuard {
    fn new(process_id: Option<u32>) -> Self {
        #[cfg(unix)]
        {
            let pgid = process_id
                .and_then(|id| i32::try_from(id).ok())
                .map(nix::unistd::Pid::from_raw);
            Self { pgid }
        }
        #[cfg(not(unix))]
        {
            let _ = process_id;
            Self {}
        }
    }

    fn disarm(&mut self) {
        #[cfg(unix)]
        {
            self.pgid = None;
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGroupDropGuard {
    fn drop(&mut self) {
        if let Some(pgid) = self.pgid.take() {
            let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL);
        }
    }
}

async fn capture_stream<R>(
    mut stream: R,
    total: Arc<AtomicUsize>,
    overflow: mpsc::Sender<()>,
) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut captured = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = stream.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let previous = total.fetch_add(count, Ordering::Relaxed);
        if previous < MAX_OUTPUT_BYTES {
            let allowed = count.min(MAX_OUTPUT_BYTES - previous);
            captured.extend_from_slice(&buffer[..allowed]);
        }
        if previous.saturating_add(count) > MAX_OUTPUT_BYTES {
            let _ = overflow.try_send(());
        }
    }
    Ok(captured)
}

async fn join_capture(task: &mut JoinHandle<std::io::Result<Vec<u8>>>) -> Result<Vec<u8>> {
    task.await
        .context("command output task failed")?
        .map_err(Into::into)
}

async fn terminate_process_group(child: &mut command_group::AsyncGroupChild) {
    let _ = timeout(Duration::from_secs(5), child.kill()).await;
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListDirArgs {
    #[serde(default = "default_list_dir_path")]
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadFileArgs {
    path: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_read_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteFileArgs {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunCommandArgs {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_command_timeout")]
    timeout_seconds: u64,
}

fn default_list_dir_path() -> String {
    ".".to_owned()
}

fn default_read_limit() -> usize {
    200
}

fn default_command_timeout() -> u64 {
    30
}

#[cfg(test)]
mod tests {
    use super::ToolBox;
    use crate::domain::ToolCall;
    use serde_json::json;
    use tempfile::tempdir;

    #[tokio::test]
    async fn writes_and_reads_inside_workspace() {
        let temp = tempdir().expect("tempdir");
        let tools = ToolBox::new(temp.path()).expect("tools");
        tools
            .execute(&ToolCall {
                id: "write".to_owned(),
                name: "write_file".to_owned(),
                arguments: json!({"path": "notes/test.txt", "content": "one\ntwo"}),
            })
            .await
            .expect("write");
        let output = tools
            .execute(&ToolCall {
                id: "read".to_owned(),
                name: "read_file".to_owned(),
                arguments: json!({"path": "notes/test.txt"}),
            })
            .await
            .expect("read");
        assert_eq!(output, "1: one\n2: two");
    }

    #[tokio::test]
    async fn rejects_parent_and_absolute_paths() {
        let temp = tempdir().expect("tempdir");
        let tools = ToolBox::new(temp.path()).expect("tools");
        let parent = tools
            .execute(&ToolCall {
                id: "parent".to_owned(),
                name: "read_file".to_owned(),
                arguments: json!({"path": "../outside.txt"}),
            })
            .await;
        assert!(parent.is_err());

        let absolute_path = temp.path().join("absolute.txt");
        let absolute = tools
            .execute(&ToolCall {
                id: "absolute".to_owned(),
                name: "write_file".to_owned(),
                arguments: json!({"path": absolute_path, "content": "no"}),
            })
            .await;
        assert!(absolute.is_err());

        let padded = tools
            .execute(&ToolCall {
                id: "padded".to_owned(),
                name: "write_file".to_owned(),
                arguments: json!({"path": "notes.txt ", "content": "no"}),
            })
            .await;
        assert!(padded.is_err());
        assert!(!temp.path().join("notes.txt").exists());
    }

    #[tokio::test]
    async fn rejects_arguments_that_do_not_match_the_approved_schema() {
        let temp = tempdir().expect("tempdir");
        std::fs::write(temp.path().join("notes.txt"), "one").expect("fixture");
        let tools = ToolBox::new(temp.path()).expect("tools");

        for arguments in [
            json!({"path": "notes.txt", "limit": 0}),
            json!({
                "path": "notes.txt",
                "limit": 1001
            }),
        ] {
            assert!(
                tools
                    .execute(&ToolCall {
                        id: "bad-limit".to_owned(),
                        name: "read_file".to_owned(),
                        arguments,
                    })
                    .await
                    .is_err()
            );
        }

        assert!(
            tools
                .execute(&ToolCall {
                    id: "bad-timeout".to_owned(),
                    name: "run_command".to_owned(),
                    arguments: json!({"program": "unused", "timeout_seconds": 61}),
                })
                .await
                .is_err()
        );
        assert!(
            tools
                .execute(&ToolCall {
                    id: "unknown-field".to_owned(),
                    name: "list_dir".to_owned(),
                    arguments: json!({"path": ".", "recursive": true}),
                })
                .await
                .is_err()
        );
    }

    #[test]
    fn directory_listing_limits_are_checked_before_allocation_grows_unbounded() {
        let mut entries = Vec::new();
        let mut bytes = 0;
        for _ in 0..super::MAX_LIST_ENTRIES {
            super::push_directory_entry(&mut entries, &mut bytes, "x".to_owned())
                .expect("within entry limit");
        }
        assert!(super::push_directory_entry(&mut entries, &mut bytes, "x".to_owned()).is_err());

        let mut entries = Vec::new();
        let mut bytes = 0;
        assert!(
            super::push_directory_entry(
                &mut entries,
                &mut bytes,
                "x".repeat(super::MAX_LIST_OUTPUT_BYTES + 1),
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_symlink_escape_when_supported() {
        let workspace = tempdir().expect("workspace");
        let outside = tempdir().expect("outside");
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, "original").expect("outside file");
        let link = workspace.path().join("escape.txt");
        if create_file_symlink(&outside_file, &link).is_err() {
            return;
        }
        let tools = ToolBox::new(workspace.path()).expect("tools");
        let result = tools
            .execute(&ToolCall {
                id: "escape".to_owned(),
                name: "write_file".to_owned(),
                arguments: json!({"path": "escape.txt", "content": "changed"}),
            })
            .await;
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(outside_file).expect("outside contents"),
            "original"
        );
    }

    #[tokio::test]
    async fn replacing_a_hard_link_does_not_modify_the_external_inode() {
        let workspace = tempdir().expect("workspace");
        let outside = tempdir().expect("outside");
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, "original").expect("outside file");
        let link = workspace.path().join("linked.txt");
        std::fs::hard_link(&outside_file, &link).expect("hard link");
        let tools = ToolBox::new(workspace.path()).expect("tools");
        tools
            .execute(&ToolCall {
                id: "replace".to_owned(),
                name: "write_file".to_owned(),
                arguments: json!({"path": "linked.txt", "content": "replacement"}),
            })
            .await
            .expect("replace");
        assert_eq!(
            std::fs::read_to_string(outside_file).expect("outside contents"),
            "original"
        );
        assert_eq!(
            std::fs::read_to_string(link).expect("workspace contents"),
            "replacement"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replacing_a_file_preserves_its_mode() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("script.sh");
        std::fs::write(&path, "old").expect("write old");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("set mode");
        let tools = ToolBox::new(temp.path()).expect("tools");
        tools
            .execute(&ToolCall {
                id: "replace-script".to_owned(),
                name: "write_file".to_owned(),
                arguments: json!({"path": "script.sh", "content": "new"}),
            })
            .await
            .expect("replace");
        let mode = std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[tokio::test]
    async fn stream_capture_stops_buffering_at_the_global_limit() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use tokio::{io::AsyncWriteExt, sync::mpsc};

        let (mut writer, reader) = tokio::io::duplex(super::MAX_OUTPUT_BYTES * 2);
        let total = Arc::new(AtomicUsize::new(0));
        let (overflow_tx, mut overflow_rx) = mpsc::channel(1);
        let capture = tokio::spawn(super::capture_stream(reader, total.clone(), overflow_tx));
        writer
            .write_all(&vec![b'x'; super::MAX_OUTPUT_BYTES + 1])
            .await
            .expect("write");
        drop(writer);
        let bytes = capture.await.expect("task").expect("capture");
        assert_eq!(bytes.len(), super::MAX_OUTPUT_BYTES);
        assert!(total.load(Ordering::Relaxed) > super::MAX_OUTPUT_BYTES);
        assert_eq!(overflow_rx.recv().await, Some(()));
    }

    #[tokio::test]
    async fn timeout_covers_descendant_processes() {
        let temp = tempdir().expect("tempdir");
        let tools = ToolBox::new(temp.path()).expect("tools");
        let (program, args) = descendant_command();
        let started = std::time::Instant::now();
        let error = tools
            .execute(&ToolCall {
                id: "descendant-timeout".to_owned(),
                name: "run_command".to_owned(),
                arguments: json!({
                    "program": program,
                    "args": args,
                    "timeout_seconds": 1
                }),
            })
            .await
            .expect_err("process group must time out");
        assert!(error.to_string().contains("timed out after 1 seconds"));
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
    }

    #[tokio::test]
    async fn dropping_command_future_kills_its_process_group() {
        let temp = tempdir().expect("tempdir");
        let tools = ToolBox::new(temp.path()).expect("tools");
        let (program, args) = cancellation_command();
        let call = ToolCall {
            id: "cancel-process-group".to_owned(),
            name: "run_command".to_owned(),
            arguments: json!({
                "program": program,
                "args": args,
                "timeout_seconds": 30
            }),
        };
        let task = tokio::spawn(async move { tools.execute(&call).await });
        let started = temp.path().join("started.txt");
        for _ in 0..40 {
            if started.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(started.exists(), "command did not start");
        task.abort();
        assert!(
            task.await
                .expect_err("task must be cancelled")
                .is_cancelled()
        );
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert!(!temp.path().join("survived.txt").exists());
    }

    #[cfg(windows)]
    fn descendant_command() -> (&'static str, Vec<&'static str>) {
        (
            "powershell.exe",
            vec![
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Process powershell.exe -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30') -NoNewWindow",
            ],
        )
    }

    #[cfg(unix)]
    fn descendant_command() -> (&'static str, Vec<&'static str>) {
        ("sh", vec!["-c", "sleep 30 &"])
    }

    #[cfg(windows)]
    fn cancellation_command() -> (&'static str, Vec<&'static str>) {
        (
            "powershell.exe",
            vec![
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Set-Content -LiteralPath started.txt -Value started; Start-Sleep -Seconds 2; Set-Content -LiteralPath survived.txt -Value survived",
            ],
        )
    }

    #[cfg(unix)]
    fn cancellation_command() -> (&'static str, Vec<&'static str>) {
        (
            "sh",
            vec![
                "-c",
                "printf started > started.txt; sleep 2; printf survived > survived.txt",
            ],
        )
    }

    #[cfg(unix)]
    fn create_file_symlink(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_file_symlink(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }
}
