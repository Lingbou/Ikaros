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
- `context_profile()`：返回 provider-aware context-window metadata，供 runtime context budget 使用。

`ModelRequest` 携带 messages、类型化 request options 和可选 tool definition。Request options 包括输出上限、sampling 字段、stop sequence、reasoning 控制，以及用于 provider-specific 请求字段的 `extra_body` object。实际模型名由已配置的 provider 持有。`ModelResponse` 携带 provider、model、content、usage 和标准化 tool call。`ModelStreamEvent` 是 runtime event 层消费的 stream 协议；`chunks` 和 `tool_calls` 仍作为聚合字段保留给现有调用方。

`ModelContextProfile` 记录 context window、默认输出 token 预留、tokenizer kind 和 metadata source。Runtime 会在 context assembly 前用它收窄 `ContextBudget`。这还不是完整 provider registry；cost、health、cooldown、fallback chain 和 native tokenizer adapter 仍是后续工作。

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

Streaming 状态：

- `openai-compatible` 会把 provider SSE response body 解析成 rich `ModelStreamEvent`。
- `ollama` 会把 `/api/chat` streaming JSON lines 解析成同样的 event 形状。
- `anthropic` 当前暴露的是 generate-backed 的归一化 stream event；还不是原生 Anthropic streaming parser。
- `mock` 为测试发出确定性的本地 chunk。

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
    compat_profile: auto
    params:
      max_tokens: null
      temperature: null
      top_p: null
      n: null
      presence_penalty: null
      frequency_penalty: null
      seed: null
      stop: []
    reasoning:
      enabled: null
      effort: null
    extra_body: {}
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
    # 可选。留空时使用 http://127.0.0.1:11434。
    base_url: ""

model:
  default:
    provider: ollama
    model: llama3.2
    transport: ollama-chat
```

Anthropic 和 Ollama 是 native adapter，不是 OpenAI-compatible profile。Anthropic 会在本地解析正数 Messages API `max_tokens`，把 reasoning 配置映射到 Claude adaptive 或 manual thinking，并为 Claude 4.7+ 这类会拒绝 sampling 参数的模型省略 `temperature`、`top_p` 等字段。Ollama 会把 `params.max_tokens` 映射为 `options.num_predict`，并把显式配置的 `temperature`、`top_p`、`seed` 和 `stop` 透传给 `/api/chat` options。

Runtime workflow 和 provider adapter 使用同一套类型化 `ModelRequestOptions`。配置默认值会先和 per-call options 合并，再由 adapter 构造 wire payload。具体 profile 仍可以根据目标 provider 的要求省略、改写或新增字段。

## OpenAI-Compatible Adapter

OpenAI-compatible adapter 负责 Chat Completions 请求/响应、HTTP client、普通 completion、SSE stream parsing、tool-call 转换、request profile handling 和 stream tool-call accumulator。它不拥有 agent loop。

OpenAI-compatible provider 名称保持厂商中立。Provider 和模型差异放在 `model.default.compat_profile`，不通过 provider name alias 表达。`auto` 会先按 `providers.model.base_url` 选择 profile，再按模型名 hint 选择，最后回退到 `generic`。

Provider-specific profile name 只对 `provider: openai-compatible` 有效；原生 provider 只接受 `auto` 或 `generic`。

已实现的 profile：

- `generic`：当前标准 Chat Completions 行为。
- `moonshot-kimi`：省略 `temperature`，缺少 `max_tokens` 时默认 `32000`，发送 Kimi/Moonshot thinking 控制，并把 tool schema 修正到 Moonshot JSON Schema 子集。
- `deepseek`：对 `deepseek-reasoner` 和 DeepSeek V4+ 模型发送 thinking 控制；`deepseek-chat` V3 保持不变。
- `gemini-openai`：只对 Gemini family 模型把 reasoning options 映射到 Gemini OpenAI-compatible thinking config。
- `openrouter`：把 routing/session 字段保留在最终请求体中，并避免给现代 Claude route 发送无效 reasoning payload。
- `qwen`：把 Qwen/DashScope message 规范化为 text parts，给 system prompt 片段加入 ephemeral cache 标记，启用高分辨率图片字段，并在缺少 `max_tokens` 时默认使用 `65536`。
- `local-openai-compatible`：用于本地 Chat Completions server 的保守 profile；缺少 `max_tokens` 时默认使用 `65536`，避免本地服务输出过短。

Request builder 输出最终 raw HTTP JSON body。不要机械复制 OpenAI SDK 参数名：SDK 的 `extra_body` 会被合并进 body，因此 Kimi 的 `thinking` 是顶层 wire 字段，而 Gemini OpenAI-compatible 才使用真实的顶层 `extra_body.google.thinking_config` 字段。

当 provider 明确返回 `temperature` 或可省略的 `max_tokens` 是 unsupported parameter 时，adapter 会删除该字段并只重试一次 HTTP 请求。其它 provider error 不会被自动改写重试。
成功重试会记录 `kind: unsupported_parameter_retry` 的 `ModelRequestDiagnostic`；response、stream、audit payload 和 coding report 可以展示这条诊断，但不能暴露 prompt 或 secret。

当前 adapter 会读取 provider response body，再把 SSE `data:` 行解析成 typed event。文本、reasoning、refusal、native tool-call、usage 和 done marker 都会转换成 `ModelStreamEvent`。Tool-call fragment 会先累计到完整标准化 call；随后再发出 `ToolCallStart`、一次累计后脱敏的 `ToolCallDelta` 和 `ToolCallEnd`。这样可以避免半截 tool name，也避免 split secret-like 值通过 fragment-level redaction 泄漏。它还不是真正的 network-incremental streaming parser。

## 治理

Governance wrapper 处理：

- provider adapter 前的请求脱敏
- 每分钟请求限制
- 每日 token 预算估算
- 不含 prompt 的用量日志
- streaming response 的用量记录

用量记录位于本地 audit 状态中，包含 provider、model、timestamp 和 token count，不保存 prompt。

Governance 包裹 provider adapter。它应在 adapter 前看到 request，但不应理解 provider-specific wire format。因此 redaction、rate limiting、daily token budget 和 usage recording 对所有 provider family 一致生效。

每日 token 预算检查会计入配置或 per-call 的输出上限。当 OpenAI-compatible profile 提供隐式输出上限时，例如 Kimi 的 `32000` 或 Qwen/local 的 `65536`，governance preflight 会使用这个 profile 默认值，避免低估 strict profile 的请求成本。

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
