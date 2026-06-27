# 配置

Ikaros 默认把本地状态存�?`~/.ikaros`。可以用 `IKAROS_HOME` �?`--ikaros-home` 隔离运行�?

```bash
export IKAROS_HOME=/tmp/ikaros-dev
ikaros --ikaros-home /tmp/ikaros-dev doctor
```

`ikaros init` 会创�?runtime home，并只写一个配置文件：`IKAROS_HOME/config.yaml`。它不会从仓库里�?example 目录读取配置�?

## Schema Version

生成配置会包含顶�?`schema_version: 1`。Runtime 加载�?`ikaros config validate`
都要求显式写出这个字段。缺失或不支持的 schema version 会在 runtime 使用配置前报错�?

## Provider 设置

不要把真�?API key 放进文档、测试、示例或 tracked 文件。本地未跟踪�?
`IKAROS_HOME/config.yaml` 可以直接保存第三�?API key，用于普通运行和 smoke test�?

`ikaros init` 默认只写一份极简模型配置�?

```yaml
schema_version: 1

model:
  default:
    preset: auto
    model: ""
    api_key: ""
    base_url: ""
```

常见的单模型场景只需要填 `model.default.model`、`api_key` �?`base_url`。`preset: auto`
会保�?runtime provider-profile 自动检测；已知 provider 时可以改�?`kimi`、`openai`、`anthropic` �?
`ollama` 等具�?preset。Preset 会在配置加载时展开�?provider family、wire transport �?compatibility profile�?
所以大多数用户不需要手�?`provider`、`transport` �?`compat_profile`�?

`ikaros init --full` 会写出完整默�?YAML，包�?provider、agent profile、memory、RAG、voice、gateway �?
execution 等可选配置段。完整配置同样不带注释�?

模型凭证可以写在 `model.default` 下面，也可以写在共享�?`providers.model` 下面�?
Inline �?`model.default.api_key` �?`model.default.base_url` 优先；inline 为空时回退�?
`providers.model`。Fallback 模型条目如果自己写了 `api_key` �?`base_url` 就使用自己的值，否则继承共享�?
model provider 设置�?

`providers.embedding`、`providers.tts`、`providers.asr` �?`providers.search` 仍然是对应资源类型的共享
provider 设置。明�?key 只应存在于本�?runtime home，不应提交进仓库�?

`providers.search` �?`web_search` 提供默认 key �?endpoint。内�?`duckduckgo-html`
provider 可以�?`base_url` 为空时运行；Brave、Bing、SerpAPI �?Tavily 风格 provider
可以使用 `providers.search.api_key` �?`providers.search.base_url`，也可以在命令里通过
`ikaros web search --provider ... --endpoint ... --api-key ...` 覆盖�?

`ikaros setup` 会写入同一批字段，并且不会打印 secret 值。如果同一�?
OpenAI-compatible endpoint 同时提供模型、embedding、TTS �?ASR，可以配合对应的
resource model 参数使用 `--reuse-model-provider-for-embedding`�?
`--reuse-model-provider-for-tts` �?`--reuse-model-provider-for-asr`。交互式 setup
会在模型 provider 配好后询问是否复用这�?key �?base URL�?

## 配置校验

编辑本地 runtime 配置后运行：

```bash
ikaros config validate
ikaros config show
```

普�?runtime 加载配置和显�?`config validate` 会共用同一�?shape 与语义校验，并在返回 `IkarosConfig` 前拒绝未知字段、非�?
provider/runtime/transport/backend 组合、缺失的 key、URL、模型名，以�?descriptor-only 的外�?memory provider。输出只使用
`providers.model.api_key` 这类字段路径说明缺失或非法，不会打印 secret 值�?

自动化场景使用：

```bash
ikaros config validate --json
ikaros config show --json
```

`config show` 只输出脱�?runtime 摘要：provider family、模型名、存�?backend�?
execution 设置，以�?`model_api_key_configured` 这类布尔值。它不会打印明文 credential
�?base URL。JSON 模式只向 stdout 写报告。配置无效时 validate 仍返回非零退出码�?
但报告可稳定读取 `valid`、`path`、`errors[]` �?`warnings[]`�?

