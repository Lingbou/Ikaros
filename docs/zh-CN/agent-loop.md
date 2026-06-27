# Agent Loop

Agent loop 是 `ikaros-runtime` 中的模型引导执行路径。它允许模型请求 harness skill，接收工具结果，然后继续直到返回最终答案或触发停止条件。

Loop 拥有 turn 编排。它不拥有 provider 认证、provider wire format、策略决策或 host 执行。这些职责分别属于 `ModelProvider`/`ModelTransport` 和 harness。

## 范围

这个 loop 设计得比较小：

- 有界迭代次数
- 支持 provider-native tool call
- 对非 native 输出使用严格 JSON fallback envelope
- 通过 harness dispatch skill
- typed `AgentEvent` 和 `ModelStreamEvent` 记录
- 审计只记录元数据，不记录 prompt
- 观察 guardrail

模型永远不能直接执行工具。每个 tool call 都会被标准化并发送到 `ExecutionSession`。

## 接口

`AgentRuntime::run_turn()` 接收：

- `AgentLoopInput`：可选 session id、可选 turn id、可选 task id、system prompt 和 user input。
- `ModelProvider`：当前配置的 provider adapter。
- `ExecutionSession`：policy、approval、audit 和 environment 上下文。
- `SkillRegistry`：可执行的 harness skill。
- `AgentLoopOptions`：iteration、sampling、streaming 和 guardrail 设置。

默认实现是 `HarnessAgentRuntime`。如果调用方需要不同 loop 实现，应替换 runtime 层，而不是污染 provider adapter。

`RecordingAgentRuntime` 可以包装任意 `AgentRuntime`，并记录它转发给调用方 sink
的同一套 typed event stream。它是 replay/test adapter，用于需要完整内存事件轨迹
的调用方，避免把 `AgentLoopReport` 当成事实来源。

Agent-loop system prompt 也通过共享的 `PromptBuilder` pipeline 组装。Base system
text、严格 tool-call 协议、deferred tool disclosure guidance 和 available tool
manifest 会先成为独立 typed prompt section，然后再渲染。`TurnStart` event 会持久化
prompt metadata（`kind`、`source`、`title`、priority、token estimate 和 redaction
state），但不会持久化原始 prompt section 内容。Replay 和 UI 可以解释为什么本轮带了
tool guidance，而不需要反向解析 prompt 文本或存储 secret。

`AgentHarness` 是 `AgentRuntime` 上层的 stateful wrapper，面向需要稳定
session id、每轮 turn id、phase 跟踪和 continuation queue 的调用方。它负责
harness phase，并能运行 steer、follow-up、next-turn、resume、compact 和 retry
continuation；其中模型 turn 会交给 `AgentRuntime::run_turn_with_events()`。返回的 `AgentHarnessTurn` 以 typed
events 为主。Harness 会收集同一条已经转发给调用方 sink 的 emitted event stream，
并用这条 stream 回填 `AgentLoopReport.events` 作为兼容摘要。内置 chat 和 task
agent-loop 入口已经使用这个 wrapper。Agent-loop handoff 也使用这条路径，并在调用方
没有提供 source 时补上 subagent session source。Gateway task drain 和 schedule task
execution 也使用 session-aware task agent-loop path，并携带 gateway/schedule 派生的
session id、turn id 和 source metadata。直接的 `run_agent_loop*` helper 保留为测试和
特殊 runtime 的底层 API。

Harness phase 不只是展示用枚举。`AgentHarnessPhase` 现在已经有公开的 branch
summary、compaction marker 和 retry marker 操作，分别通过
`append_branch_summary()`、`append_compaction()` 和 `append_retry_marker()` 写入。
每个 helper 都作为一个有界 harness phase 执行，并通过 `SessionStore` 落到
append-only session tree。Branch、compaction、retry 和 active-leaf 操作是追加或选择
entry，不会改写已经发生的 turn。

如果 harness 配置了 `SessionStore`，continuation queue 就是 durable 的。
`ikaros-session` 会把 queued、running、completed、failed 和 cancelled continuation
写入 `state.db`。Claim continuation 时会写入 lease、递增 attempt count、记录 status reason，并在选择下一项前回收
过期的 running lease。Failed 或 cancelled continuation 可以带明确原因重新入队。Harness 可以
claim 并完成 message continuation（`steer`、`follow_up`、`next_turn`、`resume`）和
maintenance continuation（`compact`、`retry`），以及第一版可恢复 tool-result
continuation。Message continuation 会运行真实 turn；maintenance continuation 会追加 session
entry，并发出 `ContinuationStarted` / `ContinuationCompleted` / `ContinuationFailed`
事件，不会伪造模型响应。Tool-result continuation 会通过 harness session 重新执行可恢复
tool，脱敏 terminal payload，并把 queued continuation 标记为 completed 或 failed，不会伪造模型响应。显式 harness
取消会写入 `ContinuationCancelled` event。正在运行的 durable message continuation 会轮询
store 中的外部取消状态，取消当前 turn token，并在 worker 停止时写入 acknowledged 事件。
`ikaros debug continuations <session-id>` 可以查询 queue status、status reason、lease owner、
lease expiry、attempt、terminal summary、worker lease timeout evidence、error 和脱敏 payload。没有
continuation store 时，harness 仍保留内存队列，用于测试和特殊 one-shot caller。
这仍然只是 continuation queue，不是完整 scheduler。轮询间隔调优、scheduler 级 worker coordination、
更细的 tool-result 调度策略和面向 automation 的 timeout report 仍属于后续 runtime hardening。

