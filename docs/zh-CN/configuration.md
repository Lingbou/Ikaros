# 配置

Ikaros 默认把本地状态存到 `~/.ikaros`。可以用 `IKAROS_HOME` 或 `--ikaros-home` 隔离运行：

```bash
export IKAROS_HOME=/tmp/ikaros-dev
ikaros --ikaros-home /tmp/ikaros-dev doctor
```

`ikaros init` 会创建 runtime home，并只写一个配置文件：`IKAROS_HOME/config.yaml`。它不会从仓库里的 example 目录读取配置。

## Provider 设置

不要把真实 API key 放进文档、测试、示例或 tracked 文件。本地未跟踪的 `IKAROS_HOME/config.yaml` 可以直接保存第三方 API key，用于普通运行和 smoke test。

`ikaros init` 生成的配置把外部资源设置放在文件最上方。每个远程 API provider 都有 `api_key` 和 `base_url`；模型名称放在真正发请求的功能段落里。

```yaml
providers:
  model:
    api_key: ""
    base_url: ""
  embedding:
    api_key: ""
    base_url: ""
  tts:
    api_key: ""
    base_url: ""
  asr:
    api_key: ""
    base_url: ""
```

`providers.*` 是 schema-only 的凭证和 endpoint 区域。它不会被合并进 `model.default`、`rag` 或 `voice` 结构；runtime 会把对应的 provider 设置和选择 provider family、transport、model、timeout、budget 的功能配置一起传给 model、embedding、TTS、ASR factory。

Provider 设置只从这个区域读取。明文 key 只应存在于本机 runtime home，不应提交进仓库。生成配置直接使用这些本地明文字段。

## 配置校验

编辑本地 runtime 配置后运行：

```bash
ikaros config validate
```

普通 runtime 加载配置时已经会检查 YAML 形状，并在返回 `IkarosConfig` 前拒绝未知字段。显式 `config validate` 会复用同一套 shape check，并额外校验 provider/runtime/transport/backend 组合、缺失的 key、URL、模型名，以及 descriptor-only 的外部 memory provider。输出只使用 `providers.model.api_key` 这类字段路径说明缺失或非法，不会打印 secret 值。

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
    plan:
      mode: plan
      workspace_writes: deny
      shell: ask
      network: ask
      memory_context: true
      rag_context: false
```

普通 chat 建议保持 `rag_context` 为 false。只有当前 turn 需要带 citation 的本地 reference snippet 时，再在 profile 中启用或传入 `--rag-top-k`。

使用 profile：

```bash
ikaros --agent plan chat --message "review only"
ikaros agent run --profile build --dry-run "inspect this repo"
```

Profile 不能绕过破坏性命令、直接 secret 访问、受保护路径、发布动作、workspace 外写入或 self-modify 的硬性拒绝。

## Agent Instance

Agent instance 是 runtime identity。Profile 回答“这个 agent 应该怎样工作”；instance 回答“哪个 agent 正在运行、在哪个 workspace、使用哪个 state 和 routing policy”。

示例：

```yaml
agent:
  instances:
    repo-build:
      profile: build
      workspace: /home/user/src/project
      state_dir: /home/user/.ikaros/agents/repo-build
      session_policy:
        history_scope: workspace
        allow_session_switch: true
        max_parallel_subagents: 4
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
- `session_policy.history_scope`：`agent`、`session` 或 `workspace`。
- `session_policy.allow_session_switch`：该身份是否允许 runtime 切换 session。
- `session_policy.max_parallel_subagents`：并发 delegated work 上限。
- `auth_scope.local_only`：该身份默认是否 local-only。
- `auth_scope.allow_network`：该身份的 network 默认策略。
- `route_bindings`：gateway routing 使用的 channel/account/peer/thread 绑定。

解析规则：

1. 请求名称先匹配 `agent.instances.<name>`。
2. 如果没有 instance，则同名解析为 `agent.profiles.<name>`。
3. 如果调用方没有传名称，则使用 `agent.default`。

审批和审计记录应使用解析后的 instance `agent_id`，而不只是 profile name。

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

chat_history:
  backend: sqlite

rag:
  backend: sqlite
  embedding_provider: hash
```

Memory policy 字段：

- `promote_threshold`：综合分数达到或超过该值时，记录 `promote` action，并给 record 标记 policy-promoted。
- `demote_threshold`：综合分数低于或等于该值时，记录 `demote` action，并给 record 标记 policy-demoted。
- `forget_threshold`：综合分数低于或等于该值时，记录 `forget` action，并删除低分 record。
- `max_records_per_scope`：每个 kind/scope 的保留上限。某轮写入导致 scope 超限时，会删除最低分 record，并以 quota reason 记录为 `forget` action。

主要本地路径：

- `IKAROS_HOME/memory/`
- `IKAROS_HOME/chat/`
- `IKAROS_HOME/rag/`
- `IKAROS_HOME/audit/`
- `IKAROS_HOME/automation/`
- `IKAROS_HOME/gateway/`
- `IKAROS_HOME/skills/`

## 模型 Provider

生成配置使用协议级 `openai-compatible` provider，key、URL 和模型名为空。模型调用会在网络请求之前检查这些字段，缺少任何必要字段都会直接报错。

支持的模型 provider 名称包括 `mock`、`openai-compatible`、`anthropic` 和 `ollama`。

OpenAI-compatible 示例：

```yaml
providers:
  model:
    api_key: "replace-with-provider-key"
    base_url: "https://api.example.com/v1"

