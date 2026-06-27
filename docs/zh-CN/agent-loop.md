# Agent Loop

Agent loop 是 `ikaros-agent` 中的模型引导执行路径。它让模型从当前 `SkillRegistry` 请求受治理的工具，接收工具结果，并持续执行到返回最终答案或触发停止条件。

## 边界

Agent loop 负责 turn 编排，但不负责：

- provider 认证或 wire format，这些属于 `ikaros-providers`。
- policy、approval、audit、sandbox、tool dispatch，这些属于 `ikaros-execution`。
- config、路径、agent instance、provider/store/registry 装配，这些属于 `ikaros-host`、CLI 或 surface 边界。

模型永远不能直接执行工具。每个 tool call 都必须被标准化，然后通过 `ExecutionSession` 和受治理的 `ExecutionEnv` 执行。

## 接口

`ikaros-agent` 暴露 `run_agent_loop`、`run_agent_loop_with_events`、`HarnessAgentRuntime`、`AgentHarness` 等底层和 stateful wrapper。调用方应传入已经装配好的 provider、execution session、skill registry、session writer 和 event sink。

Chat、task、gateway drain、schedule 和 agent handoff 都应走显式 context/session 的 agent API，不应重新引入 `ikaros-runtime` 或把 host 装配藏进 agent 层。

## 规则

- Agent loop 只产出 typed `AgentEvent`、stream event 和结构化 report。
- 持久化由 `ikaros-state::session` 负责。
- 审批与审计由 `ikaros-execution` 负责。
- 任何需要读取 config、解析 workspace、创建 provider/store/registry 的逻辑都应上移到 `ikaros-host` 或入口层。
