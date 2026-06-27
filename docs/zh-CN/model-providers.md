# 模型 Provider

模型相关代码位于 `ikaros-providers::model`，但 agent turn 不由 provider 拥有。当前边界分成三层：

- `ModelProvider`：生�?stream 模型输出�?
- `ModelTransport`：描�?provider wire format、base URL、streaming �?
  tool-call normalization 能力�?
- `AgentRuntime`：拥�?turn loop、tool continuation、stop reason �?harness dispatch�?

默认 runtime �?`harness-agent-loop`，默�?OpenAI-compatible transport �?
`openai-compatible-chat-completions`�?

## 接口契约

`ModelProvider` 暴露两个操作�?

- `generate(request)`：返回一�?`ModelResponse`�?
- `stream(request)`：返回包含文�?chunk、标准化 tool call、最终元数据�?typed `ModelStreamEvent` �?`ModelStream`�?
- `context_profile()`：返�?provider-aware context-window metadata，供 runtime context budget 使用�?

`ModelRequest` 携带 messages、类型化 request options 和可�?tool definition。Request options 包括输出上限、sampling
字段、stop sequence、reasoning 控制，以及用�?provider-specific 请求字段�?`extra_body` object。实际模型名由已配置�?provider
持有。`ModelResponse` 携带 provider、model、content、usage 和标准化 tool call。`ModelStreamEvent` �?runtime event
层消费的 stream 协议；`chunks` �?`tool_calls` 仍作为聚合字段保留给现有调用方�?

`ModelContextProfile` 记录 context window、默认输�?token 预留、tokenizer kind �?metadata source。Runtime 会在
context assembly 前用它收�?`ContextBudget`，并选择 context token estimator。OpenAI-compatible �?mock provider
已有本地确定�?estimator；Anthropic �?Ollama 当前选择显式 fallback estimator�?

`ProviderRegistry` 会提供本�?provider descriptor，供 inspect �?runtime 规划使用。Descriptor 包含 provider family�?
解析后的 profile、capability、context metadata、cost 字段、prompt-cache policy �?health state。这�?registry 是本�?
metadata：`provider inspect` 不会调用远端 provider。Runtime provider 调用会写�?`provider-health.jsonl`，连�?
retryable failure 后会进入 durable cooldown，并提供按顺序尝试备�?provider �?fallback-chain `ModelProvider`
primitive。Fallback 通过 `model.default.fallbacks` 配置；每�?fallback entry 都有自己�?provider、transport、model�?
可�?profile、请求参数，以及 `api_key` / `base_url`。Runtime 会构建一�?governed provider，先�?`model.default`，只�?
retryable provider failure 才按顺序尝试后续 fallback。Fallback 接手请求时，返回�?response/stream 会带脱敏 diagnostics�?
记录失败 provider 和最终选中�?fallback provider，供 replay/debug 解释 failover，但不暴�?prompt �?secret。Governance
wrapper 对同一 provider �?retry 成功后也会在 response/stream 中附带脱�?`provider_retry_*` diagnostics；retry
diagnostics 会包含基础重试延迟、已应用 jitter、最终重试延迟，以及影响延迟�?`Retry-After` header 或正文提示。Retry �?fallback
diagnostics 都只记录元数据；prompt 内容和疑�?secret 的错误片段会在进�?report/log 前脱敏。诊断边界还会在进入事件、audit payload �?coding
report 前限制字段长度：diagnostic kind 最�?96 个字符，message 最�?512 个字符，parameter 最�?128 个字符。同一路径还会发出结构�?
`tracing` 事件，覆�?provider request start、retry failure、terminal failure、completion、fallback failure �?
fallback selection；字段使用同一套脱敏边界，只包�?provider/model metadata、attempt、错误分类和诊断种类，不包含 prompt 文本�?

每条诊断�?agent loop 中会变成一条带类型�?`ModelDiagnostic` `AgentEvent`�?
与对应模型轮次一起落�?session timeline。`ikaros debug trace` 会把
`model_diagnostic` 事件里的诊断种类作为 `diagnostic_kind` 暴露，workbench
�?`/trace` 视图�?live cells 也会渲染同一种类。replay �?debug 永不打印
prompt 内容或原始错误，只显示脱敏后�?provider、model、attempt 和错误分类�?
Agent loop 还会发出结构�?`agent_loop_model_result` trace event，使用和 debug/replay
一致的 `session:<session_id>:turn:<turn_id>` correlation id�?

