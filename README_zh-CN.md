# Ikaros

[English](README.md) | [文档](docs/README.md)

Ikaros 是一个早期阶段的本地优先 agent runtime，使用 Rust 编写。它把
persona、记忆、RAG、模型提供方、工具执行、策略审批和审计日志分开，便于维护和扩展。

这个仓库目前是一个 pre-MVP 本地 runtime，适合本地开发和实验。它还不是稳定产品，也不承诺稳定 API。

`ikaros init` 会创建一份极简的本地 `config.yaml`，里面只包含 inline 模型字段。
远程模型调用会在本地填好 `api_key`、`base_url` 和 `model` 之前直接报错。
RAG embedding 默认保持本地 `hash`，语音默认保持 `mock`，除非 setup 或本地配置显式修改。

## 能做什么

- 提供本地 CLI 和 terminal workbench：聊天、session replay、context/memory/RAG
  检查、计划任务、消息入口、审批、插件、coding turn 报告、代码审查辅助和受控编辑。
- 暴露第一版本地 API、MCP、browser/CDP、web search/extract、vision、image generation
  和多模态 attachment 界面，并复用同一套本地 runtime 边界。
- 默认把记忆、聊天历史、RAG 索引、自动化元数据、消息网关、审批和审计日志保存在本地。
- 所有工具执行都经过 harness 层，包含策略判断、审批请求、审计事件、dry-run 和 guardrail。
- 实现 OpenAI-compatible、Anthropic-compatible 和 Ollama 模型 adapter，
  以及本地 RAG embedding、受 harness 治理的远程 RAG embedding egress 和
  OpenAI-compatible TTS/ASR adapter。mock provider 只在显式选择时用于离线测试。
- 支持 `build`、`plan`、`general` 这类 agent profile，用来调整上下文和普通策略行为，但不能绕过硬性安全规则。

## 仓库结构

- `crates/ikaros-core`：共享配置、路径、任务状态、脱敏、错误和 agent profile 类型。
- `crates/ikaros-automation`：本地计划任务元数据和运行状态。
- `crates/ikaros-body`：可替换的 body/status/frame 合约和 dashboard 渲染。
- `crates/ikaros-cli`：`ikaros` 命令行程序。
- `crates/ikaros-coding`：仓库扫描、受控 patch、结构化 patch failure、turn diff
  tracking、代码审查、coding turn report、自修改记录和测试命令分析。
- `crates/ikaros-context`：context bundle、prompt section、reference、
  provider-aware token budget、token estimator、quota-based compaction 和
  diff primitive。
- `crates/ikaros-gateway`：本地消息 inbox/outbox 元数据和投递路由。
- `crates/ikaros-harness`：策略引擎、审批队列、审计日志、技能执行 session、插件和任务 runner。
- `crates/ikaros-host`：host 侧组装层，负责 runtime location、agent instance、
  execution session、skill registry 和受治理网络出口。
- `crates/ikaros-memory`：本地 JSONL 和 SQLite 记忆存储、lifecycle hook 和 policy journal primitive。
- `crates/ikaros-mcp`：受 harness 托管的 MCP stdio server 和一次性 MCP stdio probe
  primitive。
- `crates/ikaros-models`：mock、OpenAI-compatible、Anthropic 和 Ollama 模型
  provider，以及 provider profile、context profile、registry metadata、治理、
  retry policy、health state 和用量日志。
- `crates/ikaros-protocol`：API、TUI、gateway、replay 和外部界面共享的版本化协议类型。
- `crates/ikaros-rag`：本地 RAG 摄取、索引、检索和本地 embedding primitive。远程
  embedding HTTP 属于受 harness 治理的 RAG skill，不属于这个 core crate。
- `crates/ikaros-runtime`：聊天、任务、计划任务、消息 drain、body 状态、诊断和 agent handoff 编排。
- `crates/ikaros-session`：session id、turn id、typed agent event、按 turn 的 session
  写入、SQLite `state.db`、append-only session entry 和 replay 查询。
