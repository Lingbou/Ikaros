// SPDX-License-Identifier: GPL-3.0-only

use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use crate::support::{TestHome, http_get, read_child_stderr, spawn_ikaros, wait_for_child};

static LOOPBACK_BIND_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn message_webhook_process_accepts_local_http_once() {
    let Some(_bind_lock) = claim_loopback_bind_or_skip("message webhook") else {
        return;
    };
    let env = TestHome::new();
    env.init();

    let mut child = spawn_ikaros(
        &env.home,
        &env.workspace,
        ["message", "webhook", "--once", "--port", "0"],
    );
    let message_url = read_startup_url(&mut child, "webhook", "message_webhook: ");
    let endpoint = message_url.expect("message webhook endpoint");
    let host_port = endpoint
        .strip_prefix("http://")
        .and_then(|value| value.strip_suffix("/message"))
        .expect("loopback message endpoint");

    let body = r#"{"content":"hello webhook token=abc123","kind":"task","source":"smoke","profile":"plan"}"#;
    let mut stream = TcpStream::connect(host_port).expect("connect webhook");
    stream
        .write_all(
            format!(
                "POST /message HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .expect("write webhook request");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish webhook request");
    let response = read_response_allowing_reset(&mut stream, "webhook");
    assert!(response.contains("HTTP/1.1 202 Accepted"));
    assert!(response.contains("\"ok\": true"));
    assert!(response.contains("[REDACTED_SECRET]"));
    assert!(!response.contains("abc123"));

    let status = wait_for_child(&mut child, Duration::from_secs(5));
    assert!(status.success(), "webhook process exited with {status}");
    let stderr = read_child_stderr(&mut child, "webhook");
    assert!(stderr.trim().is_empty(), "webhook stderr:\n{stderr}");

    let inbox = fs::read_to_string(env.home.join("gateway/inbox.jsonl")).expect("gateway inbox");
    assert!(inbox.contains("hello webhook token=[REDACTED_SECRET]"));
    assert!(inbox.contains("\"source\":\"smoke\""));
    assert!(inbox.contains("\"kind\":\"Task\""));
    assert!(inbox.contains("\"agent\":\"plan\""));
    assert!(!inbox.contains("abc123"));
}

#[test]
fn body_dashboard_server_process_serves_frame_json_once() {
    let Some(_bind_lock) = claim_loopback_bind_or_skip("body dashboard server") else {
        return;
    };
    let env = TestHome::new();
    env.init();

    let mut child = spawn_ikaros(
        &env.home,
        &env.workspace,
        [
            "body",
            "serve",
            "--once",
            "--port",
            "0",
            "--events",
            "2",
            "--refresh-seconds",
            "1",
        ],
    );
    let frame_url = read_startup_url(&mut child, "body server", "frame_json: ");
    let endpoint = frame_url.expect("frame json endpoint");
    let host_port = endpoint
        .strip_prefix("http://")
        .and_then(|value| value.strip_suffix("/frame.json"))
        .expect("loopback frame endpoint");

    let response = http_get(host_port, "/frame.json");
    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(response.contains("Content-Type: application/json"));
    assert!(response.contains("\"body\": \"Web\""));
    assert!(response.contains("\"persona_name\": \"Ikaros\""));
    assert!(response.contains("\"events\": []"));

    let status = wait_for_child(&mut child, Duration::from_secs(5));
    assert!(status.success(), "body server process exited with {status}");
    let stderr = read_child_stderr(&mut child, "body server");
    assert!(stderr.trim().is_empty(), "body server stderr:\n{stderr}");
}

fn claim_loopback_bind_or_skip(label: &str) -> Option<MutexGuard<'static, ()>> {
    let guard = LOOPBACK_BIND_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => {
            drop(listener);
            Some(guard)
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping {label} loopback smoke: binding 127.0.0.1:0 is not permitted here");
            None
        }
        Err(error) => panic!("failed loopback bind preflight for {label}: {error}"),
    }
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

fn read_startup_url(child: &mut std::process::Child, label: &str, prefix: &str) -> Option<String> {
    let stdout = child
        .stdout
        .take()
        .unwrap_or_else(|| panic!("{label} stdout"));
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    for _ in 0..8 {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .unwrap_or_else(|error| panic!("{label} startup line: {error}"));
        if bytes == 0 {
            let stderr = read_child_stderr(child, label);
            panic!("{label} exited before printing {prefix:?}; stderr:\n{stderr}");
        }
        if let Some(url) = line.trim().strip_prefix(prefix) {
            std::thread::spawn(move || {
                let mut sink = String::new();
                let _ = reader.read_to_string(&mut sink);
            });
            return Some(url.to_owned());
        }
    }
    None
}

#[test]
fn worker_once_processes_local_message_and_schedule_queues() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let sent = env.run([
        "message",
        "send",
        "--kind",
        "task",
        "--profile",
        "build",
        "summarize worker message smoke",
    ]);
    assert!(sent.contains("enqueued:"));
    assert!(sent.contains("\"status\": \"Pending\""));

    let message_worker = env.run([
        "message",
        "worker",
        "--once",
        "--limit",
        "1",
        "--interval-seconds",
        "1",
    ]);
    assert!(message_worker.contains("message_worker: started"));
    assert!(message_worker.contains("\"kind\": \"gateway_worker_tick\""));
    assert!(message_worker.contains("\"pending\": 1"));
    assert!(message_worker.contains("\"drained\": 1"));
    assert!(message_worker.contains("\"status\": \"Processed\""));
    assert!(message_worker.contains("\"kind\": \"task\""));

    let inbox = fs::read_to_string(env.home.join("gateway/inbox.jsonl")).expect("gateway inbox");
    assert!(inbox.contains("\"status\":\"Processed\""));
    assert!(inbox.contains("gateway task step(s)"));
    let outbox = fs::read_to_string(env.home.join("gateway/outbox.jsonl")).expect("gateway outbox");
    assert!(outbox.contains("\"kind\":\"task_report\""));
    assert!(outbox.contains("\\\"state\\\": \\\"Completed\\\""));

    let scheduled = env.run([
        "schedule",
        "add",
        "--profile",
        "build",
        "summarize worker schedule smoke",
    ]);
    assert!(scheduled.contains("scheduled:"));
    assert!(scheduled.contains("\"enabled\": true"));

    let schedule_worker = env.run([
        "schedule",
        "worker",
        "--once",
        "--limit",
        "1",
        "--interval-seconds",
        "1",
    ]);
    assert!(schedule_worker.contains("schedule_worker: started"));
    assert!(schedule_worker.contains("\"kind\": \"schedule_worker_tick\""));
    assert!(schedule_worker.contains("\"due\": 1"));
    assert!(schedule_worker.contains("\"ran\": 1"));
    assert!(schedule_worker.contains("\"task_state\": \"Completed\""));
    assert!(schedule_worker.contains("\"target\": \"local_file\""));

    let schedules =
        fs::read_to_string(env.home.join("automation/schedules.jsonl")).expect("schedule store");
    assert!(schedules.contains("\"enabled\":false"));
    assert!(schedules.contains("\"last_status\":\"Completed\""));
    assert!(schedules.contains("scheduled step(s)"));
    assert!(env.home.join("automation/deliveries").exists());
}