Provider 不应�?

- dispatch tool
- 审批 policy request
- �?memory �?RAG state
- 修改 workspace
- �?usage log 中保�?prompt

这些职责分别属于 runtime、harness、memory/RAG store �?governance�?

`ModelTransportDescriptor` 记录 provider family、transport name、base URL、是否支�?streaming、是否支�?native
tool-call normalization。Runtime selection 使用这些元数据，避免 agent loop 耦合 provider-specific JSON�?

## Provider

已实现：

- `mock`：确定性的本地 provider，用于显式离线测试�?
- `openai-compatible`：Chat Completions adapter�?
- `anthropic`：原�?Anthropic Messages API adapter，解�?`tool_use`�?
- `ollama`：本�?Ollama `/api/chat` adapter；模型支持时解析 tool call�?

Streaming 状态：

- `openai-compatible` 会把 provider SSE response body 解析�?rich `ModelStreamEvent`�?
- `ollama` 会把 `/api/chat` streaming JSON lines 解析成同样的 event 形状�?
- `anthropic` 会发�?`stream: true` Messages 请求，并解析原生 Anthropic SSE event，覆�?text delta、tool-use JSON
  delta、usage �?cache accounting。当�?HTTP client 仍会先读取完�?response body 再解析；socket 级增量投递属于后�?transport
  hardening�?
- `mock` 为测试发出确定性的本地 chunk�?

支持�?provider 名称�?

- OpenAI-compatible：`openai-compatible`
- Anthropic：`anthropic`
- Ollama：`ollama`
- 离线测试：`mock`

## 模型 Preset

`model.default.preset` 是面向用户的 provider 服务捷径。它会在配置加载时展开�?
provider family、transport �?compatibility profile。如果高级配置显式写�?
`provider`、`transport` �?`compat_profile`，这些字段仍会覆�?preset�?

Preset 摘要�?

- `auto`：OpenAI-compatible Chat Completions，`auto` profile，不固定 base URL�?
- `openai`：OpenAI-compatible Chat Completions，`generic` profile�?
  `https://api.openai.com/v1`�?
- `kimi`：OpenAI-compatible Chat Completions，`moonshot-kimi` profile�?
  `https://api.moonshot.cn/v1`�?
- `deepseek`：OpenAI-compatible Chat Completions，`deepseek` profile�?
  `https://api.deepseek.com`�?
- `gemini`：OpenAI-compatible Chat Completions，`gemini-openai` profile�?
  `https://generativelanguage.googleapis.com/v1beta/openai`�?
- `openrouter`：OpenAI-compatible Chat Completions，`openrouter` profile�?
  `https://openrouter.ai/api/v1`�?
- `qwen`：OpenAI-compatible Chat Completions，`qwen` profile�?
  `https://dashscope.aliyuncs.com/compatible-mode/v1`�?
- `local-openai`：OpenAI-compatible Chat Completions�?
  `local-openai-compatible` profile，`http://127.0.0.1:8080/v1`�?
- `ollama`：原�?Ollama chat，`ollama-native` profile�?
  `http://127.0.0.1:11434`�?
- `anthropic`：原�?Anthropic Messages，`anthropic-native` profile�?
  `https://api.anthropic.com`�?
- `mock`：mock provider �?mock profile�?

�?provider 配置建议把凭证直接写�?`model.default`�?

```yaml
model:
  default:
    preset: kimi
    model: kimi-k2-0711-preview
    api_key: "replace-with-provider-key"
    base_url: "https://api.moonshot.cn/v1"
```

�?provider 或多资源共享配置可以把共享模型凭证放�?`providers.model`；如果同时写�?
`model.default.api_key` �?`model.default.base_url`，inline 字段仍然优先�?

可以用下面的命令查看当前配置解析出的本地 descriptor�?

```bash
ikaros provider inspect
ikaros provider health
ikaros provider health --live
ikaros provider matrix
ikaros provider matrix --live
ikaros provider profiles
```