## 执行边界

Runtime session 会通过 `ikaros-host` �?`execution` 段创�?`ExecutionEnv`�?

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

共享�?`ExecutionEnv`、filesystem、process �?network trait 位于 `ikaros-execution::toolkit`�?
具体�?local、dry-run、Docker、workspace-scoped �?network-governed 实现位于
`ikaros-execution::sandbox`�?

`network.enabled` 打开 HTTP egress backend。网络出口仍然默认拒绝：
`allow_provider_hosts` 会把 active model provider、agent instance 模型 provider
覆盖、embedding/TTS/ASR/search provider、内�?web-search provider 默认值和本地
Ollama 默认地址解析出的精确 host 放入 allowlist；`allowed_hosts` 用于后续
network-capable tool 的额外精�?host。这里写 host name，不写完�?URL �?
`host:port`�?
网络出口会拒绝非 HTTP scheme、私�?链路本地/组播 IP 字面量，并关闭自动重定向�?
有效 provider base URL 包括 inline �?`model.default.base_url`、共享的
`providers.*.base_url`、per-agent instance 模型 provider 覆盖，以及本�?Ollama
默认地址。内�?web-search 默认值当前覆�?DuckDuckGo HTML、Brave、Bing、SerpAPI
�?Tavily 风格 endpoint。显式配置的 loopback host 仍可用于 Ollama 这类本地
provider；普通域名如果解析到受限地址，HTTP egress transport 会拒绝请求。解析出�?
socket address 会在验证�?pin 到本�?HTTP client，避免同一次请求再做第二次独立
DNS lookup。这�?runtime guardrail，不是完�?OS-level 网络沙箱�?

`sandbox.backend` 支持 `local`、`dry-run` �?`docker`。Local session 使用
workspace-scoped filesystem/process 环境和受�?HTTP egress。Dry-run session
保留读取能力，但只跳过文件写入和进程执行。Docker session 会通过 `docker run`
执行进程，把 workspace bind mount 到容器内 `/workspace`，把进程 cwd 映射到该
mount 内，并用 `--network none` 启动容器；`sandbox.image` 用来选择容器镜像�?
文件读写仍经�?workspace-scoped `ExecutionEnv`，provider HTTP egress 仍使用进程容器外�?
governed runtime transport。网络出口由 `execution.network.enabled` �?host allowlist
单独控制；如�?dry-run 也必须避免网络副作用，需要把 `network.enabled` 设为 false�?
`read_scope` 目前固定�?`workspace`；已有路径会 canonicalize，因此读写都会拒�?symlink 逃逸�?

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

当前 client 切片只支�?`stdio` transport。配置里�?server 不会被当成可信本地命令：
`ikaros mcp probe <id>` 会通过 harness process 边界启动它，因此仍使�?policy�?
approval、audit、workspace scope、timeout �?output cap。`include_tools` �?
`exclude_tools` 是对 `tools/list` 结果应用的精确工具名过滤。HTTP MCP transport
目前没有启用；后续必须通过 `NetworkEgress` 实现�?

常用命令�?

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

普�?chat 建议保持 `rag_context` �?false。只有当�?turn 需要带 citation 的本�?
reference snippet 时，再在 profile 中启用或传入 `--rag-top-k`�?
长期 memory search 也需要显式打开。普�?chat 会读取已接受�?memory projection �?session working memory；如果某�?turn 需�?
retrieved memory result，再使用 `--memory-search-limit` �?`memory_search` 工具�?
`toolsets` 控制当前 profile 启用哪些 skill group。只有直接工具面
`core`、`workspace`、`memory` 会进入模�?tool manifest；默�?profile 会启�?
`rag`、`coding`、`voice`、`plugin`，但这些工具集仍保持 deferred。模型可以通过
`tool_search`、`tool_describe`、`tool_call`
发现并显式调�?deferred tool，但 bridge 会拒绝当�?profile 未启�?toolset 里的
deferred tool。目标工具的实际执行仍经�?harness policy、approval �?audit。启用任�?
deferred toolset �?profile 必须同时保留 `core`，因�?bridge tools 属于 `core`�?

