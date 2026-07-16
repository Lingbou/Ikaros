use crate::domain::{AssistantReply, Message, Role, ToolCall};
use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::time::Duration;

const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;

pub(crate) trait ModelProvider: Send + Sync {
    async fn complete(&self, messages: &[Message], tools: &[Value]) -> Result<AssistantReply>;
}

#[derive(Debug, Clone)]
pub struct OpenAiProvider {
    client: Client,
    endpoint: String,
    model: String,
    api_key: Option<String>,
}

impl OpenAiProvider {
    pub fn new(base_url: &str, model: String, api_key: Option<String>) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent("ikaros/0.1")
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            client,
            endpoint: chat_completions_endpoint(base_url),
            model,
            api_key,
        })
    }
}

impl ModelProvider for OpenAiProvider {
    async fn complete(&self, messages: &[Message], tools: &[Value]) -> Result<AssistantReply> {
        let mut body = Map::new();
        body.insert("model".to_owned(), Value::String(self.model.clone()));
        body.insert(
            "messages".to_owned(),
            Value::Array(messages.iter().map(message_to_wire).collect()),
        );
        if !tools.is_empty() {
            body.insert("tools".to_owned(), Value::Array(tools.to_vec()));
        }

        let mut request = self.client.post(&self.endpoint).json(&body);
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }
        let mut response = request
            .send()
            .await
            .with_context(|| format!("provider request failed: {}", self.endpoint))?;
        let status = response.status();
        let text = read_response_text(&mut response).await?;
        if !status.is_success() {
            bail!(
                "provider returned HTTP {}: {}",
                status.as_u16(),
                truncate(&text, 4_000)
            );
        }
        parse_response(&text)
    }
}

async fn read_response_text(response: &mut reqwest::Response) -> Result<String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES as u64)
    {
        bail!("provider response exceeded the {MAX_PROVIDER_RESPONSE_BYTES}-byte limit");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("failed to read provider response")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_PROVIDER_RESPONSE_BYTES {
            bail!("provider response exceeded the {MAX_PROVIDER_RESPONSE_BYTES}-byte limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).context("provider response was not valid UTF-8")
}

fn chat_completions_endpoint(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_owned()
    } else {
        format!("{base}/chat/completions")
    }
}

fn message_to_wire(message: &Message) -> Value {
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut value = Map::new();
    value.insert("role".to_owned(), Value::String(role.to_owned()));
    match &message.content {
        Some(content) => {
            value.insert("content".to_owned(), Value::String(content.clone()));
        }
        None if message.role == Role::Assistant => {
            value.insert("content".to_owned(), Value::Null);
        }
        None => {}
    }
    if !message.tool_calls.is_empty() {
        value.insert(
            "tool_calls".to_owned(),
            Value::Array(
                message
                    .tool_calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": call.arguments.to_string(),
                            }
                        })
                    })
                    .collect(),
            ),
        );
    }
    if let Some(tool_call_id) = &message.tool_call_id {
        value.insert(
            "tool_call_id".to_owned(),
            Value::String(tool_call_id.clone()),
        );
    }
    Value::Object(value)
}

fn parse_response(text: &str) -> Result<AssistantReply> {
    let response: ChatResponse =
        serde_json::from_str(text).context("provider returned invalid JSON")?;
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("provider returned no choices"))?;
    let mut tool_calls = Vec::with_capacity(choice.message.tool_calls.len());
    for call in choice.message.tool_calls {
        if call.kind != "function" {
            bail!("unsupported provider tool call type: {}", call.kind);
        }
        let raw_arguments = call.function.arguments.trim();
        let arguments = if raw_arguments.is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(raw_arguments).with_context(|| {
                format!(
                    "provider returned invalid arguments for tool {}",
                    call.function.name
                )
            })?
        };
        tool_calls.push(ToolCall {
            id: call.id,
            name: call.function.name,
            arguments,
        });
    }
    if choice.message.content.is_none() && tool_calls.is_empty() {
        bail!("provider returned neither text nor tool calls");
    }
    Ok(AssistantReply {
        content: choice.message.content,
        tool_calls,
    })
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireToolCall>,
}