- `crates/ikaros-service`：本地 worker 进程的 service manager 模板渲染。
- `crates/ikaros-skills`：文件系统、shell/git、记忆、RAG、语音、代码、persona、插件和
  progressive-disclosure tool bridge 等内置技能。
- `crates/ikaros-soul`：persona、情绪、语气和关系模型基础类型。
- `crates/ikaros-voice`：TTS 和 ASR provider 抽象，包含 mock 和 OpenAI-compatible 实现。
- `docs/`：按语言分组的设计文档和子系统文档。

## 快速开始

```bash
cargo run -p ikaros-cli -- init
cargo run -p ikaros-cli -- setup --interactive
cargo run -p ikaros-cli -- setup \
  --api-key "$MODEL_API_KEY" \
  --base-url https://api.example.com/v1 \
  --model provider-model-id \
  --reuse-model-provider-for-embedding \
  --embedding-model provider-embedding-model
cargo run -p ikaros-cli -- config validate
cargo run -p ikaros-cli -- config show
cargo run -p ikaros-cli -- provider inspect
cargo run -p ikaros-cli -- provider health
cargo run -p ikaros-cli -- provider matrix
cargo run -p ikaros-cli -- provider profiles
cargo run -p ikaros-cli -- doctor
cargo run -p ikaros-cli --
ikaros
cargo run -p ikaros-cli -- workbench
cargo run -p ikaros-cli -- chat
cargo run -p ikaros-cli -- chat --message "hello"
```

默认入口就是 fullscreen TUI：安装后运行 `ikaros`，或在源码 checkout 中运行
`cargo run -p ikaros-cli --`。普通本地交互建议从这里开始：聊天、查看当前
session、检查 context 和 memory、处理工具审批，以及在终端里跑 coding workflow。

使用 `ikaros <PATH>` 可以在另一个 workspace 启动。`--workspace <PATH>` 仍保留给
兼容和脚本使用。脚本需要机器可读 screen/status snapshot 时，使用显式
`ikaros workbench`、`debug` 和 inspect/status 命令，而不是依赖默认 human TUI 的
stdout。

常用 workbench 命令：

- `/status`：查看当前 agent、模型、provider health、预算、gateway 和队列状态。
- `/screen`：渲染可导航的 status/timeline/main/side 面板。
- `/timeline`、`/replay`、`/trace`：从 `state.db` 查看历史 turn 和失败原因。
- `/context`、`/memory`、`/rag`、`/tools`：查看 agent 当前能看到什么、能用哪些工具。
- `/sandbox [--probe]`：检查当前执行隔离、进程、环境和网络诊断。
- `/attach`：把 image、audio 或 file content block 加到下一次 chat turn。
- `/web`、`/browser`、`/vision`、`/image`：使用受治理的 web、CDP、vision 和
  image generation 界面。
- `/provider inspect`、`/provider health`、`/provider matrix`、`/provider debug`：检查
  provider 配置和诊断信息。
- `/approval`：列出或处理待审批请求。
- `/cancel`：取消当前 session 中 queued 或 running continuation。
- `/code plan|apply|test|review|rollback`：运行受治理的 coding workflow。
- `/api status`：查看本地 OpenAI-compatible API 界面。
- `/mcp status`：查看配置的外部 MCP server。
- `/mcp call-http <url> <tool>`：通过当前 session 的 `NetworkEgress` 边界调用
  HTTP MCP tool。

fullscreen TUI 现在是默认终端界面：已有 raw-mode input、鼠标滚轮滚动、
bracketed paste、真实 TTY 下的持久 redraw，以及供显式 screen/debug workflow
使用的确定性结构化 export。真实 fullscreen TTY 中，只读 slash command 会刷新
workbench，不再直接打印 protocol line；`/help` 和 `/commands` 会打开 command
palette。`screen_json`、`screen_mode`、trace 和 status snapshot 仍可通过显式
screen/debug 命令和非 TTY 脚本路径获取。

常用本地工作流：

