// SPDX-License-Identifier: GPL-3.0-only

use super::http::{
    McpHttpProbeResponse, filter_mcp_tools, mcp_initialize_request, mcp_tools_call_request,
    parse_mcp_arguments,
};
use super::*;
use async_trait::async_trait;
use ikaros_core::{Result, RiskLevel};
use ikaros_execution::harness::{ExecutionSession, NetworkEgressResponse};
use ikaros_execution::toolkit::{Skill, SkillContext, SkillOutput, SkillRegistry};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tempfile::tempdir;
use tokio::io::BufReader;

#[derive(Debug, Clone)]
struct EchoSkill;

#[async_trait]
impl Skill for EchoSkill {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn description(&self) -> &'static str {
        "Echo input"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {"type": "string"}
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: Value, _ctx: SkillContext) -> Result<SkillOutput> {
        Ok(SkillOutput::new("echoed", input))
    }
}

#[tokio::test]
async fn mcp_stdio_lists_and_calls_harness_skills() {
    let temp = tempdir().expect("tempdir");
    let mut registry = SkillRegistry::new();
    registry.register(EchoSkill);
    let session = ExecutionSession::new(temp.path(), temp.path().join("audit"));
    let input = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}
"#;
    let mut output = Vec::new();
    serve_mcp_stdio(
        registry,
        session,
        BufReader::new(input.as_slice()),
        &mut output,
    )
    .await
    .expect("serve");
    let lines = String::from_utf8(output)
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json"))
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[0]["result"]["serverInfo"]["runtime"],
        "ikaros-execution"
    );
    assert_eq!(lines[1]["result"]["tools"][0]["name"], "echo");
    assert_eq!(lines[2]["result"]["isError"], false);
    assert!(
        lines[2]["result"]["content"][1]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("hi")
    );
}

#[test]
fn mcp_initialize_request_uses_supported_protocol_version() {
    let request = mcp_initialize_request();
    assert_eq!(request["jsonrpc"], "2.0");
    assert_eq!(request["method"], "initialize");
    assert_eq!(request["params"]["protocolVersion"], "2024-11-05");
    assert_eq!(request["params"]["clientInfo"]["name"], "ikaros");
}

#[test]
fn mcp_tools_call_request_preserves_arguments_shape() {
    let request = mcp_tools_call_request("web_search", json!({"query": "ikaros"}));
    assert_eq!(request["jsonrpc"], "2.0");
    assert_eq!(request["method"], "tools/call");
    assert_eq!(request["params"]["name"], "web_search");
    assert_eq!(request["params"]["arguments"]["query"], "ikaros");
}

#[test]
fn mcp_arguments_must_be_json_object() {
    assert!(parse_mcp_arguments(r#"{"query":"ikaros"}"#).is_ok());
    assert!(parse_mcp_arguments(r#"["not", "object"]"#).is_err());
}

#[test]
fn mcp_http_tool_filter_applies_include_and_exclude() {
    let tools = vec![
        json!({"name": "read", "description": "Read"}),
        json!({"name": "write", "description": "Write"}),
        json!({"name": "secret", "description": "sk-tool-secret"}),
    ];
    let filtered = filter_mcp_tools(tools, &["read".into(), "secret".into()], &["secret".into()]);
    assert_eq!(
        filtered,
        vec![json!({"name": "read", "description": "Read"})]
    );
}

#[test]
fn mcp_http_response_redacts_non_json_preview() {
    let response = McpHttpProbeResponse::from_response(
        NetworkEgressResponse {
            status: 500,
            headers: BTreeMap::new(),
            body: "error sk-mcp-secret".into(),
            body_bytes: None,
        },
        1024,
    )
    .into_json();
    assert_eq!(response["http_status"], 500);
    assert!(!response.to_string().contains("sk-mcp-secret"));
    assert!(
        response["body_preview"]
            .as_str()
            .expect("preview")
            .contains("[REDACTED_SECRET]")
    );
}
