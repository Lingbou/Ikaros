# Ikaros

[English](README.md) | [文档](docs/README.md)

Ikaros 是一个早期阶段的本地优先 agent runtime，使用 Rust 编写。它把 persona、记忆、RAG、模型提供方、工具执行、策略审批和审计日志分开，便于维护和扩展。

这个仓库目前是一个 pre-MVP 本地 runtime，适合本地开发和实验。它还不是稳定产品，也不承诺稳定 API。生成配置使用本地存储和协议级 provider 设置；远程模型、embedding、TTS 和 ASR 调用会在本地填好 `api_key`、`base_url` 和 `model` 之前直接报错。

## 能做什么

- 提供本地 CLI 工作流：聊天、记忆、RAG、计划任务、消息入口、审批、插件、coding turn 报告、代码审查辅助和受控编辑。
- 默认把记忆、聊天历史、RAG 索引、自动化元数据、消息网关、审批和审计日志保存在本地。
- 所有工具执行都经过 harness 层，包含策略判断、审批请求、审计事件、dry-run 和 guardrail。
- 实现 OpenAI-compatible、Anthropic-compatible 和 Ollama 模型 adapter，以及 OpenAI-compatible 的 embedding、TTS 和 ASR adapter。mock provider 只在显式选择时用于离线测试。
- 支持 `build`、`plan`、`general` 这类 agent profile，用来调整上下文和普通策略行为，但不能绕过硬性安全规则。

## 仓库结构

- `crates/ikaros-core`：共享配置、路径、任务状态、脱敏、错误和 agent profile 类型。
- `crates/ikaros-automation`：本地计划任务元数据和运行状态。
- `crates/ikaros-body`：可替换的 body/status/frame 合约和 dashboard 渲染。
- `crates/ikaros-cli`：`ikaros` 命令行程序。
- `crates/ikaros-coding`：仓库扫描、受控 patch、结构化 patch failure、turn diff tracking、代码审查、coding turn report、自修改记录和测试命令分析。
- `crates/ikaros-context`：context bundle、section、reference、provider-aware token budget、token estimator adapter、quota-based compaction 和 diff primitive。
- `crates/ikaros-gateway`：本地消息 inbox/outbox 元数据和投递路由。
- `crates/ikaros-harness`：策略引擎、审批队列、审计日志、技能执行 session、插件和任务 runner。
- `crates/ikaros-memory`：本地 JSONL 和 SQLite 记忆存储、lifecycle hook 和 policy journal primitive。
- `crates/ikaros-models`：mock、OpenAI-compatible、Anthropic 和 Ollama 模型 provider，以及 context profile、治理和用量日志。
- `crates/ikaros-rag`：本地 RAG 摄取、索引、检索和 embedding provider。
- `crates/ikaros-runtime`：聊天、任务、计划任务、消息 drain、body 状态、诊断和 agent handoff 编排。
- `crates/ikaros-session`：session id、turn id、typed agent event、按 turn 的 session 写入、SQLite `state.db`、append-only session entry 和 replay 查询。
- `crates/ikaros-service`：本地 worker 进程的 service manager 模板渲染。
- `crates/ikaros-skills`：文件系统、shell/git、记忆、RAG、语音、代码、persona 和插件等内置技能。
- `crates/ikaros-soul`：persona、情绪、语气和关系模型基础类型。
- `crates/ikaros-voice`：TTS 和 ASR provider 抽象，包含 mock 和 OpenAI-compatible 实现。
- `docs/`：按语言分组的设计文档和子系统文档。

## 快速开始

```bash
cargo run -p ikaros-cli -- init
cargo run -p ikaros-cli -- config validate
cargo run -p ikaros-cli -- doctor
cargo run -p ikaros-cli -- chat
cargo run -p ikaros-cli -- chat --message "hello"
```

常用本地命令：

```bash
cargo run -p ikaros-cli -- memory add "Keep RAG local-first" --kind project --scope ikaros
cargo run -p ikaros-cli -- memory search "RAG"
cargo run -p ikaros-cli -- memory add --kind relationship --scope default --observer alice --subject bob "Bob likes pancakes"
cargo run -p ikaros-cli -- memory projection render --scope ikaros
cargo run -p ikaros-cli -- memory candidate list
cargo run -p ikaros-cli -- memory candidate accept <candidate-id> --supersedes <memory-id> --reason "user corrected this"
cargo run -p ikaros-cli -- memory working prune
cargo run -p ikaros-cli -- rag ingest docs --scope project
cargo run -p ikaros-cli -- rag search "harness policy"
cargo run -p ikaros-cli -- task run "summarize this repository" --dry-run
cargo run -p ikaros-cli -- code workflow "review a candidate patch" --diff "<unified diff>"
cargo run -p ikaros-cli -- debug context-diff <session-id>
cargo run -p ikaros-cli -- debug memory-lifecycle <session-id>
cargo run -p ikaros-cli -- debug coding-turn <session-id>
cargo run -p ikaros-cli -- approval list
cargo run -p ikaros-cli -- skill list
```

使用 `IKAROS_HOME=/custom/path` 或 `--ikaros-home /custom/path` 可以隔离本地状态。默认状态目录是 `~/.ikaros`。

## 配置

`ikaros init` 会创建 `IKAROS_HOME/config.yaml`。默认配置使用 JSONL 本地存储和通用 OpenAI-compatible provider 条目，凭证字段为空。普通 chat 默认注入已接受的 memory projection、最近历史和当前 session working memory；RAG 是带 citation 的 reference retrieval，除非 profile 显式启用或用户传入 `--rag-top-k`，否则不会自动注入。

可以在 `~/.ikaros/config.yaml` 中切换到 SQLite：

```yaml
memory:
  backend: sqlite

chat_history:
  backend: sqlite

rag:
  backend: sqlite
  embedding_provider: hash
```

不要把真实 API key 写进这个仓库。远程 provider 应在 `~/.ikaros/config.yaml` 或其他 `IKAROS_HOME/config.yaml` 中配置。生成的配置会把模型 key、URL 和聊天模型名放在靠前位置：

```yaml
providers:
  model:
    api_key: "replace-with-your-model-key"
    base_url: "https://api.example.com/v1"
  embedding:
    api_key: "replace-with-your-embedding-key"
    base_url: "https://api.example.com/v1"

model:
  default:
    model: provider-model-id
    provider: openai-compatible
```

Provider 设置只保存在本地，不进仓库。编辑后运行 `ikaros config validate`。缺 key、URL、模型名、非法 backend、未知字段和 descriptor-only 的外部 memory provider 都会被报告，且不会打印 secret 值。

## 安全模型

Ikaros 把本地工具执行视为需要策略约束的操作：

- harness 范围内的安全读取默认允许。
- 工作区写入、shell 写操作、网络调用和疑似 secret 路径会经过策略判断，必要时返回审批请求。
- 破坏性命令、直接 secret 访问、发布动作和普通自修改默认拒绝。
- 审批请求和工具调用会在本地记录，并进行脱敏。
- 远程部署只用于测试环境；MVP 前按手动流程处理。

self-modify 命令范围很窄：proposal 本地保存，apply 需要 approval id，会检查目标漂移，post-check 失败可以回滚。

## 开发

常用检查：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
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
- [Roadmap](ROADMAP.md)
