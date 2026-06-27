# 配置

Ikaros 默认把本地状态存到 `~/.ikaros`。可以用 `IKAROS_HOME` 或 `--ikaros-home` 隔离运行：

```bash
export IKAROS_HOME=/tmp/ikaros-dev
ikaros --ikaros-home /tmp/ikaros-dev doctor
```

`ikaros init` 会创建 runtime home，并只写一个配置文件：`IKAROS_HOME/config.yaml`。它不会从仓库里的 example 目录读取配置。

## Schema Version

生成配置会包含顶层 `schema_version: 1`。Runtime 加载和 `ikaros config validate`
都要求显式写出这个字段。缺失或不支持的 schema version 会在 runtime 使用配置前报错。

## Provider 设置

不要把真实 API key 放进文档、测试、示例或 tracked 文件。本地未跟踪的
`IKAROS_HOME/config.yaml` 可以直接保存第三方 API key，用于普通运行和 smoke test。

`ikaros init` 默认只写一份极简模型配置：

```yaml
schema_version: 1

model:
  default:
    preset: auto
    model: ""
    api_key: ""
    base_url: ""
```

常见的单模型场景只需要填 `model.default.model`、`api_key` 和 `base_url`。`preset: auto`
会保留 runtime provider-profile 自动检测；已知 provider 时可以改成 `kimi`、`openai`、`anthropic` 或
`ollama` 等具体 preset。Preset 会在配置加载时展开成 provider family、wire transport 和 compatibility profile，
所以大多数用户不需要手写 `provider`、`transport` 或 `compat_profile`。

`ikaros init --full` 会写出完整默认 YAML，包括 provider、agent profile、memory、RAG、voice、gateway 和
execution 等可选配置段。完整配置同样不带注释。

模型凭证可以写在 `model.default` 下面，也可以写在共享池 `providers.model` 下面。
Inline 的 `model.default.api_key` 和 `model.default.base_url` 优先；inline 为空时回退到
`providers.model`。Fallback 模型条目如果自己写了 `api_key` 和 `base_url` 就使用自己的值，否则继承共享的
model provider 设置。

`providers.embedding`、`providers.tts`、`providers.asr` 和 `providers.search` 仍然是对应资源类型的共享
provider 设置。明文 key 只应存在于本机 runtime home，不应提交进仓库。

`providers.search` 给 `web_search` 提供默认 key 或 endpoint。内置 `duckduckgo-html`
provider 可以在 `base_url` 为空时运行；Brave、Bing、SerpAPI 和 Tavily 风格 provider
可以使用 `providers.search.api_key` 与 `providers.search.base_url`，也可以在命令里通过
`ikaros web search --provider ... --endpoint ... --api-key ...` 覆盖。

`ikaros setup` 会写入同一批字段，并且不会打印 secret 值。如果同一个
OpenAI-compatible endpoint 同时提供模型、embedding、TTS 或 ASR，可以配合对应的
resource model 参数使用 `--reuse-model-provider-for-embedding`、
`--reuse-model-provider-for-tts` 或 `--reuse-model-provider-for-asr`。交互式 setup
会在模型 provider 配好后询问是否复用这组 key 和 base URL。

## 配置校验

编辑本地 runtime 配置后运行：

```bash
ikaros config validate
ikaros config show
```

普通 runtime 加载配置和显式 `config validate` 会共用同一套 shape 与语义校验，并在返回 `IkarosConfig` 前拒绝未知字段、非法
provider/runtime/transport/backend 组合、缺失的 key、URL、模型名，以及 descriptor-only 的外部 memory provider。输出只使用
`providers.model.api_key` 这类字段路径说明缺失或非法，不会打印 secret 值。

自动化场景使用：

```bash
ikaros config validate --json
ikaros config show --json
```