model:
  default:
    provider: openai-compatible
    transport: openai-compatible-chat-completions
    model: provider-model-id
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

Provider 名称表示 adapter family，不表示厂商。任何 Chat Completions-compatible 服务都使用 `openai-compatible`，具体 endpoint 和模型写在 `providers.model.base_url` 和 `model.default.model`。

`compat_profile` 控制 OpenAI-compatible adapter 内部的 provider/model 请求差异。`auto` 会先按 `providers.model.base_url` 匹配，再按模型名 hint 匹配，最后回退到 `generic`。支持显式指定：

- `generic`：只发送标准 Chat Completions 字段。
- `moonshot-kimi`：Kimi/Moonshot。省略 `temperature`，缺少 `max_tokens` 时默认 `32000`，发送 Kimi thinking 字段，并把 tool schema 修正到 Moonshot 更严格的 JSON Schema 子集。
- `deepseek`：对 `deepseek-reasoner` 和 `deepseek-v4+` 模型发送 DeepSeek thinking 字段；`deepseek-chat` V3 保持普通 chat 形态。
- `gemini-openai`：只对 Gemini family 模型把 reasoning 配置映射为 Gemini OpenAI-compatible 的 `extra_body.google.thinking_config`。
- `openrouter`：保留 OpenRouter routing 字段，并避免给现代 Claude route 发送无效 reasoning 字段。
- `qwen`：Qwen/DashScope 兼容请求形态。会把 message content 规范化为 text parts，给 system prompt 的最后一段加 ephemeral cache 标记，启用高分辨率图片字段，并在缺少 `max_tokens` 时默认使用 `65536`。
- `local-openai-compatible`：用于 LM Studio、vLLM、SGLang 等本地 Chat Completions 服务，默认更保守；缺少 `max_tokens` 时使用 `65536`，避免本地服务默认输出过短。

Provider-specific 的 `compat_profile` 只对 `provider: openai-compatible` 有效。原生 `anthropic`、`ollama` 和 `mock` provider 只接受 `auto` 或 `generic`。

对可选数值型 `params` 字段，`null` 表示 adapter 不发送该参数，除非选中的 profile 提供 provider 默认值。当前支持：

- `max_tokens`：正数输出 token 上限。
- `temperature`：采样温度，校验范围是闭区间 `0.0..2.0`。
- `top_p`：nucleus sampling，校验范围是闭区间 `0.0..1.0`。
- `n`：provider 支持时请求的正数 completion 数量。
- `presence_penalty` 和 `frequency_penalty`：校验范围是闭区间 `-2.0..2.0`。
- `seed`：provider 支持时使用的确定性 seed。
- `stop`：非空 stop sequence 列表，列表元素不能为空字符串。

`reasoning.effort` 可取 `none`、`minimal`、`low`、`medium`、`high`、`xhigh` 或 `max`。Runtime 可以为特定 workflow 设置 per-call options，但 strict profile 仍会移除或改写目标 provider 会拒绝的字段。`extra_body` 是 JSON object，会在 common params 之后、profile-specific shaping 之前合并进 provider 请求体；日志和审计只能记录脱敏摘要，不能写入原始 secret-like 值。

每日 token 预算预检查会使用配置或 per-call 的 `max_tokens`。如果选中的 OpenAI-compatible profile 在缺少 `max_tokens` 时提供默认值，例如 `moonshot-kimi`、`qwen` 或 `local-openai-compatible`，这个 profile 默认输出上限也会进入预算估算。

如果 OpenAI-compatible provider 明确返回 `temperature` 或可省略的 `max_tokens` 不支持，adapter 会删除该字段并只重试一次 HTTP 请求。鉴权、额度、网络和普通参数校验错误不会走这个重试路径。

Anthropic 示例：

```yaml
providers:
  model:
    api_key: "replace-with-anthropic-key"
    base_url: "https://api.anthropic.com/v1"

model:
  default:
    provider: anthropic
    model: claude-sonnet-4-5
```

Anthropic adapter 总会发送正数 `max_tokens`。当 `model.default.reasoning`
启用 thinking 时，现代 Claude 模型使用 adaptive thinking 和
`output_config.effort`；旧 Claude 模型使用 budget-based thinking。Claude
4.7 及更新模型会省略 `temperature`、`top_p` 等 sampling 字段，即使某个 workflow
显式传入了这些字段。

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
```

Ollama adapter 会把 `params.max_tokens` 映射为原生 `options.num_predict`，并把显式配置的 `temperature`、`top_p`、`seed` 和 `stop` 放入 `/api/chat` 的 native `options` object。

用量记录写到本地 audit 状态中，不包含 prompt 文本。

## RAG

生成配置为远程 embedding 使用同一套 provider 形状：

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

如果要完全本地索引，不使用 provider 凭证，需要显式选择本地 embedding adapter：

```yaml
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small
```

Cloud embedding 使用 OpenAI-compatible 形状，并在 provider 调用前通过 harness 审批。支持的 embedding provider 名称包括 `hash`、`sparse`、`mock` 和 `openai-compatible`。

外部 memory provider 目前只是 descriptor 元数据。远程 append/search adapter 尚未实现，因此 `ikaros config validate` 会拒绝启用的外部 memory provider。

## 语音

生成配置为远程 OpenAI-compatible TTS 和 ASR 预留空的凭证和模型名：

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

离线测试可以显式选择 `mock`。Cloud voice provider 只接受 `openai-compatible`；配置的远端服务必须实际提供对应的 TTS 或 ASR endpoint。TTS 文本在 provider 调用前脱敏；输出文件视为 workspace 写入。

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
