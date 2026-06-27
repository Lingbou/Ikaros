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
    assert!(inbox.contains("\"safe_tools\":true"));
    assert!(!inbox.contains("abc123"));
}

#[test]
fn message_webhook_hmac_secret_rejects_unsigned_request_without_enqueueing() {
    let Some(_bind_lock) = claim_loopback_bind_or_skip("message webhook hmac") else {
        return;
    };
    let env = TestHome::new();
    env.init();

    let mut child = spawn_ikaros(
        &env.home,
        &env.workspace,
        [
            "message",
            "webhook",
            "--once",
            "--port",
            "0",
            "--hmac-secret",
            "webhook-secret",
        ],
    );
    let message_url = read_startup_url(&mut child, "webhook hmac", "message_webhook: ");
    let endpoint = message_url.expect("message webhook endpoint");
    let host_port = endpoint
        .strip_prefix("http://")
        .and_then(|value| value.strip_suffix("/message"))
        .expect("loopback message endpoint");

    let body = r#"{"content":"signed webhook token=abc123","kind":"task","source":"signed","profile":"build"}"#;
    let mut stream = TcpStream::connect(host_port).expect("connect webhook");
    stream
        .write_all(
            format!(
                "POST /message HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .expect("write unsigned webhook request");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish unsigned webhook request");
    let response = read_response_allowing_reset(&mut stream, "webhook hmac");
    assert!(response.contains("HTTP/1.1 401 Unauthorized"));
    assert!(response.contains("missing webhook signature"));
    assert!(!response.contains("abc123"));

    let status = wait_for_child(&mut child, Duration::from_secs(5));
    assert!(status.success(), "webhook process exited with {status}");
    let stderr = read_child_stderr(&mut child, "webhook hmac");
    assert!(stderr.trim().is_empty(), "webhook stderr:\n{stderr}");
    assert!(!env.home.join("gateway/inbox.jsonl").exists());
}

#[test]
fn message_webhook_hmac_secret_accepts_valid_signature() {
    let Some(_bind_lock) = claim_loopback_bind_or_skip("message webhook hmac signed") else {
        return;
    };
    let env = TestHome::new();
    env.init();

    let mut child = spawn_ikaros(
        &env.home,
        &env.workspace,
        [
            "message",
            "webhook",
            "--once",
            "--port",
            "0",
            "--hmac-secret",
            "webhook-secret",
        ],
    );
    let message_url = read_startup_url(&mut child, "webhook hmac signed", "message_webhook: ");
    let endpoint = message_url.expect("message webhook endpoint");
    let host_port = endpoint
        .strip_prefix("http://")
        .and_then(|value| value.strip_suffix("/message"))
        .expect("loopback message endpoint");

    let body = r#"{"content":"signed webhook token=abc123","kind":"task","source":"signed","profile":"build"}"#;
    let mut stream = TcpStream::connect(host_port).expect("connect webhook");
    stream
        .write_all(
            format!(
                "POST /message HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nX-Ikaros-Signature: sha256=c1bcc80fac6f0fd1920ef5cca3dcb3165381b2c01e8a8de87d7579f7809b329b\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .expect("write signed webhook request");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish signed webhook request");
    let response = read_response_allowing_reset(&mut stream, "webhook hmac signed");
    assert!(response.contains("HTTP/1.1 202 Accepted"));
    assert!(response.contains("\"ok\": true"));
    assert!(response.contains("[REDACTED_SECRET]"));
    assert!(!response.contains("abc123"));

    let status = wait_for_child(&mut child, Duration::from_secs(5));
    assert!(status.success(), "webhook process exited with {status}");
    let stderr = read_child_stderr(&mut child, "webhook hmac signed");
    assert!(stderr.trim().is_empty(), "webhook stderr:\n{stderr}");

    let inbox = fs::read_to_string(env.home.join("gateway/inbox.jsonl")).expect("gateway inbox");
    assert!(inbox.contains("signed webhook token=[REDACTED_SECRET]"));
    assert!(inbox.contains("\"source\":\"signed\""));
    assert!(inbox.contains("\"kind\":\"Task\""));
    assert!(inbox.contains("\"agent\":\"build\""));
    assert!(!inbox.contains("abc123"));
}

#[test]
fn message_webhook_hmac_secret_rejects_invalid_signature_without_enqueueing() {
    let Some(_bind_lock) = claim_loopback_bind_or_skip("message webhook hmac invalid") else {
        return;
    };
    let env = TestHome::new();
    env.init();

    let mut child = spawn_ikaros(
        &env.home,
        &env.workspace,
        [
            "message",
            "webhook",
            "--once",
            "--port",
            "0",
            "--hmac-secret",
            "webhook-secret",
        ],
    );
    let message_url = read_startup_url(&mut child, "webhook hmac invalid", "message_webhook: ");
    let endpoint = message_url.expect("message webhook endpoint");
    let host_port = endpoint
        .strip_prefix("http://")
        .and_then(|value| value.strip_suffix("/message"))
        .expect("loopback message endpoint");

    let body = r#"{"content":"signed webhook token=abc123","kind":"task","source":"signed","profile":"build"}"#;
    let mut stream = TcpStream::connect(host_port).expect("connect webhook");
    stream
        .write_all(
            format!(
                "POST /message HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nX-Ikaros-Signature: sha256=01bcc80fac6f0fd1920ef5cca3dcb3165381b2c01e8a8de87d7579f7809b329b\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .expect("write invalid signed webhook request");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish invalid signed webhook request");
    let response = read_response_allowing_reset(&mut stream, "webhook hmac invalid");
    assert!(response.contains("HTTP/1.1 401 Unauthorized"));
    assert!(response.contains("invalid webhook signature"));
    assert!(!response.contains("abc123"));

    let status = wait_for_child(&mut child, Duration::from_secs(5));
    assert!(status.success(), "webhook process exited with {status}");
    let stderr = read_child_stderr(&mut child, "webhook hmac invalid");
    assert!(stderr.trim().is_empty(), "webhook stderr:\n{stderr}");
    assert!(!env.home.join("gateway/inbox.jsonl").exists());
}

#[test]
fn message_webhook_acl_rejects_disallowed_peer_without_enqueueing() {
    let Some(_bind_lock) = claim_loopback_bind_or_skip("message webhook acl") else {
        return;
    };
    let env = TestHome::new();
    env.init();

    let mut child = spawn_ikaros(
        &env.home,
        &env.workspace,
        [
            "message",
            "webhook",
            "--once",
            "--port",
            "0",
            "--allow-source",
            "telegram",
            "--allow-peer",
            "alice",
        ],
    );
    let message_url = read_startup_url(&mut child, "webhook acl", "message_webhook: ");
    let endpoint = message_url.expect("message webhook endpoint");
    let host_port = endpoint
        .strip_prefix("http://")
        .and_then(|value| value.strip_suffix("/message"))
        .expect("loopback message endpoint");

    let body =
        r#"{"content":"blocked peer token=abc123","kind":"chat","source":"telegram","peer":"bob"}"#;
    let mut stream = TcpStream::connect(host_port).expect("connect webhook");
    stream
        .write_all(
            format!(
                "POST /message HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .expect("write acl webhook request");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish acl webhook request");
    let response = read_response_allowing_reset(&mut stream, "webhook acl");
    assert!(response.contains("HTTP/1.1 403 Forbidden"));
    assert!(response.contains("webhook ACL rejected peer"));
    assert!(!response.contains("abc123"));

    let status = wait_for_child(&mut child, Duration::from_secs(5));
    assert!(status.success(), "webhook process exited with {status}");
    let stderr = read_child_stderr(&mut child, "webhook acl");
    assert!(stderr.trim().is_empty(), "webhook stderr:\n{stderr}");
    assert!(!env.home.join("gateway/inbox.jsonl").exists());
}

#[test]
fn message_webhook_acl_accepts_allowed_peer() {
    let Some(_bind_lock) = claim_loopback_bind_or_skip("message webhook acl allowed") else {
        return;
    };
    let env = TestHome::new();
    env.init();

    let mut child = spawn_ikaros(
        &env.home,
        &env.workspace,
        [
            "message",
            "webhook",
            "--once",
            "--port",
            "0",
            "--allow-source",
            "telegram",
            "--allow-peer",
            "alice",
        ],
    );
    let message_url = read_startup_url(&mut child, "webhook acl allowed", "message_webhook: ");
    let endpoint = message_url.expect("message webhook endpoint");
    let host_port = endpoint
        .strip_prefix("http://")
        .and_then(|value| value.strip_suffix("/message"))
        .expect("loopback message endpoint");

    let body = r#"{"content":"allowed peer token=abc123","kind":"chat","source":"telegram","peer":"alice","thread":"chat-1","message_id":"msg-1"}"#;
    let mut stream = TcpStream::connect(host_port).expect("connect webhook");
    stream
        .write_all(
            format!(
                "POST /message HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .expect("write allowed acl webhook request");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish allowed acl webhook request");
    let response = read_response_allowing_reset(&mut stream, "webhook acl allowed");
    assert!(response.contains("HTTP/1.1 202 Accepted"));
    assert!(response.contains("\"ok\": true"));
    assert!(!response.contains("abc123"));

    let status = wait_for_child(&mut child, Duration::from_secs(5));
    assert!(status.success(), "webhook process exited with {status}");
    let stderr = read_child_stderr(&mut child, "webhook acl allowed");
    assert!(stderr.trim().is_empty(), "webhook stderr:\n{stderr}");

    let inbox = fs::read_to_string(env.home.join("gateway/inbox.jsonl")).expect("gateway inbox");
    assert!(inbox.contains("allowed peer token=[REDACTED_SECRET]"));
    assert!(inbox.contains("\"source\":\"telegram\""));
    assert!(inbox.contains("\"peer\":\"alice\""));
    assert!(inbox.contains("\"thread\":\"chat-1\""));
    assert!(inbox.contains("\"message_id\":\"msg-1\""));
    assert!(!inbox.contains("abc123"));
}

#[test]
fn message_webhook_require_pairing_accepts_code_then_future_peer_messages() {
    let Some(_bind_lock) = claim_loopback_bind_or_skip("message webhook pairing") else {
        return;
    };
    let env = TestHome::new();
    env.init();
    let created = env.run([
        "message",
        "pairing",
        "create",
        "--source",
        "telegram",
        "--account",
        "bot",
        "--peer",
        "alice",
    ]);
    let pairing_code = output_line_value(&created, "message_pairing_code: ");
    let listed = env.run(["message", "pairing", "list"]);
    assert!(listed.contains("[REDACTED_PAIRING_CODE]"));
    assert!(!listed.contains(&pairing_code));

    let mut first_child = spawn_ikaros(
        &env.home,
        &env.workspace,
        [
            "message",
            "webhook",
            "--once",
            "--port",
            "0",
            "--require-pairing",
        ],
    );
    let first_url = read_startup_url(&mut first_child, "webhook pairing", "message_webhook: ")
        .expect("message webhook endpoint");
    let first_host_port = webhook_host_port(&first_url);
    let first_body = format!(
        r#"{{"content":"pair me token=abc123","kind":"chat","source":"telegram","account":"bot","peer":"alice","pairing_code":"{pairing_code}"}}"#
    );
    let first_response =
        post_json_to_webhook(&first_host_port, "webhook pairing first", &first_body);
    assert!(first_response.contains("HTTP/1.1 202 Accepted"));
    assert!(first_response.contains("\"ok\": true"));
    assert!(!first_response.contains("abc123"));
    let first_status = wait_for_child(&mut first_child, Duration::from_secs(5));
    assert!(
        first_status.success(),
        "webhook process exited with {first_status}"
    );

    let mut second_child = spawn_ikaros(
        &env.home,
        &env.workspace,
        [
            "message",
            "webhook",
            "--once",
            "--port",
            "0",
            "--require-pairing",
        ],
    );
    let second_url = read_startup_url(
        &mut second_child,
        "webhook pairing second",
        "message_webhook: ",
    )
    .expect("message webhook endpoint");
    let second_host_port = webhook_host_port(&second_url);
    let second_body = r#"{"content":"already paired token=abc123","kind":"chat","source":"telegram","account":"bot","peer":"alice"}"#;
    let second_response =
        post_json_to_webhook(&second_host_port, "webhook pairing second", second_body);
    assert!(second_response.contains("HTTP/1.1 202 Accepted"));
    assert!(second_response.contains("\"ok\": true"));
    assert!(!second_response.contains("abc123"));
    let second_status = wait_for_child(&mut second_child, Duration::from_secs(5));
    assert!(
        second_status.success(),
        "webhook process exited with {second_status}"
    );

    let pairings =
        fs::read_to_string(env.home.join("gateway/pairings.jsonl")).expect("gateway pairings");
    assert!(pairings.contains("\"status\":\"Paired\""));
    assert!(pairings.contains("\"peer\":\"alice\""));
    assert!(!pairings.contains("abc123"));
    let inbox = fs::read_to_string(env.home.join("gateway/inbox.jsonl")).expect("gateway inbox");
    assert!(inbox.contains("pair me token=[REDACTED_SECRET]"));
    assert!(inbox.contains("already paired token=[REDACTED_SECRET]"));
    assert!(!inbox.contains("abc123"));
}

#[test]
fn message_webhook_require_pairing_rejects_unpaired_peer_without_enqueueing() {
    let Some(_bind_lock) = claim_loopback_bind_or_skip("message webhook pairing reject") else {
        return;
    };
    let env = TestHome::new();
    env.init();

    let mut child = spawn_ikaros(
        &env.home,
        &env.workspace,
        [
            "message",
            "webhook",
            "--once",
            "--port",
            "0",
            "--require-pairing",
        ],
    );
    let endpoint = read_startup_url(&mut child, "webhook pairing reject", "message_webhook: ")
        .expect("message webhook endpoint");
    let host_port = webhook_host_port(&endpoint);
    let body =
        r#"{"content":"unpaired token=abc123","kind":"chat","source":"telegram","peer":"alice"}"#;
    let response = post_json_to_webhook(&host_port, "webhook pairing reject", body);
    assert!(response.contains("HTTP/1.1 403 Forbidden"));
    assert!(response.contains("webhook pairing required"));
    assert!(!response.contains("abc123"));

    let status = wait_for_child(&mut child, Duration::from_secs(5));
    assert!(status.success(), "webhook process exited with {status}");
    let stderr = read_child_stderr(&mut child, "webhook pairing reject");
    assert!(stderr.trim().is_empty(), "webhook stderr:\n{stderr}");
    assert!(!env.home.join("gateway/inbox.jsonl").exists());
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

fn post_json_to_webhook(host_port: &str, label: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(host_port).unwrap_or_else(|error| {
        panic!("connect {label}: {error}");
    });
    stream
        .write_all(
            format!(
                "POST /message HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .unwrap_or_else(|error| panic!("write {label} request: {error}"));
    stream
        .shutdown(Shutdown::Write)
        .unwrap_or_else(|error| panic!("finish {label} request: {error}"));
    read_response_allowing_reset(&mut stream, label)
}

fn webhook_host_port(endpoint: &str) -> String {
    endpoint
        .strip_prefix("http://")
        .and_then(|value| value.strip_suffix("/message"))
        .unwrap_or_else(|| panic!("loopback message endpoint: {endpoint}"))
        .to_owned()
}

fn output_line_value(output: &str, prefix: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing output line {prefix:?} in:\n{output}"))
        .to_owned()
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
    assert!(message_worker.contains("message_worker_lock:"));
    assert!(message_worker.contains("message_worker_events:"));
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
    assert!(!env.home.join("gateway/message-worker.lock").exists());
    let worker_events = fs::read_to_string(env.home.join("gateway/message-worker-events.jsonl"))
        .expect("worker events");
    assert!(worker_events.contains("\"event\":\"started\""));
    assert!(worker_events.contains("\"event\":\"stopped\""));
    assert!(worker_events.contains("\"status\":\"completed\""));
    assert!(!worker_events.contains("sk-"));

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

#[test]
fn message_worker_refuses_existing_gateway_worker_lock() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    fs::create_dir_all(env.home.join("gateway")).expect("gateway dir");
    fs::write(
        env.home.join("gateway/message-worker.lock"),
        format!("pid={}\nowner=worker-token=abc123\n", std::process::id()),
    )
    .expect("seed worker lock");

    let output = env.run_failure([
        "message",
        "worker",
        "--once",
        "--limit",
        "1",
        "--interval-seconds",
        "1",
    ]);

    assert!(output.contains("message worker already running"));
    assert!(output.contains("message-worker.lock"));
    assert!(output.contains("[REDACTED_SECRET]"));
    assert!(!output.contains("abc123"));
}

#[test]
fn message_worker_recovers_stale_gateway_worker_lock() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    fs::create_dir_all(env.home.join("gateway")).expect("gateway dir");
    fs::write(
        env.home.join("gateway/message-worker.lock"),
        "pid=999999999\nstarted_at=2026-06-23T00:00:00Z\n",
    )
    .expect("seed stale worker lock");

    let output = env.run([
        "message",
        "worker",
        "--once",
        "--limit",
        "1",
        "--interval-seconds",
        "1",
    ]);

    assert!(output.contains("message_worker_lock_recovered:"));
    assert!(output.contains("stale=true"));
    assert!(output.contains("message_worker: started"));
    assert!(output.contains("\"pending\": 0"));
    assert!(!env.home.join("gateway/message-worker.lock").exists());
    let has_stale_archive = fs::read_dir(env.home.join("gateway"))
        .expect("gateway dir")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("message-worker.lock.stale.")
        });
    assert!(has_stale_archive);
}

#[test]
fn message_worker_stop_request_is_redacted_visible_and_consumed() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let requested = env.run([
        "message",
        "worker-stop",
        "--reason",
        "maintenance token=abc123",
    ]);
    assert!(requested.contains("message_worker_stop: requested=true"));
    assert!(requested.contains("message-worker.stop"));
    assert!(requested.contains("maintenance token=[REDACTED_SECRET]"));
    assert!(!requested.contains("abc123"));
    let stop_file =
        fs::read_to_string(env.home.join("gateway/message-worker.stop")).expect("stop file");
    assert!(stop_file.contains("maintenance token=[REDACTED_SECRET]"));
    assert!(!stop_file.contains("abc123"));

    let status = env.run(["message", "status"]);
    assert!(status.contains("gateway_worker_stop: requested=true"));
    assert!(status.contains("maintenance token=[REDACTED_SECRET]"));
    assert!(!status.contains("abc123"));

    let worker = env.run([
        "message",
        "worker",
        "--once",
        "--limit",
        "1",
        "--interval-seconds",
        "1",
    ]);
    assert!(worker.contains("message_worker: stop_requested"));
    assert!(!env.home.join("gateway/message-worker.stop").exists());
    let events = fs::read_to_string(env.home.join("gateway/message-worker-events.jsonl"))
        .expect("worker events");
    assert!(events.contains("\"status\":\"stopped\""));
    assert!(events.contains("stop requested"));
}

#[test]
fn message_cancel_marks_pending_message_cancelled_and_skips_worker_delivery() {
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
        "cancel this queued message",
    ]);
    assert!(sent.contains("enqueued:"));
    let message_id = first_gateway_message_id(&env);

    let cancelled = env.run([
        "message",
        "cancel",
        &message_id,
        "--reason",
        "operator token=abc123",
    ]);
    assert!(cancelled.contains("message_cancelled: true"));
    assert!(cancelled.contains("status=Cancelled"));
    assert!(cancelled.contains("operator token=[REDACTED_SECRET]"));
    assert!(!cancelled.contains("abc123"));

    let status = env.run(["message", "status"]);
    assert!(status.contains("gateway_cancelled: 1"));
    assert!(status.contains("gateway_pending: 0"));

    let worker = env.run([
        "message",
        "worker",
        "--once",
        "--limit",
        "1",
        "--interval-seconds",
        "1",
    ]);
    assert!(worker.contains("\"pending\": 0"));
    assert!(!env.home.join("gateway/outbox.jsonl").exists());
}

#[test]
fn message_daemon_start_status_stop_and_restart_use_worker_state() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let started = env.run([
        "message",
        "daemon",
        "start",
        "--limit",
        "1",
        "--interval-seconds",
        "1",
    ]);
    assert!(started.contains("message_daemon: started"));
    assert!(started.contains("message_daemon_pid:"));
    assert!(started.contains("message_daemon_log:"));
    assert!(started.contains("message-worker.lock"));

    let status = env.run(["message", "daemon", "status"]);
    assert!(status.contains("message_daemon_status: running"));
    assert!(status.contains("gateway_worker_lock: present=true"));
    assert!(status.contains("gateway_worker_forensics:"));

    let stopped = env.run([
        "message",
        "daemon",
        "stop",
        "--reason",
        "daemon maintenance token=abc123",
    ]);
    assert!(stopped.contains("message_daemon_stop: requested=true"));
    assert!(stopped.contains("daemon maintenance token=[REDACTED_SECRET]"));
    assert!(!stopped.contains("abc123"));

    let stopped_status = wait_until_daemon_status(&env, "message_daemon_status: stopped");
    assert!(stopped_status.contains("gateway_worker_stop: requested=false"));
    assert!(stopped_status.contains("latest_status=stopped"));

    let restarted = env.run([
        "message",
        "daemon",
        "restart",
        "--limit",
        "1",
        "--interval-seconds",
        "1",
    ]);
    assert!(restarted.contains("message_daemon_restart: starting=true"));
    assert!(restarted.contains("message_daemon: started"));
    assert!(restarted.contains("message_daemon_pid:"));

    let running_again = env.run(["message", "daemon", "status"]);
    assert!(running_again.contains("message_daemon_status: running"));

    let stopped_again = env.run(["message", "daemon", "stop"]);
    assert!(stopped_again.contains("message_daemon_stop: requested=true"));
    let final_status = wait_until_daemon_status(&env, "message_daemon_status: stopped");
    assert!(final_status.contains("latest_status=stopped"));

    let events = fs::read_to_string(env.home.join("gateway/message-worker-events.jsonl"))
        .expect("daemon worker events");
    assert!(events.contains("\"event\":\"started\""));
    assert!(events.contains("\"status\":\"stopped\""));
    assert!(!events.contains("abc123"));
}

fn wait_until_daemon_status(env: &TestHome, expected: &str) -> String {
    let mut last = String::new();
    for _ in 0..20 {
        last = env.run(["message", "daemon", "status"]);
        if last.contains(expected) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {expected}; last status:\n{last}");
}

fn first_gateway_message_id(env: &TestHome) -> String {
    let inbox = fs::read_to_string(env.home.join("gateway/inbox.jsonl")).expect("gateway inbox");
    let line = inbox.lines().next().expect("gateway message");
    let value: serde_json::Value = serde_json::from_str(line).expect("gateway message json");
    value["id"].as_str().expect("message id").to_owned()
}