`config show` 只输出脱敏 runtime 摘要：provider family、模型名、存储 backend、
execution 设置，以及 `model_api_key_configured` 这类布尔值。它不会打印明文 credential
或 base URL。JSON 模式只向 stdout 写报告。配置无效时 validate 仍返回非零退出码，
但报告可稳定读取 `valid`、`path`、`errors[]` 和 `warnings[]`。

## 执行边界

Runtime session 会通过 `ikaros-host` 从 `execution` 段创建 `ExecutionEnv`：

```yaml
execution:
  network:
    enabled: true
    allow_provider_hosts: true
    allowed_hosts: []
    timeout_ms: 30000
  sandbox:
    backend: local
    image: rust:1.85-bookworm
    read_scope: workspace
```

共享的 `ExecutionEnv`、filesystem、process 和 network trait 位于 `ikaros-toolkit`。
具体的 local、dry-run、Docker、workspace-scoped 和 network-governed 实现位于
`ikaros-sandbox`。

`network.enabled` 打开 HTTP egress backend。网络出口仍然默认拒绝：
`allow_provider_hosts` 会把 active model provider、agent instance 模型 provider
覆盖、embedding/TTS/ASR/search provider、内置 web-search provider 默认值和本地
Ollama 默认地址解析出的精确 host 放入 allowlist；`allowed_hosts` 用于后续
network-capable tool 的额外精确 host。这里写 host name，不写完整 URL 或
`host:port`。
网络出口会拒绝非 HTTP scheme、私网/链路本地/组播 IP 字面量，并关闭自动重定向。
有效 provider base URL 包括 inline 的 `model.default.base_url`、共享的
`providers.*.base_url`、per-agent instance 模型 provider 覆盖，以及本地 Ollama
默认地址。内置 web-search 默认值当前覆盖 DuckDuckGo HTML、Brave、Bing、SerpAPI
和 Tavily 风格 endpoint。显式配置的 loopback host 仍可用于 Ollama 这类本地
provider；普通域名如果解析到受限地址，HTTP egress transport 会拒绝请求。解析出的
socket address 会在验证后 pin 到本次 HTTP client，避免同一次请求再做第二次独立
DNS lookup。这是 runtime guardrail，不是完整 OS-level 网络沙箱。

`sandbox.backend` 支持 `local`、`dry-run` 和 `docker`。Local session 使用
workspace-scoped filesystem/process 环境和受控 HTTP egress。Dry-run session
保留读取能力，但只跳过文件写入和进程执行。Docker session 会通过 `docker run`
执行进程，把 workspace bind mount 到容器内 `/workspace`，把进程 cwd 映射到该
mount 内，并用 `--network none` 启动容器；`sandbox.image` 用来选择容器镜像。
文件读写仍经过 workspace-scoped `ExecutionEnv`，provider HTTP egress 仍使用进程容器外的
governed runtime transport。网络出口由 `execution.network.enabled` 和 host allowlist
单独控制；如果 dry-run 也必须避免网络副作用，需要把 `network.enabled` 设为 false。
`read_scope` 目前固定为 `workspace`；已有路径会 canonicalize，因此读写都会拒绝 symlink 逃逸。

## MCP Server

外部 MCP server 配在 `mcp.servers` 下，并且默认需要显式启用：

```yaml
mcp:
  servers:
    - id: local-tools
      enabled: false
      transport: stdio
      command: /path/to/mcp-server
      args: []
      include_tools: []
      exclude_tools: []
      timeout_ms: 5000
      max_output_bytes: 65536
```

当前 client 切片只支持 `stdio` transport。配置里的 server 不会被当成可信本地命令：
`ikaros mcp probe <id>` 会通过 harness process 边界启动它，因此仍使用 policy、
approval、audit、workspace scope、timeout 和 output cap。`include_tools` 和
`exclude_tools` 是对 `tools/list` 结果应用的精确工具名过滤。HTTP MCP transport
目前没有启用；后续必须通过 `NetworkEgress` 实现。

常用命令：

```bash
ikaros mcp status
ikaros mcp status --json
ikaros mcp probe local-tools --force
```

