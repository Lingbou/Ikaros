// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[cfg(target_os = "linux")]
#[tokio::test]
async fn local_execution_env_kills_timed_out_process() {
    let temp = tempfile::tempdir().expect("tempdir");
    let pid_file = temp.path().join("child.pid");
    let request = ProcessRequest::program(
        "sh",
        vec![
            "-c".into(),
            "printf '%s\n' $$ > \"$1\"; while :; do sleep 1; done".into(),
            "ikaros-timeout-test".into(),
            pid_file.to_string_lossy().to_string(),
        ],
        temp.path(),
    )
    .with_timeout_ms(200);

    let error = LocalExecutionEnv
        .run_process(request)
        .await
        .expect_err("command should time out");

    assert!(error.to_string().contains("timed out"));
    let pid = fs::read_to_string(&pid_file)
        .expect("pid file")
        .trim()
        .parse::<u32>()
        .expect("pid");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !PathBuf::from(format!("/proc/{pid}")).exists(),
        "timed-out child process should be killed and reaped"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn local_execution_env_timeout_kills_process_group() {
    let temp = tempfile::tempdir().expect("tempdir");
    let marker = temp.path().join("escaped-grandchild");
    let request = ProcessRequest::program(
        "sh",
        vec![
            "-c".into(),
            format!("(sleep 0.4; touch {}) & sleep 5", marker.display()),
        ],
        temp.path(),
    )
    .with_timeout_ms(100);

    let error = LocalExecutionEnv
        .run_process(request)
        .await
        .expect_err("process group should time out");

    assert!(error.to_string().contains("timed out"));
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    assert!(
        !marker.exists(),
        "timeout must kill descendant processes, not only the shell parent"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn local_execution_env_output_cap_kills_process_group() {
    let temp = tempfile::tempdir().expect("tempdir");
    let marker = temp.path().join("output-cap-grandchild");
    let request = ProcessRequest::program(
        "sh",
        vec![
            "-c".into(),
            format!(
                "printf 'abcdefghijklmnop'; (sleep 0.2; touch {}) & sleep 1",
                marker.display()
            ),
        ],
        temp.path(),
    )
    .with_max_output_bytes(8);

    let error = LocalExecutionEnv
        .run_process(request)
        .await
        .expect_err("output cap should fail the process");

    assert!(error.to_string().contains("stdout exceeded 8 bytes"));
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        !marker.exists(),
        "output cap must kill descendant processes, not only stop reading stdout"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn local_execution_env_does_not_inherit_host_environment_by_default() {
    let temp = tempfile::tempdir().expect("tempdir");
    let request = ProcessRequest::program(
        "sh",
        vec!["-c".into(), "printf '%s' \"${HOME:-missing}\"".into()],
        temp.path(),
    );
    let output = LocalExecutionEnv
        .run_process(request)
        .await
        .expect("process runs");

    assert_eq!(output.stdout, "missing");
}

#[tokio::test]
async fn process_request_passes_only_explicit_environment() {
    let temp = tempfile::tempdir().expect("tempdir");
    let request = if cfg!(windows) {
        ProcessRequest::program(
            "cmd",
            vec!["/C".into(), "echo %IKAROS_ALLOWED_VALUE%".into()],
            temp.path(),
        )
        .with_env("IKAROS_ALLOWED_VALUE", "visible")
    } else {
        ProcessRequest::program(
            "sh",
            vec!["-c".into(), "printf '%s' \"$IKAROS_ALLOWED_VALUE\"".into()],
            temp.path(),
        )
        .with_env("IKAROS_ALLOWED_VALUE", "visible")
    };

    let output = LocalExecutionEnv
        .run_process(request)
        .await
        .expect("process runs");

    assert_eq!(output.stdout.trim(), "visible");
}
