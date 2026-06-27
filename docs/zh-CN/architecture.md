# Architecture

Ikaros 是一个 persona-first、本地优先的 agent workspace。核心边界是：`ikaros-agent` 编排应用用例，`ikaros-execution` 治理工具执行，`ikaros-providers` 只处理外部 provider wire format，本地状态默认保存在 `IKAROS_HOME` 下。

## Crates

- `ikaros-core`：配置、路径、错误、脱敏、agent/profile、persona、emotion 和纯 agent config resolution。
- `ikaros-protocol`：API、TUI、gateway、replay、session projection 等稳定 wire/session shape。
- `ikaros-state`：session、memory、RAG、automation、gateway queue/store 等本地持久化。
- `ikaros-execution`：policy、approval、audit、sandbox、process/filesystem/network execution、tool contract、coding primitive 和 task/plugin dispatch。
- `ikaros-providers`：model、embedding、voice、vision/image、web、registry 和治理 adapter。
- `ikaros-host`：config/path/agent instance/provider/store/execution env/registry 的 composition root，也拥有 init/doctor、persona file management、approval resolution 和 host adapter。
- `ikaros-agent`：chat、context assembly、coding workflow、task loop、schedule、gateway drain、body status、agent loop、agent handoff 和 session runner。
- `ikaros-skills`：内置 skill/tool packs。
- `ikaros-surfaces`：local API、MCP、gateway adapter/webhook、body dashboard 和 service manager。
- `ikaros-terminal`：TUI screen/input/render/slash/status/timeline。
- `ikaros-cli`：薄 `clap`、dispatch 和 terminal output adapter。

当前 Cargo workspace 只包含上面的 11 个架构 crate。旧 implementation crate 已删除或吸收，包括 `ikaros-api`、`ikaros-automation`、`ikaros-body`、`ikaros-coding`、`ikaros-context`、`ikaros-gateway`、`ikaros-harness`、`ikaros-mcp`、`ikaros-memory`、`ikaros-models`、`ikaros-rag`、`ikaros-runtime`、`ikaros-sandbox`、`ikaros-service`、`ikaros-session`、`ikaros-soul`、`ikaros-toolkit`、`ikaros-tui` 和 `ikaros-voice`；它们不再是 workspace member。

## 依赖方向

当前直接 workspace 依赖如下；架构守卫会防止 legacy crate 回流和高层 facade 倒挂：

```text
cli -> agent/core/execution/host/protocol/providers/skills/state/surfaces/terminal
host -> core/execution/providers/skills/state
surfaces -> core/execution/host/protocol/providers/skills/state
terminal -> core/protocol
agent -> core/execution/protocol/providers/state
skills -> core/execution/protocol/providers/state
providers -> core/protocol
execution -> core
state -> core/protocol
protocol -> core
core -> none
```

## 规则

- `ikaros-cli` 只做参数解析、dispatch 和终端输出适配。
- `ikaros-agent` 不加载 config、不解析 workspace、不组装 host resource。
- `ikaros-terminal` 不拥有 execution、provider setup、config loading 或 store。
- `ikaros-surfaces` 不暴露本地 state store。
- 共享稳定 shape 下沉到 `ikaros-protocol`；纯基础类型下沉到 `ikaros-core`。