## Agent Profile

Profile 选择 persona overlay 和普通策略行为：

```yaml
agent:
  default: build
  profiles:
    build:
      mode: build
      workspace_writes: ask
      shell: allow
      network: ask
      memory_context: true
      rag_context: false
      toolsets: [core, workspace, memory, rag, coding, voice, plugin]
    plan:
      mode: plan
      workspace_writes: deny
      shell: ask
      network: ask
      memory_context: true
      rag_context: false
      toolsets: [core, workspace, memory, rag, coding, voice, plugin]
```

普通 chat 建议保持 `rag_context` 为 false。只有当前 turn 需要带 citation 的本地
reference snippet 时，再在 profile 中启用或传入 `--rag-top-k`。
长期 memory search 也需要显式打开。普通 chat 会读取已接受的 memory projection 和 session working memory；如果某个 turn 需要
retrieved memory result，再使用 `--memory-search-limit` 或 `memory_search` 工具。
`toolsets` 控制当前 profile 启用哪些 skill group。只有直接工具面
`core`、`workspace`、`memory` 会进入模型 tool manifest；默认 profile 会启用
`rag`、`coding`、`voice`、`plugin`，但这些工具集仍保持 deferred。模型可以通过
`tool_search`、`tool_describe`、`tool_call`
发现并显式调用 deferred tool，但 bridge 会拒绝当前 profile 未启用 toolset 里的
deferred tool。目标工具的实际执行仍经过 harness policy、approval 和 audit。启用任何
deferred toolset 的 profile 必须同时保留 `core`，因为 bridge tools 属于 `core`。

使用 profile：

```bash
ikaros --agent plan chat --message "review only"
ikaros agent run --profile build --dry-run "inspect this repo"
```

Profile 不能绕过破坏性命令、直接 secret 访问、受保护路径、发布动作、workspace 外写入或 self-modify 的硬性拒绝。

## Agent Instance

Agent instance 是 runtime identity。Profile 回答“这个 agent 应该怎样工作”；instance 回答“哪个 agent 正在运行、在哪个
workspace、使用哪个 state 和 routing policy”。

示例：

```yaml
agent:
  instances:
    repo-build:
      profile: build
      workspace: /home/user/src/project
      state_dir: /home/user/.ikaros/agents/repo-build
      toolsets: [core, workspace, memory, coding]
      providers:
        model:
          api_key: "sk-..."
          base_url: "https://api.example.com/v1"
      model:
        provider: openai-compatible
        runtime: harness-agent-loop
        transport: openai-compatible-chat-completions
        model: repo-specialist-model
      session_policy:
        history_scope: workspace
        allow_session_switch: true
        max_parallel_subagents: 4
        max_delegation_depth: 2
      auth_scope:
        local_only: true
        allow_network: ask
      route_bindings:
        - channel: cli
```

字段：

- `profile`：用于 persona 和普通策略行为的 profile overlay。
- `workspace`：可选 workspace override。省略时使用调用方 workspace。
- `state_dir`：可选 state dir override。省略时使用 `IKAROS_HOME/agents/<agent_id>`。
- `toolsets`：可选的模型可见/延迟披露 toolset allowlist override。省略时使用所选 profile 的 toolsets。启用 `rag`、`coding`、
  `voice` 或 `plugin` 这类 deferred toolset 时，应保留 `core`，让 `tool_search`、`tool_describe` 和 `tool_call`
  可见。
- `providers.model`：该 identity 的模型 endpoint 和 key override。省略时使用顶层 `providers.model`。
- `model`：该 identity 的完整 `ModelConfig` override。省略时使用 `model.default`。
- `session_policy.history_scope`：`agent`、`session` 或 `workspace`。
- `session_policy.allow_session_switch`：该身份是否允许 runtime 切换 session。
- `session_policy.max_parallel_subagents`：并发 delegated work 上限。
- `session_policy.max_delegation_depth`：嵌套 agent handoff 深度上限；超过上限的请求会在 delegated task 启动前失败。
- `auth_scope.local_only`：该身份默认是否 local-only。
- `auth_scope.allow_network`：该身份的 network 默认策略。
- `route_bindings`：gateway routing 使用的 channel/account/peer/thread 绑定。

