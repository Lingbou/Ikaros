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

加载配置时，`providers.model` 会注入 `model.default`，`providers.embedding` 会注入 `rag`，`providers.tts` 和 `providers.asr` 会分别注入 `voice.tts`、`voice.asr`。

Provider 设置只从这个区域读取。明文 key 只应存在于本机 runtime home，不应提交进仓库。生成配置直接使用这些本地明文字段。

## 配置校验

编辑本地 runtime 配置后运行：

```bash
ikaros config validate
```

校验器读取 `IKAROS_HOME/config.yaml`，检查 YAML 形状，拒绝未知字段，校验 provider/runtime/transport/backend 组合，并在远程调用前报告缺失的 key、URL 和模型名。输出只使用 `providers.model.api_key` 这类字段路径说明缺失或非法，不会打印 secret 值。

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
      rag_context: true
    plan:
      mode: plan
      workspace_writes: deny
      shell: ask
      network: ask
      memory_context: true
      rag_context: true
```

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

chat_history:
  backend: sqlite

rag:
  backend: sqlite
  embedding_provider: hash
```

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
    model: provider-model-id
    rate_limit_per_minute: 60
    daily_token_budget: 100000
```

Provider 名称表示 adapter family，不表示厂商。任何 Chat Completions-compatible 服务都使用 `openai-compatible`，具体 endpoint 和模型写在 `providers.model.base_url` 和 `model.default.model`。

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
```

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