使用 profile�?

```bash
ikaros --agent plan chat --message "review only"
ikaros agent run --profile build --dry-run "inspect this repo"
```

Profile 不能绕过破坏性命令、直�?secret 访问、受保护路径、发布动作、workspace 外写入或 self-modify 的硬性拒绝�?

## Agent Instance

Agent instance �?runtime identity。Profile 回答“这�?agent 应该怎样工作”；instance 回答“哪�?agent 正在运行、在哪个
workspace、使用哪�?state �?routing policy”�?

示例�?

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

字段�?

- `profile`：用�?persona 和普通策略行为的 profile overlay�?
- `workspace`：可�?workspace override。省略时使用调用�?workspace�?
- `state_dir`：可�?state dir override。省略时使用 `IKAROS_HOME/agents/<agent_id>`�?
- `toolsets`：可选的模型可见/延迟披露 toolset allowlist override。省略时使用所�?profile �?toolsets。启�?`rag`、`coding`�?
  `voice` �?`plugin` 这类 deferred toolset 时，应保�?`core`，让 `tool_search`、`tool_describe` �?`tool_call`
  可见�?
- `providers.model`：该 identity 的模�?endpoint �?key override。省略时使用顶层 `providers.model`�?
- `model`：该 identity 的完�?`ModelConfig` override。省略时使用 `model.default`�?
- `session_policy.history_scope`：`agent`、`session` �?`workspace`�?
- `session_policy.allow_session_switch`：该身份是否允许 runtime 切换 session�?
- `session_policy.max_parallel_subagents`：并�?delegated work 上限�?
- `session_policy.max_delegation_depth`：嵌�?agent handoff 深度上限；超过上限的请求会在 delegated task 启动前失败�?
- `auth_scope.local_only`：该身份默认是否 local-only�?
- `auth_scope.allow_network`：该身份�?network 默认策略�?
- `route_bindings`：gateway routing 使用�?channel/account/peer/thread 绑定�?

解析规则�?

1. 请求名称先匹�?`agent.instances.<name>`�?
2. 如果没有 instance，则同名解析�?`agent.profiles.<name>`�?
3. 如果调用方没有传名称，则使用 `agent.default`�?

审批和审计记录应使用解析后的 instance `agent_id`，而不只是 profile name�?

Chat、TUI、coding model loop、task agent-loop、`doctor` 以及 `provider
inspect|health|matrix` 都会通过当前 `AgentInstance` 解析模型设置�?
Embedding、TTS �?ASR 仍使用全局资源配置，除非对�?runtime 路径显式增加
instance override�?

## 本地 Store

JSONL 是默认后端。较大的本地 store 可以使用 SQLite�?

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

Agent `state.db` session store 是权威聊�?timeline。普�?chat turn 不再写单独的
chat history mirror。历史、搜索、replay、context assembly �?workbench 视图都从
session replay 投影�?

Memory policy 字段�?

- `promote_threshold`：综合分数达到或超过该值时，记�?`promote` action，并�?record 标记 policy-promoted�?
- `demote_threshold`：综合分数低于或等于该值时，记�?`demote` action，并�?record 标记 policy-demoted�?
- `forget_threshold`：综合分数低于或等于该值时，记�?`forget` action，并删除低分 record�?
- `max_records_per_scope`：每�?kind/scope 的保留上限。某轮写入导�?scope
  超限时，会删除最低分 record，并�?quota reason 记录�?`forget` action�?

主要本地路径�?

- `IKAROS_HOME/memory/`
- `IKAROS_HOME/rag/`
- `IKAROS_HOME/audit/`
- `IKAROS_HOME/automation/`
- `IKAROS_HOME/gateway/`
- `IKAROS_HOME/skills/`

## 模型 Provider

极简配置�?`preset: auto` 和空�?key、URL、模型名开始。远程模型调用会在网络请求之前检查必要字段，缺少字段会直接报错�?
支持�?preset ID �?`model-providers.md`；支持的 provider family 包括 `openai-compatible`、`anthropic`、`ollama`
�?`mock`�?

