// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn run_tests_rejects_non_test_shell_commands() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "run_tests",
            json!({"command": "echo unsafe > created.txt"}),
        )
        .await
        .expect("policy denial");
    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("deny"));
    assert!(!workspace.join("created.txt").exists());
}

#[tokio::test]
async fn shell_guarded_rejects_non_allowlisted_shell_strings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "shell_guarded",
            json!({"command": "echo unsafe > created.txt"}),
        )
        .await
        .expect("policy denial");

    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("deny"));
    assert!(!workspace.join("created.txt").exists());
}

#[tokio::test]
async fn shell_guarded_runs_allowlisted_commands_without_shell_interpretation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let request = Arc::new(Mutex::new(None));
    let env = Arc::new(RecordingProcessEnv {
        request: request.clone(),
    });
    let session =
        ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(env);

    let result = session
        .execute_skill(
            &registry,
            "shell_guarded",
            json!({"command": "cargo test -p ikaros-core"}),
        )
        .await
        .expect("shell guarded run");

    assert!(result.ok);
    let request = request
        .lock()
        .expect("record request")
        .clone()
        .expect("process request");
    assert!(!request.use_shell);
    assert_eq!(request.command, "cargo");
    assert_eq!(request.args, vec!["test", "-p", "ikaros-core"]);
    assert_eq!(request.cwd, workspace);
}

#[tokio::test]
async fn command_backed_plugin_skill_runs_through_harness() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\ninput=$(cat)\ncase \"$input\" in *abc123*) printf 'raw-ok token=abc123\\n' ;; *) printf 'missing raw input: %s\\n' \"$input\"; exit 2 ;; esac\n",
        "@echo off\r\nfindstr /C:\"abc123\" >nul\r\nif errorlevel 1 (\r\n  echo missing raw input\r\n  exit /b 2\r\n)\r\necho raw-ok token=abc123\r\n",
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "echo"
description = "Echo redacted input."
risk = "safe_read"
input_schema = { type = "object", properties = { message = { type = "string" } } }

[skills.command]
program = "__PROGRAM__"
timeout_ms = 1000
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.echo", "input": {"message": "token=abc123"}}),
        )
        .await
        .expect("plugin run");

    assert!(result.ok);
    assert_eq!(result.output["plugin"], json!("hello"));
    assert_eq!(result.output["skill"], json!("echo"));
    assert_eq!(result.output["status"], json!(0));
    let stdout = result.output["stdout"].as_str().expect("stdout");
    assert!(stdout.contains("raw-ok"));
    assert!(stdout.contains("[REDACTED_SECRET]"));
    assert!(!stdout.contains("abc123"));
}

#[tokio::test]
async fn command_backed_plugin_runs_with_plugin_directory_as_cwd() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\nbasename \"$PWD\"\n",
        "@echo off\r\nfor %%I in (.) do echo %%~nxI\r\n",
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "cwd"
description = "Report plugin cwd."
risk = "safe_read"

[skills.command]
program = "__PROGRAM__"
timeout_ms = 1000
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.cwd", "input": {}}),
        )
        .await
        .expect("plugin run");

    assert_eq!(result.output["status"], json!(0));
    assert_eq!(
        result.output["stdout"].as_str().unwrap_or("").trim(),
        "hello"
    );
}

#[tokio::test]
async fn command_backed_plugin_requires_runtime_permission_for_write_risk() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\nprintf ran > ran.txt\n",
        "@echo off\r\necho ran>ran.txt\r\n",
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "write"
description = "Write plugin marker."
risk = "local_write"

[skills.command]
program = "__PROGRAM__"
timeout_ms = 1000
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let agent = plugin_write_agent();
    let session = ExecutionSession::new_with_agent(workspace, temp.path().join("audit"), &agent);

    let result = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.write", "input": {}}),
        )
        .await
        .expect("policy denial");

    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("deny"));
    assert!(!plugin_dir.join("ran.txt").exists());
}

#[tokio::test]
async fn command_backed_plugin_runs_when_runtime_permission_matches_risk() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\nprintf ran > ran.txt\n",
        "@echo off\r\necho ran>ran.txt\r\n",
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "write"
description = "Write plugin marker."
risk = "local_write"