解析规则：

1. 请求名称先匹配 `agent.instances.<name>`。
2. 如果没有 instance，则同名解析为 `agent.profiles.<name>`。
3. 如果调用方没有传名称，则使用 `agent.default`。

审批和审计记录应使用解析后的 instance `agent_id`，而不只是 profile name。

Chat、TUI、coding model loop、task agent-loop、`doctor` 以及 `provider
inspect|health|matrix` 都会通过当前 `AgentInstance` 解析模型设置。
Embedding、TTS 和 ASR 仍使用全局资源配置，除非对应 runtime 路径显式增加
instance override。

## 本地 Store

JSONL 是默认后端。较大的本地 store 可以使用 SQLite：

```yaml
memory:
  backend: sqlite
  policy:
    promote_threshold: 0.75
    demote_threshold: 0.35
    forget_threshold: 0.15
    max_records_per_scope: 2000

rag:
  backend: sqlite
  embedding_provider: hash
```

Agent `state.db` session store 是权威聊天 timeline。普通 chat turn 不再写单独的
chat history mirror。历史、搜索、replay、context assembly 和 workbench 视图都从
session replay 投影。

Memory policy 字段：

- `promote_threshold`：综合分数达到或超过该值时，记录 `promote` action，并给 record 标记 policy-promoted。
- `demote_threshold`：综合分数低于或等于该值时，记录 `demote` action，并给 record 标记 policy-demoted。
- `forget_threshold`：综合分数低于或等于该值时，记录 `forget` action，并删除低分 record。
- `max_records_per_scope`：每个 kind/scope 的保留上限。某轮写入导致 scope
  超限时，会删除最低分 record，并以 quota reason 记录为 `forget` action。

主要本地路径：

- `IKAROS_HOME/memory/`
- `IKAROS_HOME/rag/`
- `IKAROS_HOME/audit/`
- `IKAROS_HOME/automation/`
- `IKAROS_HOME/gateway/`
- `IKAROS_HOME/skills/`

## 模型 Provider

极简配置从 `preset: auto` 和空的 key、URL、模型名开始。远程模型调用会在网络请求之前检查必要字段，缺少字段会直接报错。
支持的 preset ID 见 `model-providers.md`；支持的 provider family 包括 `openai-compatible`、`anthropic`、`ollama`
和 `mock`。

Runtime chat、task agent loop、provider-backed coding command 和 provider-backed RAG embedding skill 的
provider 调用现在都经过 session environment，不再直接使用裸 HTTP client。用 `ikaros provider health` 查看本地 health
ledger，用 `ikaros provider health --live` 执行真实 provider health probe。

使用 inline 凭证的单 provider OpenAI-compatible 示例：

```yaml
model:
  default:
    preset: kimi
    model: provider-model-id
    api_key: "replace-with-provider-key"
    base_url: "https://api.moonshot.cn/v1"
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
    cost:
      currency: USD
      input_per_million: null
      output_per_million: null
      cache_read_per_million: null
      cache_write_per_million: null
    rate_limit_per_minute: 60
    daily_token_budget: null
    max_retries: 2
```

Preset 名称是面向用户的 provider 捷径。Provider 名称表示 adapter family，不表示厂商。任何 Chat
Completions-compatible 服务都使用 OpenAI-compatible preset，具体 endpoint 和模型写在
`model.default.base_url` 和 `model.default.model`。多 provider 配置可以把共享模型 key 和 endpoint 放到
`providers.model`；inline 模型字段仍会覆盖这个共享池。