`AgentLoopOptions::with_hooks()` 可以安装 observer-only `AgentLoopHooks`，覆盖
provider request/response 和 tool call 边界。Hook payload 只携带已脱敏元数据和
event anchor，不携带原始 prompt 或 tool secret。Hook 失败会记录为 runtime error
event，但不会修改或停止 turn。持久事实仍应从 typed `AgentEvent` stream 和
持久化 session timeline 读取；hook 是 telemetry、policy observation、UI 和 replay
diagnostics 的扩展边界。

需要持久化 timeline 的调用方应使用 `run_turn_with_events()` 并传入
`AgentEventSink`。`ikaros-session` 提供逐事件写入的 `PersistingAgentEventSink`、
按 turn 事务写入本地 SQLite `SessionStore` 的 `PersistingAgentTurnSink`、用于
replay/test 和 live UI 的 `CollectingAgentEventSink`，以及把同一条 typed event stream
同时送到持久化和 observer 的 `FanoutAgentEventSink`。Runtime wrapper 应复用这些
sink，而不是各自实现本地 callback fan-out。

`session_id` 是 event timeline 的持久化身份；`turn_id` 标识该 timeline 内的一轮持久化 turn。调用方需要让 chat session entry、
投影出的 history record 和 agent event 共用同一个 turn identity 时，可以显式传入 turn id。`task_id` 只作为 task/report 元数据。
调用方不传 session id 时，loop 会为该 turn 创建新的 `SessionId`，不会再落到全局 `"local"` session。

`AgentHarnessConfig` 也可以携带调用方提供的 `turn_id`。Chat 用它保证 append-only
session entry、投影出的 history record 和 agent event 落在同一轮 turn 上。这个值是
one-turn override：该 turn 执行后，后续 continuation 会拿到新的 turn id，除非调用方
再次显式提供。Task agent-loop 会让 harness 在 task session 内创建新的 turn id。
调用方可以 clone harness 的 cancellation token，或直接调用 `AgentHarness::cancel()`，
以取消下一次 provider request、尚未启动的已规划 tool call，或仍在 await 的运行中
tool future。

默认选项：

- `max_iterations = 4`
- `max_tokens = 512`
- `temperature = 0.2`
- `stream = false`
- 默认 guardrail 设置
- 新的 cancellation token

## Turn Sequence

1. 发起 provider request 前检查 cancellation token。
2. 用 system prompt、user input、之前的 assistant output、tool definition 和 tool result 构造模型请求。
3. 调用 `before_provider_request` hook，然后请求 provider 生成普通或 streaming response。
4. 调用 `after_provider_response` hook，并把 provider response 标准化为 text、stream、tool-call、usage、error 和 done 记录。
5. 优先消费 provider-native tool call。
6. 如果没有 native tool call，则从文本解析 fallback JSON 协议。
7. 如果存在 final answer，以 `FinalAnswer` 停止。
8. dispatch 已规划 tool call 前再次检查 cancellation。
9. 发出 `ToolCallStarted`，调用 `before_tool_call` hook，然后把标准化 tool call 通过 `ExecutionSession` dispatch。
10. 为每个 tool result 发出 tool lifecycle event，然后带着已脱敏的结果状态调用
   `after_tool_call` hook。普通 dispatch 会发出 `ToolCallOutputDelta`，随后发出
   `ToolCallCompleted` 或 `ToolCallFailed`；被取消的调用会发出
   `ToolCallCancelled`。如果模型已经返回 tool plan，但 dispatch 前收到取消请求，
   runtime 会为每个已规划调用发出 `ToolCallCancelled`，并且不会调用对应 skill。
   如果 tool future 已经启动但还没完成，runtime 会 drop 该 future，发出
   `ToolCallCancelled`，并以 `Cancelled` 结束本轮。
11. 把 tool result 按模型原始 tool call 顺序追加到下一次 model turn，即使 parallel
    batch 的实际完成顺序不同。
12. 继续前检查 guardrail 和 iteration budget。

Provider 返回 native tool call id 时会保留这些 id，以便下一轮用 provider 偏好的格式传回 tool result history。

Tool 调度由 harness metadata 决定，不属于 provider adapter。每个
`SkillDescriptor` 会暴露 `execution_mode` 和可选 `timeout_ms`。Runtime 会把连续
的 `parallel` tool call 组成一批并发执行，并在追加下一轮 tool result 时保持模型
原始调用顺序；`sequential` 调用会单独执行。safe-read 和 shell-read 工具默认
parallel；write、network、remote、destructive、secret 和 self-modify 风险工具默认
sequential，除非 descriptor 显式收窄或改变策略。

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

