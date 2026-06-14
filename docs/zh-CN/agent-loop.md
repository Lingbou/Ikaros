# Agent Loop

Agent loop 是 `ikaros-runtime` 中的模型引导执行路径。它允许模型请求 harness skill，接收工具结果，然后继续直到返回最终答案或触发停止条件。

Loop 拥有 turn 编排。它不拥有 provider 认证、provider wire format、策略决策或 host 执行。这些职责分别属于 `ModelProvider`/`ModelTransport` 和 harness。

## 范围

这个 loop 设计得比较小：

- 有界迭代次数
- 支持 provider-native tool call
- 对非 native 输出使用严格 JSON fallback 解析
- 通过 harness dispatch skill
- typed `AgentEvent` 和 `ModelStreamEvent` 记录
- 审计只记录元数据，不记录 prompt
- 观察 guardrail

模型永远不能直接执行工具。每个 tool call 都会被标准化并发送到 `ExecutionSession`。

## 接口

`AgentRuntime::run_turn()` 接收：

- `AgentLoopInput`：可选 session id、可选 task id、system prompt 和 user input。
- `ModelProvider`：当前配置的 provider adapter。
- `ExecutionSession`：policy、approval、audit 和 environment 上下文。
- `SkillRegistry`：可执行的 harness skill。
- `AgentLoopOptions`：iteration、sampling、streaming 和 guardrail 设置。

默认实现是 `HarnessAgentRuntime`。如果调用方需要不同 loop 实现，应替换 runtime 层，而不是污染 provider adapter。

需要持久化 timeline 的调用方应使用 `run_turn_with_events()` 并传入 `AgentEventSink`。`ikaros-session` 提供 `PersistingAgentEventSink`，可以把发出的事件写入本地 SQLite `SessionStore`。

`session_id` 是 event timeline 的持久化身份；`task_id` 只作为 task/report 元数据。调用方不传 session id 时，loop 会为该 turn 创建新的 `SessionId`，不会再落到全局 `"local"` session。

默认选项：

- `max_iterations = 4`
- `max_tokens = 512`
- `temperature = 0.2`
- `stream = false`
- 默认 guardrail 设置

## Turn Sequence

每次迭代顺序一致：

1. 用 system prompt、user input、之前的 assistant output、tool definition 和 tool result 构造模型请求。
2. 请求 provider 生成普通或 streaming response。
3. 优先消费 provider-native tool call。
4. 如果没有 native tool call，则从文本解析 fallback JSON 协议。
5. 如果存在 final answer，以 `FinalAnswer` 停止。
6. 把标准化 tool call 通过 `ExecutionSession` dispatch。
7. 把 tool result 追加到下一次 model turn。
8. 继续前检查 guardrail 和 iteration budget。

Provider 返回 native tool call id 时会保留这些 id，以便下一轮用 provider 偏好的格式传回 tool result history。

## 停止原因

Loop 可以因为以下原因停止：

- 产生最终答案
- 达到迭代预算
- 策略拒绝请求的工具
- 请求的工具需要审批
- guardrail 停止执行
- 观察到 provider error
- cancellation、compaction、tool error 或 context limit 停止 turn

Task 和 agent 命令可以用 `--agent-loop` 启用 loop。非 streaming chat 默认使用 loop；`--no-agent-loop` 强制使用单次 provider 调用。

结构化报告使用这些 stop reason：

- `FinalAnswer`
- `IterationBudget`
- `PolicyDenied`
- `WaitingForApproval`
- `GuardrailHalt`
- `Cancelled`
- `ProviderError`
- `Compacted`
- `ToolError`
- `ContextLimit`

如果无法构造完整 report，transport 和本地 store failure 仍可能作为命令错误返回。Runtime 能先发出事件时，provider failure 也会以 typed error event 暴露。

## Tool Call

首选路径：

1. Provider 接收 native tool definition。
2. Provider 返回 native tool call。
3. Runtime 标准化这些调用。
4. Harness dispatch。

回退协议：

```json
{"tool_calls":[{"id":"optional_call_id","name":"tool_name","input":{}}]}
```

最终答案：

```json
{"final_answer":"..."}
```

Fallback parser 只接受上面的 canonical 顶层 JSON object。它不接受 fenced JSON、embedded JSON、顶层数组，或 `tools`、`calls`、`tool_call`、`function_call`、`args`、`arguments`、`answer`、`response` 这类别名。每次迭代都会在报告中记录解析策略。

Loop 会报告这些 parse strategy：

- `provider_native_tool_calls`
- `json_fallback`
- `plain_text`

`repaired` 当前始终为 false。宽松 JSON repair 已在 MVP 前移除，以保持 runtime 合约收窄。

## Report Contract

`AgentLoopReport` 包含：

- stop reason
- final content
- provider 和 model 名称
- token usage
- 是否使用 streaming
- streaming 启用时的 stream chunks
- turn 期间发出的 typed events
- iteration count
- tool-call diagnostics
- tool results

Tool result summary 和 output 由 harness 产生。展示给用户或写入审计前应完成脱敏。

`AgentLoopReport.events` 是当前调用方的兼容摘要。挂载持久化 sink 后，真正的事实来源是 `ikaros-session` 里的 event stream。后续 replay、gateway 和 UI 路径应读取 session store，而不是从面向人的输出里反推 timeline。

当前内置持久化接在 agent-loop turn 上，包括默认的非 streaming chat 路径。通过 `--no-agent-loop` 选择的单次 provider chat 仍使用 chat history 和 audit store，还没有完整 agent event timeline。

## 不变量

- Prompt 可以描述工具，但只有 harness registry 定义什么能执行。
- Tool definition 包含 name、description、input schema 和 risk level。
- Tool call 被策略拒绝或等待审批时 loop 停止；loop 不会尝试换一个工具绕过策略。
- Guardrail 在每次 tool dispatch 后观察重复失败和无进展。
- Fallback JSON 协议是兼容路径；provider-native tool call 仍是首选路径。