`provider inspect` 读取 `IKAROS_HOME/config.yaml`，解析当�?provider family/profile，并输出 context window�?
默认输出预留、tokenizer、capability、profile policy、health state，以�?input、output、cache-read、cache-write token
�?cost 字段。OpenAI-compatible provider �?profile policy 会显示已解析出的 temperature、reasoning、message
normalization、tool schema normalization、额�?request body 行为�?prompt-cache 行为�?
Qwen �?system cache marker 会显示为 `qwen-system-ephemeral`；没有稳�?cache
policy �?provider 会显�?`none`。它会脱敏看起来�?secret 的模型值，不打�?
API key。配�?`model.default.fallbacks` 时，`provider inspect` 还会输出
`fallback_count` 和每�?fallback �?`fallback_row`，包含解析后�?
provider/profile/readiness �?capability 摘要。`provider health` 读取本地 health
ledger；`provider health --live` 会通过 session `NetworkEgress` 发送一个很短的
真实请求，并把成功或失败写入同一�?ledger�?
`provider matrix` 会渲染当前配置的 model、embedding、TTS �?ASR provider 行，
包含 descriptor metadata、本地就绪检查�?
脱敏后的凭证存在性、最�?health status、cooldown metadata、capability flag、profile policy、context 字段�?
input/output/cache-read/cache-write cost 字段、fallback role、fallback count/model
list 和简�?`debug_hint`�?
Workbench 里的 `/model` 复用同一�?descriptor 输出面，并从当前 runtime model 输出已配�?fallback 行�?
`provider matrix --live` �?probe model、embedding、TTS �?ASR 行：model 和远�?
embedding probe 使用 runtime `NetworkEgress`，本�?embedding probe 使用本地 RAG
store，TTS/ASR probe 使用已配置的 voice provider�?
cost 字段来自 registry metadata，并�?`config.yaml` �?`model.default.cost` 覆盖�?
如果 provider 账单不区分普�?input、output、prompt-cache read �?
prompt-cache write token，就保持未知�?`null`�?
`provider profiles` 会输出静�?OpenAI-compatible profile catalog。每�?profile �?`id` 是显�?
`model.default.compat_profile` 的解�?key；同一行还会包�?auto-detection hints、capability flag、context metadata
�?request-shaping policy 字段。Workbench 里可以用 `/provider profiles` 查看同一�?catalog�?
`ikaros debug provider` �?workbench `/provider debug` 会输出脱敏后的结构化 provider 诊断视图，包�?profile source�?
prompt-cache policy、cost metadata、health、fallback row �?live-smoke readiness hint；这个命令不会主动发�?live
provider 请求�?

OpenAI-compatible 示例�?

```yaml
model:
  default:
    preset: kimi
    model: provider-model-id
    api_key: "replace-with-provider-key"
    base_url: "https://api.moonshot.cn/v1"
    fallbacks:
      - preset: ollama
        runtime: harness-agent-loop
        model: qwen2.5-coder:7b
        base_url: "http://127.0.0.1:11434"
        api_key: ""
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
    daily_token_budget: null
```

不要把真�?key 写入 tracked 文件。Preset 名称是普通用户配置面；provider 名称表示 adapter family，只有高级配置需要刻意覆�?
preset 展开结果时才应手写�?

Runtime chat、task agent-loop、provider-backed coding workflow �?provider-backed RAG embedding skill
会通过 session environment 构�?network-capable provider。Provider adapter 仍负�?wire payload，但实际网络 I/O �?
`ExecutionEnv` �?host allowlist、redaction �?timeout policy 管控�?

Anthropic 示例�?

```yaml
model:
  default:
    preset: anthropic
    model: claude-sonnet-4-5
    api_key: "replace-with-your-anthropic-key"
    base_url: "https://api.anthropic.com"
```

Anthropic request shaping 会把 system prompt 内容作为 Anthropic content block
发送。第一个非�?system block 会带 `cache_control: { type: "ephemeral" }`
标记，让支持 prompt caching �?Claude-family provider 能复用稳�?system prefix�?
Runtime chat 会把 cache-stable �?persona、policy �?tool guidance 与动�?
history、reference、memory、RAG、compaction notice 渲染成独�?system message�?
single-call 路径会直接发送这�?layer；harness agent-loop 路径会保留同样拆分，
并把 tool-call protocol 合并进第一�?cache-stable system message，而不是把动�?
context 混进去。因此被 cache 标记的前缀在不�?turn 之间保持字节稳定�?
Anthropic usage metadata 会把 `cache_creation_input_tokens` 映射�?
`TokenUsage.cache_write_tokens`，把 `cache_read_input_tokens` 映射�?
`TokenUsage.cache_read_tokens`；workbench status �?usage log 因此可以�?cache
读写 token 与普通输�?输出 token 分开解释�?

Ollama 本地示例�?