```bash
cargo run -p ikaros-cli -- memory add "Keep RAG local-first" --kind project --scope ikaros
cargo run -p ikaros-cli -- rag ingest docs --scope project
cargo run -p ikaros-cli -- code workflow "provider coding loop" \
  --mode edit \
  --model-loop \
  --apply-patch \
  --run-tests \
  --max-iterations 2 \
  --test-command "cargo test"
cargo run -p ikaros-cli -- debug trace <session-id>
cargo run -p ikaros-cli -- debug state-db --checkpoint
cargo run -p ikaros-cli -- approval list
cargo run -p ikaros-cli -- mcp status
cargo run -p ikaros-cli -- acp serve --agent build --workspace .
cargo run -p ikaros-cli -- api serve --port 8003
cargo run -p ikaros-cli -- web search "Ikaros runtime"
cargo run -p ikaros-cli -- vision describe screenshots/workbench.png
cargo run -p ikaros-cli -- image generate "small local-first agent logo"
```

使用 `IKAROS_HOME=/custom/path` 或 `--ikaros-home /custom/path` 可以隔离本地状态。
默认状态目录是 `~/.ikaros`。

## 配置

`ikaros init` 会创建 `IKAROS_HOME/config.yaml`。默认文件刻意保持很小：

```yaml
schema_version: 1

model:
  default:
    preset: auto
    model: ""
    api_key: ""
    base_url: ""
```

常见的单模型场景只需要填 `model.default.model`、`model.default.api_key` 和
`model.default.base_url`。`preset: auto` 会保留 provider-profile 自动检测；已知
provider 时可以改成 `kimi`、`openai`、`anthropic` 或 `ollama` 等具体 preset。

如果希望一开始就看到完整默认配置，可以用 `ikaros init --full`。完整文件会包含
provider pool、agent profile、memory、RAG、voice、gateway 和 execution 等配置段。

`ikaros setup --interactive` 会交互式询问首次配置字段；同一批字段也可以用
`ikaros setup --api-key ... --base-url ... --model ...` 直接传入。如果当前文件仍是
极简配置，setup 会先把它展开成完整 YAML，再写入 provider/resource 字段。它只会把
明文 provider key 写进本地配置文件，embedding 默认保持本地 `hash`，TTS/ASR 在没有
显式提供三元组时保持 `mock`，写完后校验配置，并且不会把 key 打印到终端。
如果同一个 OpenAI-compatible endpoint 同时提供多个资源，可以使用
`--reuse-model-provider-for-embedding`、`--reuse-model-provider-for-tts` 或
`--reuse-model-provider-for-asr`，并配合对应的资源模型参数，避免重复填写同一组 key
和 base URL。

普通 chat 默认注入已接受的 memory projection、最近历史和当前 session working
memory。长期 memory search 需要显式通过 `memory_search` 工具或
`--memory-search-limit` 打开；RAG 是带 citation 的 reference retrieval，除非 profile
显式启用或用户传入 `--rag-top-k`，否则不会自动注入。

权威聊天 timeline 是 agent `state.db` session store。普通 chat turn 只把
user/assistant entry 和 typed event 写到这里；历史、搜索、replay 和 workbench 视图都从
session replay 投影出来。

`ikaros mcp serve-stdio` 会通过最小 MCP stdio JSON-RPC server 暴露当前 agent 已启用的
skill。它不会绕过 Ikaros policy：`tools/call` 使用和普通工具执行相同的
`ExecutionSession`、approval、audit、workspace scope 和 `ExecutionEnv` 路径。

`ikaros mcp status` 会列出 `mcp.servers` 下配置的外部 MCP server。
`ikaros mcp probe <id>` 会探测一个已配置的 stdio server，并应用它的 include/exclude
工具过滤；未启用的条目需要传 `--force`。

`ikaros mcp probe-stdio <command> -- <args...>` 是第一版 MCP client 切片。它会通过
harness process 边界启动一个 stdio MCP server，发送 `initialize` 和 `tools/list`，
然后输出脱敏后的 capability report。它目前是一次性 probe，并会被视为任意本地
进程启动，因此默认 policy 可能要求审批；持久 client 生命周期管理仍是后续工作。