[[skills.permissions]]
action = "run"
risk = "local_write"
paths = ["skills/hello/ran.txt"]

[skills.command]
program = "__PROGRAM__"
timeout_ms = 1000
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let agent = plugin_write_agent();
    let session = ExecutionSession::new_with_agent(workspace, temp.path().join("audit"), &agent);

    let result = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.write", "input": {}}),
        )
        .await
        .expect("plugin run");

    assert!(result.ok, "{result:?}");
    let marker = fs::read_to_string(plugin_dir.join("ran.txt")).expect("marker");
    assert_eq!(marker.trim_end_matches(['\r', '\n']), "ran");
}

#[tokio::test]
async fn command_backed_plugin_rejects_runtime_permission_path_escape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\nprintf ran > ran.txt\n",
        "@echo off\r\necho ran>ran.txt\r\n",
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "write"
description = "Write plugin marker."
risk = "local_write"

[[skills.permissions]]
action = "run"
risk = "local_write"
paths = ["../outside.txt"]

[skills.command]
program = "__PROGRAM__"
timeout_ms = 1000
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let agent = plugin_write_agent();
    let session = ExecutionSession::new_with_agent(workspace, temp.path().join("audit"), &agent);

    let result = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.write", "input": {}}),
        )
        .await
        .expect("policy denial");

    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("deny"));
    assert!(!plugin_dir.join("ran.txt").exists());
}

#[tokio::test]
async fn command_backed_plugin_rejects_oversized_stdin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\ncat >/dev/null\n",
        "@echo off\r\nmore >nul\r\n",
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "echo"
description = "Echo redacted input."
risk = "safe_read"

[skills.command]
program = "__PROGRAM__"
timeout_ms = 10000
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));
    let oversized = "x".repeat(ikaros_harness::PLUGIN_COMMAND_MAX_STDIN_BYTES + 1);

    let error = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.echo", "input": {"message": oversized}}),
        )
        .await
        .expect_err("oversized stdin should fail");

    assert!(error.to_string().contains("stdin exceeds"));
}

#[tokio::test]
async fn command_backed_plugin_rejects_oversized_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\nprintf '%070000d' 0 | tr 0 x\n",
        concat!(
            "@echo off\r\n",
            "set \"chunk=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"\r\n",
            "for /L %%i in (1,1,600) do <nul set /p \"=%chunk%\"\r\n",
        ),
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "noisy"
description = "Emit too much output."
risk = "safe_read"

[skills.command]
program = "__PROGRAM__"
timeout_ms = 1000
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));

    let error = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.noisy", "input": {}}),
        )
        .await
        .expect_err("oversized output should fail");

    let error_text = error.to_string();
    assert!(
        error_text.contains("exceeded"),
        "unexpected oversized-output error: {error_text}"
    );
    assert!(error_text.contains(&ikaros_harness::PLUGIN_COMMAND_MAX_OUTPUT_BYTES.to_string()));
}

#[tokio::test]
async fn command_backed_plugin_timeout_is_enforced() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\nsleep 1\n",
        "@echo off\r\nping -n 2 127.0.0.1 >nul\r\n",
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "slow"
description = "Sleep too long."
risk = "safe_read"

[skills.command]
program = "__PROGRAM__"
timeout_ms = 1
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));

    let error = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.slow", "input": {}}),
        )
        .await
        .expect_err("timeout should fail");

    assert!(error.to_string().contains("timed out"));
}

#[cfg(unix)]
#[tokio::test]
async fn command_backed_plugin_rejects_symlinked_program_outside_plugin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let outside = temp.path().join("outside.sh");
    fs::write(&outside, "#!/bin/sh\nprintf outside\n").expect("outside");
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o755)).expect("chmod outside");
    std::os::unix::fs::symlink(&outside, plugin_dir.join("runner.sh")).expect("symlink");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "escape"
description = "Escape plugin root."
risk = "safe_read"

[skills.command]
program = "runner.sh"
"#,
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.escape", "input": {}}),
        )
        .await
        .expect("policy denial");

    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("deny"));
}