Progressive-disclosure bridge tool 还有额外约束。Deferred tool 不能仅因为模型猜到
name 就被调用。模型必须先调用 `tool_search`，并在返回的 descriptor 中看到目标
tool；或先调用 `tool_describe` 披露该目标。之后，模型才可以在同一 execution
session 中通过 `tool_call` 调用这个已披露的 deferred tool。目标工具仍然经过 harness
policy、approval、audit 和 `ExecutionEnv`。

最终答案：

```json
{"final_answer":"..."}
```

Fallback parser 只接受上面的 canonical 顶层 JSON object。它不接受 fenced JSON、embedded JSON、顶层数组，或 `tools`、
`calls`、`tool_call`、`function_call`、`args`、`arguments`、`answer`、`response` 这类别名。`final_answer`、每个
tool `name`，以及出现的 tool `id` 去掉空白后都必须非空。每次迭代都会在报告中记录解析策略。

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

Tool lifecycle event payload 包含标准化 tool name、provider 提供的 tool call id
（如果有）、脱敏后的 input snapshot、output summary/delta、status、execution mode、
timeout，以及 approval/audit evidence 可引用的稳定 tool-event anchor。成功进入
harness dispatch 的工具结果还会发出 `AuditAnchor` event，把 tool-event id、harness
call id、audit event id、audit kind 和 audit path 绑定起来。进入 report 或持久化
session event 前必须先脱敏。Descriptor timeout 会把该 tool call 记录成 failed tool
lifecycle result，并带上结构化 timeout metadata，包括 timeout 时长和 start/end timestamp；
它不能绕过 `ExecutionSession` 或 `ExecutionEnv`。已规划调用启动前收到 cancellation 时，
会产生 `ToolCallCancelled` payload，并以 `Cancelled` 停止 turn；provider request 或
tool future 运行中被取消时，也会停止 turn 并 drop 掉 pending future。进程型本地工具依赖
`ExecutionEnv` process runner 的 `kill_on_drop` 清理子进程。

`AgentLoopReport.events` 是当前调用方的兼容摘要。挂载持久化 sink 后，真正的事实来源是 `ikaros-session` 里的 event stream。Replay、
gateway、schedule 和 UI 路径应读取 session store，而不是从面向人的输出里反推 timeline。

内置 chat 路径使用 `PersistingAgentTurnSink`。agent-loop chat 和通过 `--no-agent-loop` 选择的单次 provider chat
都会写入 user/assistant `SessionEntry`。单次 provider chat 还会发出最小 typed event timeline：session start、turn
start、user message、标准化 model stream event、context diff 和 turn end。Context diff payload 会记录本轮
provider-aware token budget、section、显式 reference、compressed sections，以及新增、删除、压缩 context 的 token 估算。
发生 context compaction 时，chat 还会在 assistant entry 前写入 `ContextCompacted` event 和 compaction session
entry。`MemoryLifecycle`、`AuditAnchor` 这类 post-turn evidence 可能出现在 `TurnEnd` 之后；消费者应依赖 event kind，
不要假设最后一个 event 一定是 turn end。

同一个 turn 的 session entry 和 chat agent event 会一起 commit 或 rollback。chat history 现在从同一个 `state.db`
replay 投影生成；core memory record、memory candidate 和 audit 目前仍是独立 store。Memory sync 会把安全的脱敏 turn
context 写入带 `MemoryRef::SessionTurn` 的 session working memory；普通 turn summary 不会提升成长期 `Task` memory。
自动 relationship 观察会进入 candidate inbox，而不是直接写入 core memory。session timeline 只保留高层 lifecycle evidence。
本地 memory journal 会记录对应的 `sync_turn` append/skipped-write 决策，以及本轮受影响 scope 的 promote、demote、forget 或
quota policy action；debug 调用方不需要直接读取 memory store 也能检查 memory lifecycle 行为。持久化 agent-loop turn 创建
approval request 时，会把脱敏后的 approval request 双写进 session approval table；后续 approve、deny 或 execute
decision 会更新同一条 session approval record，并发出 `ApprovalResolved`。

provider failure 和本地后处理 failure 会在返回失败前写入 session。失败的 chat turn 会保留 user `SessionEntry`，发出带脱敏
message 和 phase 的 `Error` event，并以 failed `TurnEnd` event 收尾，所以 replay/debug 调用方不会丢掉这段 timeline。

普通 chat turn 不再写单独的 history mirror。User/assistant entry、model stream event、
context diff、memory lifecycle evidence 和 audit anchor 都通过 session store commit。
Replay、workbench history、搜索和 context assembly 都读取 `state.db` session replay，
把它作为唯一聊天 timeline。

## 不变量

- Prompt 可以描述工具，但只有 harness registry 定义什么能执行。
- Tool definition 包含 name、description、input schema 和 risk level。
- Tool call 被策略拒绝或等待审批时 loop 停止；loop 不会尝试换一个工具绕过策略。
- Guardrail 在每次 tool dispatch 后观察重复失败和无进展。
- Fallback JSON 协议是兼容路径；provider-native tool call 仍是首选路径。