`model.default.cost` 是本地价格元数据，供 `provider inspect`、`provider matrix`、
workbench cost cell 和 usage estimate 使用。Ikaros 不硬编码实时厂商价格。需要成本估算时，
把当前 provider 账号里的 `currency`、`input_per_million`、`output_per_million`、
`cache_read_per_million` 和 `cache_write_per_million` 填进去；未知字段可以保持 `null`。
cache read/write 未配置时会按普通 input 价格估算。

Ollama 也可以作为本地 embedding provider：

```yaml
providers:
  embedding:
    api_key: ""
    # 可选。留空时使用 http://127.0.0.1:11434。
    base_url: ""

rag:
  embedding_provider: ollama
  embedding_model: nomic-embed-text
```

`max_retries` 控制 provider 外层 governance retry policy，用于 rate limit、瞬时服务端错误和 network failure 这类
retryable failure。鉴权、bad request 和 context-limit failure 是 terminal。默认策略使用较短且有上限的指数退避；它和
OpenAI-compatible adapter 内部那次 unsupported-parameter retry 是两套机制。

`compat_profile` 控制 OpenAI-compatible adapter 内部的 provider/model 请求差异。Preset 会自动填这个字段。
`auto` 会先按有效模型 base URL 匹配，再按模型名 hint 匹配，最后回退到 `generic`。支持显式指定：

- `generic`：只发送标准 Chat Completions 字段。
- `moonshot-kimi`：Kimi/Moonshot。省略 `temperature`，缺少 `max_tokens`
  时默认 `32000`，发送 Kimi thinking 字段，并把 tool schema 修正到
  Moonshot 更严格的 JSON Schema 子集。
- `deepseek`：对 `deepseek-reasoner` 和 `deepseek-v4+` 模型发送 DeepSeek
  thinking 字段；`deepseek-chat` V3 保持普通 chat 形态。
- `gemini-openai`：只对 Gemini family 模型把 reasoning 配置映射为 Gemini
  OpenAI-compatible 的 `extra_body.google.thinking_config`。
- `openrouter`：保留 OpenRouter routing 字段，并避免给现代 Claude route 发送无效 reasoning 字段。
- `qwen`：Qwen/DashScope 兼容请求形态。会把 message content 规范化为
  text parts，给 system prompt 的最后一段加 ephemeral cache 标记，启用高分辨率
  图片字段，并在缺少 `max_tokens` 时默认使用 `65536`。
- `local-openai-compatible`：用于 LM Studio、vLLM、SGLang 等本地
  Chat Completions 服务，默认更保守；缺少 `max_tokens` 时使用 `65536`，
  避免本地服务默认输出过短。

Moonshot sanitizer 只修正发给 provider 的请求 payload，不会修改注册表里的 tool
schema。具体修正规则见 [模型 Provider](model-providers.md)。

OpenAI-compatible 的 profile 值只对 `provider: openai-compatible` 有效。原生
provider 接受 `auto`、`generic`，或 preset 展开出的 native profile
（`anthropic-native`、`ollama-native`、`mock`）。

对可选数值型 `params` 字段，`null` 表示 adapter 不发送该参数，除非选中的
profile 提供 provider 默认值。当前支持：

- `max_tokens`：正数输出 token 上限。
- `temperature`：采样温度，校验范围是闭区间 `0.0..2.0`。
- `top_p`：nucleus sampling，校验范围是闭区间 `0.0..1.0`。
- `n`：provider 支持时请求的正数 completion 数量。
- `presence_penalty` 和 `frequency_penalty`：校验范围是闭区间 `-2.0..2.0`。
- `seed`：provider 支持时使用的确定性 seed。
- `stop`：非空 stop sequence 列表，列表元素不能为空字符串。

`reasoning.effort` 可取 `none`、`minimal`、`low`、`medium`、`high`、`xhigh` 或 `max`。Runtime 可以为特定 workflow
设置 per-call options，但 strict profile 仍会移除或改写目标 provider 会拒绝的字段。`extra_body` 是 JSON object，会在 common
params 之后、profile-specific shaping 之前合并进 provider 请求体；日志和审计只能记录脱敏摘要，不能写入原始 secret-like 值。