Runtime chat、task agent loop、provider-backed coding command �?provider-backed RAG embedding skill �?
provider 调用现在都经�?session environment，不再直接使用裸 HTTP client。用 `ikaros provider health` 查看本地 health
ledger，用 `ikaros provider health --live` 执行真实 provider health probe�?

使用 inline 凭证的单 provider OpenAI-compatible 示例�?

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

Preset 名称是面向用户的 provider 捷径。Provider 名称表示 adapter family，不表示厂商。任�?Chat
Completions-compatible 服务都使�?OpenAI-compatible preset，具�?endpoint 和模型写�?
`model.default.base_url` �?`model.default.model`。多 provider 配置可以把共享模�?key �?endpoint 放到
`providers.model`；inline 模型字段仍会覆盖这个共享池�?

`model.default.cost` 是本地价格元数据，供 `provider inspect`、`provider matrix`�?
workbench cost cell �?usage estimate 使用。Ikaros 不硬编码实时厂商价格。需要成本估算时�?
把当�?provider 账号里的 `currency`、`input_per_million`、`output_per_million`�?
`cache_read_per_million` �?`cache_write_per_million` 填进去；未知字段可以保持 `null`�?
cache read/write 未配置时会按普�?input 价格估算�?

Ollama 也可以作为本�?embedding provider�?

```yaml
providers:
  embedding:
    api_key: ""
    # 可选。留空时使用 http://127.0.0.1:11434�?
    base_url: ""

rag:
  embedding_provider: ollama
  embedding_model: nomic-embed-text
```

`max_retries` 控制 provider 外层 governance retry policy，用�?rate limit、瞬时服务端错误�?network failure 这类
retryable failure。鉴权、bad request �?context-limit failure �?terminal。默认策略使用较短且有上限的指数退避；它和
OpenAI-compatible adapter 内部那次 unsupported-parameter retry 是两套机制�?

`compat_profile` 控制 OpenAI-compatible adapter 内部�?provider/model 请求差异。Preset 会自动填这个字段�?
`auto` 会先按有效模�?base URL 匹配，再按模型名 hint 匹配，最后回退�?`generic`。支持显式指定：

- `generic`：只发送标�?Chat Completions 字段�?
- `moonshot-kimi`：Kimi/Moonshot。省�?`temperature`，缺�?`max_tokens`
  时默�?`32000`，发�?Kimi thinking 字段，并�?tool schema 修正�?
  Moonshot 更严格的 JSON Schema 子集�?
- `deepseek`：对 `deepseek-reasoner` �?`deepseek-v4+` 模型发�?DeepSeek
  thinking 字段；`deepseek-chat` V3 保持普�?chat 形态�?
- `gemini-openai`：只�?Gemini family 模型�?reasoning 配置映射�?Gemini
  OpenAI-compatible �?`extra_body.google.thinking_config`�?
- `openrouter`：保�?OpenRouter routing 字段，并避免给现�?Claude route 发送无�?reasoning 字段�?
- `qwen`：Qwen/DashScope 兼容请求形态。会�?message content 规范化为
  text parts，给 system prompt 的最后一段加 ephemeral cache 标记，启用高分辨�?
  图片字段，并在缺�?`max_tokens` 时默认使�?`65536`�?
- `local-openai-compatible`：用�?LM Studio、vLLM、SGLang 等本�?
  Chat Completions 服务，默认更保守；缺�?`max_tokens` 时使�?`65536`�?
  避免本地服务默认输出过短�?

Moonshot sanitizer 只修正发�?provider 的请�?payload，不会修改注册表里的 tool
schema。具体修正规则见 [模型 Provider](model-providers.md)�?

OpenAI-compatible �?profile 值只�?`provider: openai-compatible` 有效。原�?
provider 接受 `auto`、`generic`，或 preset 展开出的 native profile
（`anthropic-native`、`ollama-native`、`mock`）�?

对可选数值型 `params` 字段，`null` 表示 adapter 不发送该参数，除非选中�?
profile 提供 provider 默认值。当前支持：