```yaml
model:
  default:
    preset: ollama
    model: llama3.2
    # 可选。留空时使用 http://127.0.0.1:11434�?
    base_url: ""
```

Anthropic �?Ollama �?native adapter，不�?OpenAI-compatible profile。Anthropic 会在本地解析正数 Messages API
`max_tokens`，把 reasoning 配置映射�?Claude adaptive �?manual thinking，并�?Claude 4.7+ 这类会拒�?sampling
参数的模型省�?`temperature`、`top_p` 等字段。Ollama 会把 `params.max_tokens` 映射�?`options.num_predict`，并把显式配置的
`temperature`、`top_p`、`seed` �?`stop` 透传�?`/api/chat` options�?

Runtime workflow �?provider adapter 使用同一套类型化 `ModelRequestOptions`。配置默认值会先和 per-call options 合并，再�?
adapter 构�?wire payload。具�?profile 仍可以根据目�?provider 的要求省略、改写或新增字段�?

## OpenAI-Compatible Adapter

OpenAI-compatible adapter 负责 Chat Completions 请求/响应、HTTP client、普�?completion、SSE stream parsing�?
tool-call 转换、request profile handling �?stream tool-call accumulator。它不拥�?agent loop�?

OpenAI-compatible provider 名称保持厂商中立。Provider 和模型差异放�?
`model.default.compat_profile`，不通过 provider name alias 表达。`auto` 会按静�?
profile catalog 中的 detection hints（`base_url` marker、模�?marker、模型尾�?
prefix）顺序匹配，最后回退�?`generic`�?

OpenAI-compatible profile name 只对 `provider: openai-compatible` 有效；原�?provider 接受 `auto`、`generic`，或
preset 展开出的 native profile（`anthropic-native`、`ollama-native`、`mock`）�?

OpenAI-compatible �?quirks 来自静�?`ProviderProfile` spec catalog。Catalog 同时包含 auto-detection hints �?
request-time decision。Request builder �?provider registry 共用解析后的 decision，读�?default output tokens�?
context metadata、temperature policy、reasoning policy、message policy、tool-schema policy 和额�?request
body policy。这�?provider inspect �?wire payload 构造不会各自维护一�?profile 逻辑，新�?profile 也应集中�?catalog，而不是继续把
provider 分支散进 wire builder�?

已实现的 profile�?

- `generic`：当前标�?Chat Completions 行为�?
- `moonshot-kimi`：省�?`temperature`，缺�?`max_tokens` 时默�?`32000`�?
  发�?Kimi/Moonshot thinking 控制，并�?tool schema 修正�?Moonshot JSON
  Schema 子集�?
- `deepseek`：对 `deepseek-reasoner` �?DeepSeek V4+ 模型发�?thinking 控制；`deepseek-chat` V3 保持不变�?
- `gemini-openai`：只�?Gemini family 模型�?reasoning options 映射�?Gemini
  OpenAI-compatible thinking config�?
- `openrouter`：把 routing/session 字段保留在最终请求体中，并避免给现代 Claude route 发送无�?reasoning payload�?
- `qwen`：把 Qwen/DashScope message 规范化为 text parts，给 system prompt
  片段加入 ephemeral cache 标记，启用高分辨率图片字段，
  并在缺少 `max_tokens` 时默认使�?`65536`�?
- `local-openai-compatible`：用于本�?Chat Completions server 的保�?profile�?
  缺少 `max_tokens` 时默认使�?`65536`，避免本地服务输出过短�?

Moonshot schema sanitizer 只作用在发给 provider 的请求体上，不会修改注册表里�?
tool definition。它会把 `oneOf` 转成 `anyOf`，丢�?null 分支�?`nullable`，移�?
`title`、`minimum`、`maximum`、`format` 等目�?provider 不接受的校验字段，推断缺失的
`type`，并删除和最终标量类型不匹配�?enum 值。顶�?parameters 如果不是 object�?
会退化成�?object schema�?

Request builder 输出最�?raw HTTP JSON body。不要机械复�?OpenAI SDK 参数名：SDK �?`extra_body` 会被合并�?body，因�?Kimi
�?`thinking` 是顶�?wire 字段，�?Gemini OpenAI-compatible 才使用真实的顶层 `extra_body.google.thinking_config` 字段�?