每日 token 预算预检查会使用配置或 per-call 的 `max_tokens`。如果选中的
OpenAI-compatible profile 在缺少 `max_tokens` 时提供默认值，
例如 `moonshot-kimi`、`qwen` 或 `local-openai-compatible`，这个 profile 默认输出上限也会进入预算估算。

如果 OpenAI-compatible provider 明确返回 `temperature` 或可省略的 `max_tokens`
不支持，adapter 会删除该字段并只重试一次 HTTP 请求。
鉴权、额度、网络和普通参数校验错误不会走这个重试路径。

Anthropic 示例：

```yaml
model:
  default:
    preset: anthropic
    model: claude-sonnet-4-5
    api_key: "replace-with-anthropic-key"
    base_url: "https://api.anthropic.com"
```

Anthropic adapter 总会发送正数 `max_tokens`。当 `model.default.reasoning`
启用 thinking 时，现代 Claude 模型使用 adaptive thinking 和
`output_config.effort`；旧 Claude 模型使用 budget-based thinking。Claude
4.7 及更新模型会省略 `temperature`、`top_p` 等 sampling 字段，即使某个 workflow
显式传入了这些字段。

Ollama 本地示例：

```yaml
model:
  default:
    preset: ollama
    model: llama3.2
    # 可选。留空时使用 http://127.0.0.1:11434。
    base_url: ""
```

Ollama adapter 会把 `params.max_tokens` 映射为原生 `options.num_predict`，并把显式配置的 `temperature`、`top_p`、
`seed` 和 `stop` 放入 `/api/chat` 的 native `options` object。

用量记录写到本地 audit 状态中，不包含 prompt 文本。

## RAG

默认配置使用本地 `hash` embedding，因此本地索引不需要 provider key：

```yaml
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small
```

远程 embedding 需要显式配置 provider 设置：

```yaml
providers:
  embedding:
    api_key: ""
    base_url: ""

rag:
  backend: jsonl
  embedding_provider: openai-compatible
  embedding_model: ""
```

Embedding provider 名称包括 `hash`、`sparse`、`mock`、`ollama` 和
`openai-compatible`。`hash`、`sparse`、`mock` 是由 RAG core 实现的本地
deterministic/test adapter。`ollama` 和 `openai-compatible` 是由 RAG skill 实现的远程
egress adapter，不属于 `ikaros-rag`。它们都会在 provider 调用前通过 harness 审批；
审批后，RAG skill 会通过 session `NetworkEgress` 执行 embedding HTTP。

外部 memory provider 目前只是 descriptor 元数据。远程 append/search adapter 尚未实现，因此 runtime config load 和
`ikaros config validate` 都会拒绝启用的外部 memory provider。

## 语音

默认配置使用本地 mock voice provider，因此普通模型聊天不需要 TTS 或 ASR 凭证：

```yaml
voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
```

远程 OpenAI-compatible TTS 和 ASR 需要显式配置：

```yaml
providers:
  tts:
    api_key: ""
    base_url: ""
  asr:
    api_key: ""
    base_url: ""

voice:
  tts:
    provider: openai-compatible
    model: ""
    voice: default
  asr:
    provider: openai-compatible
    model: ""
```

Cloud voice provider 只接受 `openai-compatible`；配置的远端服务必须实际提供对应的
TTS 或 ASR endpoint。TTS 文本在 provider 调用前脱敏；输出文件视为 workspace 写入。

## Self-Modify 检查

Self-modify apply 可以使用受限 check profile：

```yaml
self_modify:
  check_profiles:
    runtime_patch:
      commands:
        - cargo check --workspace --all-features
      reason: "Runtime patches must keep the workspace compiling."
```

这些检查不会启用自动 apply。Proposal 仍然需要明确审批。