`ikaros api serve` 会启动 loopback-only 的 OpenAI-compatible API 切片，面向本地
client 暴露 chat completions、Responses、embeddings、image generation、speech、
transcription、model discovery、health 和 Ikaros protocol metadata。请求仍然使用当前
agent、session store、audit log、provider governance 和 network egress 边界。

`ikaros web`、`ikaros browser`、`ikaros vision`、`ikaros image` 和 chat attachment
是本地优先的集成界面。网络请求仍走 `NetworkEgress`；本地文件和生成产物仍受
workspace 或 `IKAROS_HOME` 边界约束。

可以在 `~/.ikaros/config.yaml` 中切换到 SQLite：

```yaml
memory:
  backend: sqlite

rag:
  backend: sqlite
  embedding_provider: hash
```

不要把真实 API key 写进这个仓库。远程 provider 应保存在
`~/.ikaros/config.yaml` 或其他本地 `IKAROS_HOME/config.yaml` 中。编辑后运行
`ikaros config validate`。

校验会报告缺 key、URL、模型名、非法 backend、未知字段和 descriptor-only 的外部
memory provider，且不会打印 secret 值。`ikaros config show` 会输出脱敏 runtime
摘要，包括 provider family、模型名、存储 backend、execution 设置，以及
credential/endpoint 的 `*_configured` 布尔值。

自动化可以使用 `ikaros config validate --json` 和 `ikaros config show --json`；
配置无效时 validate 仍返回非零退出码，但 stdout 是包含 `valid`、`errors` 和
`warnings` 的机器可读报告。

## 安全模型

Ikaros 把本地工具执行视为需要策略约束的操作：

- harness 范围内的安全读取默认允许。
- 工作区写入、shell 写操作、网络调用和疑似 secret 路径会经过策略判断，必要时返回审批请求。
- 破坏性命令、直接 secret 访问、发布动作和普通自修改默认拒绝。
- 审批请求和工具调用会在本地记录，并进行脱敏。
- 远程部署只用于测试环境；MVP 前按手动流程处理。

self-modify 命令范围很窄：proposal 本地保存，apply 需要 approval id，会检查目标漂移，post-check 失败可以回滚。

## 部署

第一版部署 artifact 是本地 Docker 镜像：

```bash
docker build -f docker/Dockerfile -t ikaros:local .
docker compose -f docker/compose.yml run --rm ikaros --help
```

Runtime 状态和明文 provider credential 不进入镜像，而是放在 `/data/ikaros`，
通常由 Docker volume 持久化。当前契约和限制见
[Docker 部署](docs/zh-CN/deployment.md)。

## 开发

常用检查：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
cargo audit
cargo doc --workspace --all-features --no-deps
cargo run -p ikaros-cli -- doctor
```

自动化工具不要擅自 commit、tag、publish 或 push，除非维护者明确要求。

## 文档

- [完整文档索引](docs/README.md)
- [架构](docs/zh-CN/architecture.md)
- [Harness 模型](docs/zh-CN/harness-model.md)
- [Agent loop 设计](docs/zh-CN/agent-loop.md)
- [安全模型](docs/zh-CN/safety-model.md)
- [记忆模型](docs/zh-CN/memory-model.md)
- [Context 引擎](docs/zh-CN/context-engine.md)
- [RAG 模型](docs/zh-CN/rag-model.md)
- [模型 Provider](docs/zh-CN/model-providers.md)
- [语音 Provider](docs/zh-CN/voice-providers.md)
- [Body 模型](docs/zh-CN/body-model.md)
- [自动化模型](docs/zh-CN/automation-model.md)
- [消息网关](docs/zh-CN/message-gateway.md)
- [Service manager 模板](docs/zh-CN/service-manager.md)
- [配置](docs/zh-CN/configuration.md)
- [API 参考](docs/zh-CN/api-reference.md)
- [插件系统](docs/zh-CN/plugin-system.md)
- [Self-modify 设计](docs/zh-CN/self-modify.md)
- [部署](docs/zh-CN/deployment.md)
- [Roadmap](ROADMAP.md)
