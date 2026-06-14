# 模型 Provider

模型相关代码位于 `ikaros-models`，但 agent turn 不由 provider 拥有。当前边界分成三层：

- `ModelProvider`：生成/stream 模型输出。
- `ModelTransport`：描述 provider wire format、base URL、streaming 和 tool-call normalization 能力。
- `AgentRuntime`：拥有 turn loop、tool continuation、stop reason 和 harness dispatch。

默认 runtime 是 `harness-agent-loop`，默认 OpenAI-compatible transport 是 `openai-compatible-chat-completions`。

## 接口契约

`ModelProvider` 暴露两个操作：

- `generate(request)`：返回一个 `ModelResponse`。
- `stream(request)`：返回包含文本 chunk、标准化 tool call、最终元数据和 typed `ModelStreamEvent` 的 `ModelStream`。

`ModelRequest` 携带 model name、messages、可选 max tokens、可选 temperature 和可选 tool definition。`ModelResponse` 携带 provider、model、content、usage 和标准化 tool call。`ModelStreamEvent` 是 runtime event 层消费的 stream 协议；`chunks` 和 `tool_calls` 仍作为聚合字段保留给现有调用方。

Provider 不应：

- dispatch tool
- 审批 policy request
- 写 memory 或 RAG state
- 修改 workspace
- 在 usage log 中保存 prompt

这些职责分别属于 runtime、harness、memory/RAG store 或 governance。

`ModelTransportDescriptor` 记录 provider family、transport name、base URL、是否支持 streaming、是否支持 native tool-call normalization。Runtime selection 使用这些元数据，避免 agent loop 耦合 provider-specific JSON。

## Provider

已实现：

- `mock`：确定性的本地 provider，用于显式离线测试。
- `openai-compatible`：Chat Completions adapter。
- `anthropic`：原生 Anthropic Messages API adapter，解析 `tool_use`。
- `ollama`：本地 Ollama `/api/chat` adapter；模型支持时解析 tool call。

支持的 provider 名称：

- OpenAI-compatible：`openai-compatible`
- Anthropic：`anthropic`
- Ollama：`ollama`
- 离线测试：`mock`

OpenAI-compatible 示例：

```yaml
providers:
  model:
    api_key: "replace-with-provider-key"
    base_url: "https://api.example.com/v1"

model:
  default:
    provider: openai-compatible
    model: provider-model-id
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    rate_limit_per_minute: 60
    daily_token_budget: 100000
```

`api_key` 和 `base_url` 存在于本机 `IKAROS_HOME/config.yaml` 的 `providers.model`。不要把真实 key 写入 tracked 文件。Provider 名称表示 adapter family，不应把厂商名编码进 `model.default.provider`。

Anthropic 示例：

```yaml
providers:
  model:
    api_key: "replace-with-your-anthropic-key"
    base_url: "https://api.anthropic.com/v1"

model:
  default:
    provider: anthropic
    model: claude-sonnet-4-5
    transport: anthropic-messages
```

Ollama 本地示例：

```yaml
providers:
  model:
    api_key: ""
    base_url: "http://127.0.0.1:11434"

model:
  default:
    provider: ollama
    model: llama3.2
    transport: ollama-chat
```

## OpenAI-Compatible Adapter

OpenAI-compatible adapter 负责 Chat Completions 请求/响应、HTTP client、普通 completion、SSE stream parsing、tool-call 转换和 stream tool-call accumulator。它不拥有 agent loop。

OpenAI-compatible provider 是厂商中立的。它不携带 provider alias，也不做模型名专用的请求修正；endpoint 差异应放在配置里，或后续通过显式 adapter option 表达，不应藏在 provider 名称里。

Streaming 会增量解析 SSE chunk。文本、reasoning、refusal、native tool-call、usage 和 done marker 都会转换成 typed `ModelStreamEvent`。Tool-call delta 会累计，直到能形成完整的标准化 tool call，再交回 agent loop；单个 argument delta 也会作为 stream event 发出。

## 治理

Governance wrapper 处理：

- provider adapter 前的请求脱敏
- 每分钟请求限制
- 每日 token 预算估算
- 不含 prompt 的用量日志
- streaming response 的用量记录

用量记录位于本地 audit 状态中，包含 provider、model、timestamp 和 token count，不保存 prompt。

Governance 包裹 provider adapter。它应在 adapter 前看到 request，但不应理解 provider-specific wire format。因此 redaction、rate limiting、daily token budget 和 usage recording 对所有 provider family 一致生效。

失败语义：

- 缺少凭证会在远程调用前失败。
- Provider HTTP error 会返回脱敏后的 response body。
- Rate-limit 或 token-budget failure 会在 provider call 前停止。
- Usage logging failure 不应暴露 prompt text。

## Tool

`ModelRequest` 可以包含 tool definition。OpenAI-compatible 和 Ollama provider 会把它们序列化为 function tool，并把 native `tool_calls` 解析回 `ModelResponse`。Anthropic 会序列化为 Messages API tools，并解析 `tool_use` block。

Runtime agent loop 优先消费 native tool call；当 provider 返回 call id 时，会保留 native tool call/tool result 历史。如果 provider 返回普通文本，loop 可以回退到内部 JSON tool-call 协议。

Tool-call normalization 规则：

- Provider-native name 和 JSON arguments 转换为 `ModelToolCall`。
- Provider call id 存在时必须保留。
- Invalid 或缺失的 argument JSON 只有在 adapter 能确定性处理时才转换为空 object。
- Provider-specific tool result history 由 runtime/model-turn 层构造，不由 skill 构造。

Adapter 应优先使用 native tool call，而不是提示模型输出 JSON。Fallback JSON 协议是 agent-loop 为 plain text provider/model 保留的兼容路径。

## Live Test

Live provider test 默认 ignored，需要显式启用：

```bash
export IKAROS_RUN_LIVE_MODEL_TESTS=1
cargo test -p ikaros-models --test live_model -- --ignored
```

当 `model.default` 匹配被测 provider 时，Live smoke test 会从当前 `IKAROS_HOME/config.yaml` 读取 `api_key`、`base_url` 和 `model`。Live smoke test 只验证连通性、基础响应和用量日志，不应打印模型内容或 secret。
