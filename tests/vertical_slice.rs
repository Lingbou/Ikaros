use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;

#[test]
fn cli_denies_noninteractive_tools_and_restores_the_session() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || serve_two_completions(listener));
    let binary = env!("CARGO_BIN_EXE_ikaros");
    let home_text = path_text(home.path());
    let workspace_text = path_text(workspace.path());
    let base_url = format!("http://{address}/v1");

    let init = isolated_command(binary)
        .args([
            "--home",
            home_text.as_str(),
            "--workspace",
            workspace_text.as_str(),
            "init",
            "--base-url",
            base_url.as_str(),
            "--model",
            "test-model",
        ])
        .output()
        .expect("init process");

    let chat = isolated_command(binary)
        .args([
            "--home",
            home_text.as_str(),
            "--workspace",
            workspace_text.as_str(),
            "chat",
            "write the smoke file",
        ])
        .output()
        .expect("chat process");

    let requests = server.join().expect("server thread").expect("server");
    assert_success("init", &init);
    assert_success("chat", &chat);
    let stdout = String::from_utf8_lossy(&chat.stdout);
    let stderr = String::from_utf8_lossy(&chat.stderr);
    assert!(stdout.contains("tool denial observed"));
    assert!(stderr.contains("decision: denied"));
    assert!(!stdout.contains('\u{1b}'));
    assert!(!stdout.contains('\u{7}'));
    assert!(!stderr.contains('\u{1b}'));
    assert!(!workspace.path().join("result.txt").exists());
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains(r#""role":"tool""#));
    assert!(requests[1].contains("Tool call denied by the user."));

    let sessions = isolated_command(binary)
        .args([
            "--home",
            home_text.as_str(),
            "--workspace",
            workspace_text.as_str(),
            "sessions",
        ])
        .output()
        .expect("sessions process");
    assert_success("sessions", &sessions);
    assert!(String::from_utf8_lossy(&sessions.stdout).contains("messages=4"));
}

fn serve_two_completions(listener: TcpListener) -> Result<Vec<String>, String> {
    let responses = [
        r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-\u001b[31m-smoke","type":"function","function":{"name":"write_file\u001b[31m","arguments":"{\"path\":\"result.txt\",\"content\":\"from tool\"}"}}]}}]}"#,
        r#"{"choices":[{"message":{"content":"tool denial observed\u001b]0;owned\u0007","tool_calls":[]}}]}"#,
    ];
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut requests = Vec::new();
    for response in responses {
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err("timed out waiting for provider request".to_owned());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(format!("provider accept failed: {error}")),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| error.to_string())?;
        requests.push(read_request(&mut stream).map_err(|error| error.to_string())?);
        write_response(&mut stream, response).map_err(|error| error.to_string())?;
    }
    Ok(requests)
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut head = String::new();
    let mut content_length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.is_empty() || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        head.push_str(&line);
    }
    head.push_str("\r\n");
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body)?;
    head.push_str(&String::from_utf8_lossy(&body));
    Ok(head)
}

fn write_response(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn path_text(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

fn isolated_command(binary: &str) -> Command {
    let mut command = Command::new(binary);
    for name in [
        "IKAROS_BASE_URL",
        "IKAROS_MODEL",
        "IKAROS_API_KEY",
        "OPENAI_API_KEY",
    ] {
        command.env_remove(name);
    }
    command
}

fn assert_success(name: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{name} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
