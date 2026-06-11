// SPDX-License-Identifier: GPL-3.0-only

use std::{
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct TestHome {
    _temp: tempfile::TempDir,
    pub home: PathBuf,
    pub workspace: PathBuf,
}

impl TestHome {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("workspace");
        Self {
            _temp: temp,
            home,
            workspace,
        }
    }

    pub fn run<I, S>(&self, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_ikaros(&self.home, &self.workspace, args)
    }

    pub fn run_failure<I, S>(&self, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_ikaros_failure(&self.home, &self.workspace, args)
    }

    pub fn init(&self) -> String {
        self.run(["init"])
    }

    pub fn use_offline_mock_config(&self) {
        fs::write(
            self.home.join("config.toml"),
            r#"[model.default]
provider = "mock"
runtime = "harness-agent-loop"
transport = "mock"
model = "mock-ikaros"

[rag]
backend = "jsonl"
embedding_provider = "hash"
embedding_model = "text-embedding-3-small"

[voice.tts]
provider = "mock"
model = "mock-tts"
voice = "default"

[voice.asr]
provider = "mock"
model = "mock-asr"
"#,
        )
        .expect("write offline mock config");
    }
}

fn run_ikaros<I, S>(home: &Path, workspace: &Path, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(env!("CARGO_BIN_EXE_ikaros"))
        .arg("--ikaros-home")
        .arg(home)
        .arg("--workspace")
        .arg(workspace)
        .env_remove("IKAROS_RUN_LIVE_MODEL_TESTS")
        .args(args)
        .output()
        .expect("run ikaros");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "ikaros exited with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout,
        stderr
    );
    assert!(
        !stdout.contains("sk-") && !stderr.contains("sk-"),
        "CLI smoke output must not contain secret-like keys\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    stdout
}

fn run_ikaros_failure<I, S>(home: &Path, workspace: &Path, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(env!("CARGO_BIN_EXE_ikaros"))
        .arg("--ikaros-home")
        .arg(home)
        .arg("--workspace")
        .arg(workspace)
        .env_remove("IKAROS_RUN_LIVE_MODEL_TESTS")
        .args(args)
        .output()
        .expect("run ikaros failure");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        !output.status.success(),
        "ikaros command unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(
        !stdout.contains("sk-") && !stderr.contains("sk-"),
        "CLI smoke output must not contain secret-like keys\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    format!("{stdout}{stderr}")
}

pub fn spawn_ikaros<I, S>(home: &Path, workspace: &Path, args: I) -> Child
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_ikaros"))
        .arg("--ikaros-home")
        .arg(home)
        .arg("--workspace")
        .arg(workspace)
        .env_remove("IKAROS_RUN_LIVE_MODEL_TESTS")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ikaros")
}

pub fn wait_for_child(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            return status;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            panic!("timed out waiting for ikaros child process");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub fn read_child_stderr(child: &mut Child, label: &str) -> String {
    let mut stderr = String::new();
    if let Some(mut stderr_pipe) = child.stderr.take() {
        stderr_pipe
            .read_to_string(&mut stderr)
            .unwrap_or_else(|error| panic!("read {label} stderr: {error}"));
    }
    assert!(
        !stderr.contains("sk-"),
        "{label} stderr must not contain secret-like keys:\n{stderr}",
    );
    stderr
}

pub fn http_get(host_port: &str, target: &str) -> String {
    let mut stream = TcpStream::connect(host_port).expect("connect http server");
    stream
        .write_all(
            format!("GET {target} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .expect("write http request");
    let response = read_response_allowing_reset(&mut stream, "http");
    assert!(
        !response.contains("sk-"),
        "HTTP response must not contain secret-like keys:\n{response}",
    );
    response
}

fn read_response_allowing_reset(stream: &mut TcpStream, label: &str) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error)
                if error.kind() == std::io::ErrorKind::ConnectionReset && !bytes.is_empty() =>
            {
                break;
            }
            Err(error) => panic!("read {label} response: {error}"),
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|error| panic!("{label} response utf8: {error}"))
}

pub fn install_echo_plugin(home: &Path) {
    let plugin_dir = home.join("skills/hello");
    write_echo_plugin(&plugin_dir);
}

pub fn write_echo_plugin(plugin_dir: &Path) {
    let bin_dir = plugin_dir.join("bin");
    fs::create_dir_all(&bin_dir).expect("plugin bin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"name = "hello"
version = "0.1.0"
description = "Smoke plugin."

[[skills]]
name = "echo"
description = "Echo redacted JSON input."
risk = "safe_read"
input_schema = { type = "object", properties = { message = { type = "string" } } }

[skills.command]
program = "bin/echo.sh"
timeout_ms = 1000
"#,
    )
    .expect("plugin manifest");
    let script_path = bin_dir.join("echo.sh");
    let mut script = fs::File::create(&script_path).expect("plugin script");
    writeln!(script, "#!/usr/bin/env sh").expect("script shebang");
    writeln!(script, "cat").expect("script body");
    make_executable(&script_path);
}

pub fn install_smoke_rust_crate(workspace: &Path) {
    fs::create_dir_all(workspace.join("src")).expect("crate src dir");
    fs::write(
        workspace.join("Cargo.toml"),
        r#"[package]
name = "smoke_crate"
version = "0.1.0"
edition = "2021"
"#,
    )
    .expect("crate manifest");
    fs::write(
        workspace.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 { a + b }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds() {
        assert_eq!(add(1, 2), 3);
    }
}
"#,
    )
    .expect("crate source");
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .expect("plugin script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("plugin script permissions");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

pub fn parse_approval_id(output: &str) -> String {
    if let Some(id) = output
        .lines()
        .find_map(|line| line.strip_prefix("approval: "))
        .map(str::to_owned)
    {
        return id;
    }
    output
        .lines()
        .find_map(|line| {
            let line = line.trim();
            line.strip_prefix("\"approval_id\": \"")
                .map(|id| id.trim_end_matches([',', '"']).to_owned())
        })
        .expect("approval id in CLI output")
}