- `max_tokens`：正数输�?token 上限�?
- `temperature`：采样温度，校验范围是闭区间 `0.0..2.0`�?
- `top_p`：nucleus sampling，校验范围是闭区�?`0.0..1.0`�?
- `n`：provider 支持时请求的正数 completion 数量�?
- `presence_penalty` �?`frequency_penalty`：校验范围是闭区�?`-2.0..2.0`�?
- `seed`：provider 支持时使用的确定�?seed�?
- `stop`：非�?stop sequence 列表，列表元素不能为空字符串�?

`reasoning.effort` 可取 `none`、`minimal`、`low`、`medium`、`high`、`xhigh` �?`max`。Runtime 可以为特�?workflow
设置 per-call options，但 strict profile 仍会移除或改写目�?provider 会拒绝的字段。`extra_body` �?JSON object，会�?common
params 之后、profile-specific shaping 之前合并�?provider 请求体；日志和审计只能记录脱敏摘要，不能写入原始 secret-like 值�?

每日 token 预算预检查会使用配置�?per-call �?`max_tokens`。如果选中�?
OpenAI-compatible profile 在缺�?`max_tokens` 时提供默认值，
例如 `moonshot-kimi`、`qwen` �?`local-openai-compatible`，这�?profile 默认输出上限也会进入预算估算�?

如果 OpenAI-compatible provider 明确返回 `temperature` 或可省略�?`max_tokens`
不支持，adapter 会删除该字段并只重试一�?HTTP 请求�?
鉴权、额度、网络和普通参数校验错误不会走这个重试路径�?

Anthropic 示例�?

```yaml
model:
  default:
    preset: anthropic
    model: claude-sonnet-4-5
    api_key: "replace-with-anthropic-key"
    base_url: "https://api.anthropic.com"
```

Anthropic adapter 总会发送正�?`max_tokens`。当 `model.default.reasoning`
启用 thinking 时，现代 Claude 模型使用 adaptive thinking �?
`output_config.effort`；旧 Claude 模型使用 budget-based thinking。Claude
4.7 及更新模型会省略 `temperature`、`top_p` �?sampling 字段，即使某�?workflow
显式传入了这些字段�?

Ollama 本地示例�?

```yaml
model:
  default:
    preset: ollama
    model: llama3.2
    # 可选。留空时使用 http://127.0.0.1:11434�?
    base_url: ""
```

Ollama adapter 会把 `params.max_tokens` 映射为原�?`options.num_predict`，并把显式配置的 `temperature`、`top_p`�?
`seed` �?`stop` 放入 `/api/chat` �?native `options` object�?

用量记录写到本地 audit 状态中，不包含 prompt 文本�?

## RAG

默认配置使用本地 `hash` embedding，因此本地索引不需�?provider key�?

```yaml
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small
```

远程 embedding 需要显式配�?provider 设置�?

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

Embedding provider 名称包括 `hash`、`sparse`、`mock`、`ollama` �?
`openai-compatible`。`hash`、`sparse`、`mock` 是由 RAG core 实现的本�?
deterministic/test adapter。`ollama` �?`openai-compatible` 是由 RAG skill 实现的远�?
egress adapter，不属于 `ikaros-state::rag`。它们都会在 provider 调用前通过 harness 审批�?
审批后，RAG skill 会通过 session `NetworkEgress` 执行 embedding HTTP�?

外部 memory provider 目前只是 descriptor 元数据。远�?append/search adapter 尚未实现，因�?runtime config load �?
`ikaros config validate` 都会拒绝启用的外�?memory provider�?

## 语音

默认配置使用本地 mock voice provider，因此普通模型聊天不需�?TTS �?ASR 凭证�?

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

远程 OpenAI-compatible TTS �?ASR 需要显式配置：

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

Cloud voice provider 只接�?`openai-compatible`；配置的远端服务必须实际提供对应�?
TTS �?ASR endpoint。TTS 文本�?provider 调用前脱敏；输出文件视为 workspace 写入�?

## Self-Modify 检�?

Self-modify apply 可以使用受限 check profile�?

```yaml
self_modify:
  check_profiles:
    runtime_patch:
      commands:
        - cargo check --workspace --all-features
      reason: "Runtime patches must keep the workspace compiling."
```

这些检查不会启用自�?apply。Proposal 仍然需要明确审批�?