�?provider 明确返回 `temperature` 或可省略�?`max_tokens` �?unsupported parameter 时，adapter 会删除该字段并只重试一�?HTTP
请求。其�?provider error 不会被自动改写重试�?
成功重试会记�?`kind: unsupported_parameter_retry` �?`ModelRequestDiagnostic`；response、stream、audit payload
�?coding report 可以展示这条诊断，但不能暴露 prompt、secret 或无�?provider 错误体�?

当前 adapter 会读�?provider response body，再�?SSE `data:` 行解析成 typed event。文本、reasoning、refusal、native
tool-call、usage �?done marker 都会转换�?`ModelStreamEvent`。文本、reasoning �?refusal delta 使用一个小�?
pending-token redactor：内容只会在遇到 whitespace 边界或最�?flush 时释放，因此�?SSE chunk 拆开�?`sk-`/`token=` 类值会作为同一�?
token 脱敏，而不是泄漏后半段。Tool-call fragment 会先累计到完整标准化 call；随后再发出 `ToolCallStart`、一次累计后脱敏�?`ToolCallDelta`
�?`ToolCallEnd`。这样可以避免半�?tool name，也避免 split secret-like 值通过 fragment-level redaction 泄漏。它还不是真正的
network-incremental streaming parser�?

## 治理

Governance wrapper 处理�?

- provider adapter 前的请求脱敏
- 每分钟请求限�?
- 每日 token 预算估算
- 不含 prompt 的用量日�?
- streaming response 的用量记�?
- provider error 分类，以�?retryable failure �?retry/backoff

用量记录位于本地 audit 状态中，包�?provider、model、timestamp �?token count，不保存
prompt。usage ledger 会为 budget �?workbench status 读取保留轻量进程内缓存；�?JSONL
文件变化时会刷新，如果遇到崩溃留下的末尾半行，会回退到最后一份有效缓存记录�?

Governance 包裹 provider adapter。它应在 adapter 前看�?request，但不应理解 provider-specific wire format。因�?
redaction、rate limiting、daily token budget �?usage recording 对所�?provider family 一致生效�?

每日 token 预算检查会计入配置�?per-call 的输出上限。当 OpenAI-compatible profile 提供隐式输出上限时，例如 Kimi �?`32000` �?
Qwen/local �?`65536`，governance preflight 会使用这�?profile 默认值，避免低估 strict profile 的请求成本�?

`model.default.max_retries` 会控制配�?provider 外层�?governance retry policy。可重试类别包括 rate-limit、瞬时服务端错误�?
network failure。认证错误、bad request �?context-limit failure �?terminal。Backoff 使用本地默认的指数退避和上限；adapter
内部针对 unsupported parameter 的精确字段重试仍然是独立机制，并且只移除明确不支持的字段�?

失败语义�?

- 缺少凭证会在远程调用前失败�?
- Provider HTTP error 会返回脱敏后�?response body�?
- Rate-limit �?token-budget failure 会在 provider call 前停止�?
- Usage logging failure 不应暴露 prompt text�?

## Tool

`ModelRequest` 可以包含 tool definition。OpenAI-compatible �?Ollama provider
会把它们序列化为 function tool，并�?native `tool_calls` 解析�?
`ModelResponse`。Anthropic 会序列化�?Messages API tools，并解析 `tool_use`
block�?

Runtime agent loop 优先消费 native tool call；当 provider 返回 call id 时，会保�?
native tool call/tool result 历史�?
如果 provider 返回普通文本，loop 可以回退到内�?JSON tool-call 协议�?

Tool-call normalization 规则�?

- Provider-native name �?JSON arguments 转换�?`ModelToolCall`�?
- Provider call id 存在时必须保留�?
- Invalid 或缺失的 argument JSON 只有�?adapter 能确定性处理时才转换为�?object�?
- Provider-specific tool result history �?runtime/model-turn 层构造，不由 skill 构造�?

Adapter 应优先使�?native tool call，而不是提示模型输�?JSON。Fallback JSON 协议�?
agent-loop �?plain text provider/model 保留的兼容路径�?

## Live Test

Live provider test 默认 ignored，需要显式启用：

```bash
export IKAROS_RUN_LIVE_MODEL_TESTS=1
cargo test -p ikaros-providers --test live_model -- --ignored
```

�?`model.default` 匹配被测 provider 时，Live smoke test 会从当前 `IKAROS_HOME/config.yaml` 读取 `api_key`�?
`base_url` �?`model`。Live smoke test 只验证连通性、基础响应和用量日志，不应打印模型内容�?secret�?