#[derive(Debug, Deserialize)]
struct WireToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: WireFunction,
}

#[derive(Debug, Deserialize)]
struct WireFunction {
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::{
        ModelProvider, OpenAiProvider, chat_completions_endpoint, message_to_wire, parse_response,
    };
    use crate::domain::{Message, ToolCall};
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    #[test]
    fn endpoint_accepts_root_or_full_path() {
        assert_eq!(
            chat_completions_endpoint("https://example.test/v1/"),
            "https://example.test/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_endpoint("https://example.test/v1/chat/completions"),
            "https://example.test/v1/chat/completions"
        );
    }

    #[test]
    fn assistant_tool_call_uses_openai_wire_shape() {
        let message = Message::assistant(
            None,
            vec![ToolCall {
                id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: json!({"path": "README.md"}),
            }],
        );
        let wire = message_to_wire(&message);
        assert_eq!(wire["role"], "assistant");
        assert_eq!(wire["content"], serde_json::Value::Null);
        assert_eq!(wire["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(
            wire["tool_calls"][0]["function"]["arguments"],
            r#"{"path":"README.md"}"#
        );
    }

    #[test]
    fn parses_native_tool_call_response() {
        let reply = parse_response(
            r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"README.md\"}"}}]}}]}"#,
        )
        .expect("response");
        assert!(reply.content.is_none());
        assert_eq!(reply.tool_calls[0].name, "read_file");
        assert_eq!(reply.tool_calls[0].arguments, json!({"path": "README.md"}));
    }

    #[test]
    fn blank_tool_arguments_become_an_empty_object() {
        let reply = parse_response(
            r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"list_dir","arguments":"  "}}]}}]}"#,
        )
        .expect("response");
        assert_eq!(reply.tool_calls[0].arguments, json!({}));
    }

    #[tokio::test]
    async fn sends_real_openai_compatible_http_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let request = read_http_request(&mut socket).await.expect("request");
            let body = r#"{"choices":[{"message":{"content":"hello from mock","tool_calls":[]}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("response");
            request
        });

        let provider = OpenAiProvider::new(
            &format!("http://{address}/v1"),
            "test-model".to_owned(),
            Some("secret-key".to_owned()),
        )
        .expect("provider");
        let reply = provider
            .complete(
                &[Message::user("hello")],
                &[json!({
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "description": "read",
                        "parameters": {"type": "object"}
                    }
                })],
            )
            .await
            .expect("completion");
        assert_eq!(reply.content.as_deref(), Some("hello from mock"));

        let request = server.await.expect("server task");
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request.contains("authorization: Bearer secret-key"));
        let (_, body) = request.split_once("\r\n\r\n").expect("request body");
        let body: serde_json::Value = serde_json::from_str(body).expect("request JSON");
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["content"], "hello");
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
    }

    #[tokio::test]
    async fn rejects_oversized_chunked_provider_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let _ = read_http_request(&mut socket).await.expect("request");
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("headers");
            let body = vec![b'x'; super::MAX_PROVIDER_RESPONSE_BYTES + 1];
            let header = format!("{:X}\r\n", body.len());
            let _ = socket.write_all(header.as_bytes()).await;
            let _ = socket.write_all(&body).await;
            let _ = socket.write_all(b"\r\n0\r\n\r\n").await;
        });

        let provider = OpenAiProvider::new(
            &format!("http://{address}/v1"),
            "test-model".to_owned(),
            None,
        )
        .expect("provider");
        let error = provider
            .complete(&[Message::user("hello")], &[])
            .await
            .expect_err("oversized response must fail");
        assert!(error.to_string().contains("provider response exceeded"));
        server.await.expect("server task");
    }

    async fn read_http_request(socket: &mut TcpStream) -> std::io::Result<String> {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let count = socket.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if bytes.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }
}
