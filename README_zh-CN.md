# Ikaros

[English](README.md) | [文档](docs/README.md)

Ikaros 是一个早期阶段、本地优先的 Rust agent 工作区。它把人格、记忆、RAG、模型/语音 provider、工具执行、策略审批和审计日志放在清晰边界后面，方便后续长期维护。

项目目前仍是 pre-MVP，适合本地开发和实验；还不是稳定产品或稳定 API。

`ikaros init` 会创建最小本地 `config.yaml`。远程模型调用需要你在本地配置 `api_key`、`base_url` 和 `model`；默认 RAG embedding 使用本地 `hash`，语音默认使用 `mock`。

## 能做什么

- 提供 CLI 和终端 workbench：聊天、session replay、context/memory/RAG 检查、计划任务、消息入口、审批、插件、coding turn 报告、代码审查辅助和受控编辑。
- 暴露本地 API、MCP、browser/CDP、web search/extract、vision、image generation 和多模态附件入口。
- 默认把记忆、聊天时间线、RAG 索引、自动化元数据、gateway 消息、审批和审计日志保存在本地。
- 所有工具执行都经过 `ikaros-execution` 的策略、审批、审计、sandbox 和 guardrail。
- 支持 OpenAI-compatible、Anthropic-compatible、Ollama、mock model provider，以及 embedding、TTS/ASR 等 provider 路径。

## 仓库结构

- `crates/ikaros-core`：配置 schema、路径、错误、脱敏、agent/profile、persona、emotion 和关系基础类型。
- `crates/ikaros-protocol`：API、TUI、gateway、replay、session projection 等稳定 wire/session 类型。
- `crates/ikaros-state`：session、memory、RAG、automation、gateway queue/store 等本地持久化。
- `crates/ikaros-execution`：policy、approval、audit、sandbox、process/filesystem/network egress、tool dispatch 和 coding primitive。
- `crates/ikaros-providers`：模型、embedding、voice、vision/image、web provider adapter 与治理。
- `crates/ikaros-host`：config/path/agent instance/provider/store/execution env/registry 的小型 composition root，以及 init/doctor 等 host 诊断。
- `crates/ikaros-agent`：chat、context assembly、coding workflow、task loop、schedule、gateway drain、agent loop 和 session runner 等应用用例。
- `crates/ikaros-skills`：内置 skill/tool packs。
- `crates/ikaros-surfaces`：API、MCP、gateway adapter/webhook、body dashboard 和 service manager。
- `crates/ikaros-terminal`：TUI/screen/input/render/slash/status/timeline。
- `crates/ikaros-cli`：轻量 `clap`、dispatch 和终端输出适配。

当前 Cargo workspace 只包含上面的 11 个架构 crate。旧的实现 crate 已经被吸收或删除，包括 `ikaros-api`、`ikaros-automation`、`ikaros-body`、`ikaros-coding`、`ikaros-context`、`ikaros-gateway`、`ikaros-harness`、`ikaros-mcp`、`ikaros-memory`、`ikaros-models`、`ikaros-rag`、`ikaros-runtime`、`ikaros-sandbox`、`ikaros-service`、`ikaros-session`、`ikaros-soul`、`ikaros-toolkit`、`ikaros-tui` 和 `ikaros-voice`。新增代码应该依赖上面的架构 crate，不要重新添加兼容 shim。

## 快速开始

```bash
cargo run -p ikaros-cli -- init
cargo run -p ikaros-cli -- setup --interactive
cargo run -p ikaros-cli -- config validate
cargo run -p ikaros-cli -- doctor
cargo run -p ikaros-cli --
cargo run -p ikaros-cli -- chat --message "hello"
```

默认入口就是终端聊天界面：安装后运行 `ikaros`，或在源码 checkout 中运行 `cargo run -p ikaros-cli --`。

## 开发验证

```bash
cargo fmt --all -- --check
cargo test -p ikaros-cli --test architecture_guard
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Live provider smoke 只能使用仓库外临时 `IKAROS_HOME`，不要把 API key 写进仓库、文档或测试输出。
