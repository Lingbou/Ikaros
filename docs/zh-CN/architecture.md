# 架构

Ikaros 是 persona-first 的本地 Agent Runtime。核心边界是：runtime 编排 turn，harness 执行工具，provider adapter 只处理模型 wire format，本地状态默认留在 `IKAROS_HOME`。

本文描述 runtime 契约，不是源码文件清单。只有所有权、调用上下文、持久化状态或用户可见行为变化时，才需要更新本页。

## 术语

- Runtime：为一次命令或 worker tick 解析 config、agent identity、store、provider、context 和 harness session 的代码。
- Harness：所有工具的策略、审批、审计和执行边界。
- Provider：连接模型、embedding、TTS 或 ASR API 的 adapter。
- Transport：某类 provider 的 wire-format 描述。
- Model stream event：标准化后的模型增量，例如 text、reasoning、tool-call start/update/end、usage、error 或 done。
- Agent event：runtime 为 session、turn、user、model、tool、approval、context、error 和 turn-end 节点发出的 typed event。
- Session store：append-only 的 session、turn、event、approval 和 replay 持久化边界。当前实现是本地 SQLite。
- Agent profile：persona 和策略 overlay。
- Agent instance：运行身份，包含 `agent_id`、workspace、state dir、session policy、auth scope 和 route bindings。
- Context source：history、memory、RAG、relationship 或 persona 这类可组装进模型 turn 的上下文来源。

## Crate

- `ikaros-core`：共享配置、路径、任务类型、脱敏、错误、agent profile 和 `AgentInstance` 身份模型。
- `ikaros-session`：`SessionId`、`TurnId`、typed `AgentEvent`、append-only session entry、`SessionStore`、`SessionWriter`、SQLite `state.db` 和 replay 查询。
- `ikaros-runtime`：诊断、聊天、任务、计划任务、gateway drain、body frame、agent handoff、`AgentRuntime` 和 `ContextEngine`。
- `ikaros-harness`：策略判断、审批请求、审计日志、`ExecutionSession`、`ExecutionEnv`、技能执行、插件、guardrail 和任务 runner。
- `ikaros-memory`：JSONL/SQLite 记忆 store、`MemoryProvider` lifecycle 和 provider registry。
- `ikaros-rag`：本地文件摄取、chunk 存储、检索和 embedding provider。
- `ikaros-models`：`ModelProvider`、`ModelTransport`、mock、OpenAI-compatible、Anthropic、Ollama、streaming、tool-call normalization、用量日志和请求治理。
- `ikaros-gateway`：本地 inbox/outbox store，以及内置 `GatewayFrame` 协议类型。
- `ikaros-voice`：mock 和 OpenAI-compatible TTS/ASR provider。
- `ikaros-skills`：通过 harness 暴露的内置技能。
- `ikaros-cli`：命令行入口和终端输出。
- `ikaros-body`：body/status/frame 合约和简单渲染器。
- `ikaros-automation`、`ikaros-service`、`ikaros-coding`、`ikaros-soul`：对应领域的支持 crate。

## Runtime Flow

大多数入口遵循同一条路径：

1. CLI 或 worker 解析 `IKAROS_HOME`、workspace、config 和 agent id/profile。
2. Runtime 解析 `AgentInstance`，得到 `agent_id`、profile overlay、workspace、state dir、session policy、auth scope 和 route bindings。
3. Runtime 创建 store、provider adapter、skill registry、context engine 和 harness session。
4. 模型 turn 通过 `AgentRuntime` 执行；默认实现是 `HarnessAgentRuntime`。Runtime 会发出 typed `AgentEvent`。调用方可以挂上 `AgentEventSink`，把事件持久化到 `ikaros-session`；现有 CLI 和 worker caller 仍可继续消费最终 report。
5. 工具 dispatch 必须经过 `ExecutionSession` 和 `ExecutionEnv`，不能绕过 harness 直接碰 host API。
6. Harness 评估策略，记录审计事件，然后执行、请求审批或拒绝。
7. Runtime 从同一条 turn 路径归约出 CLI、body、schedule、gateway、chat 或 agent caller 可用的报告。

聊天、任务执行、计划任务、gateway drain 和 agent handoff 都复用这条路径。

## Agent Identity

Profile 和 instance 必须分开理解。

Profile 描述 agent 应该怎样工作：mode、persona overlay、context source 和普通策略默认值。Instance 描述谁在运行，以及状态属于哪里。一个配置好的 instance 可以选择某个 profile，同时拥有自己的 workspace、state dir、session policy、auth scope 和 route bindings。

解析顺序：

1. 如果请求名称匹配 `agent.instances.<id>`，runtime 从该配置创建 `AgentInstance`。
2. 如果没有匹配的 instance，则把请求名称当作 agent profile 解析。
3. 如果调用方没有传名称，则使用 `agent.default`。
4. 如果默认 profile 缺失，则回退到内置 `build` profile。

调用方应把解析后的 `AgentInstance` 传给 harness session。策略 overlay 和审批重放绑定 instance identity；persona 文本本身不能授予权限。

## Local State

默认状态目录是 `~/.ikaros`，也可以用 `IKAROS_HOME` 覆盖。

JSONL 仍是默认本地格式，因为容易检查和恢复。SQLite 可用于更大的本地 store，例如 memory、chat history 和 RAG index。MVP 不依赖远程服务。

状态归属：

- `state.db`：session metadata、append-only session entry、已持久化的 agent-loop event、approval record 和 replay 数据。agent-loop event 写入可以使用按 turn 的 `SessionWriter` transaction；chat、gateway、schedule、memory 和 audit 更大范围迁入这个 store 的工作仍在进行。
- `memory/`：本地记忆记录和 memory provider registry 元数据。
- `chat/`：聊天历史和 session summaries。
- `rag/`：本地 RAG 文件、chunk 和 embedding index。
- `audit/`：策略决策、审批记录、用量日志和迁移备份。
- `automation/`：计划任务元数据和投递报告。
- `gateway/`：本地消息路由的 inbox/outbox 记录。
- `skills/`：本地安装的插件和 marketplace 元数据。
- `agents/`：instance 使用默认状态根目录时的 per-agent state dir。

## Boundaries

- Persona 影响 prompt 和上下文，不影响策略。
- Agent profile 是 persona/policy overlay；`AgentInstance` 才是运行身份。
- `ModelProvider` 生成/stream 文本；`ModelTransport` 描述 provider wire format；`ModelStreamEvent` 标准化 provider delta；`AgentRuntime` 拥有 turn loop 并发出 `AgentEvent`。
- `AgentEvent`、session id、turn id、append-only session entry 和 replay 查询属于 `ikaros-session`，不属于 runtime loop。
- `SessionWriter` 负责按 turn 的 session transaction。当前内置用法只包住 agent-loop event 持久化；整个 chat turn 原子化仍是后续工作。
- `session_id` 标识持久化 timeline；`task_id` 是 task/report 元数据，不能再作为隐式 session fallback。
- `ContextEngine` 负责 ingest、assemble、compact、after_turn；memory、history、RAG 和 relationship 是 context source。
- `MemoryProvider` 暴露 turn_start、prefetch、sync_turn、pre_compress、session_switch 和 delegation_observation lifecycle hook。
- 工具执行属于 harness 和 `ExecutionEnv`，不属于模型 provider 或 UI。
- Gateway 协议类型放在 `ikaros-gateway` 内部；没有单独的 protocol crate。
- Self-modify 是单独审批路径，不是普通写权限。

## 不变量

- 模型响应不能直接作为 host 操作执行。Tool call 必须先标准化，再通过 `ExecutionSession` dispatch。
- Runtime event 是 turn 的 append-only 观察记录。Report 可以总结它们，但工具应优先依赖 typed event 字段，而不是解析面向人的文本。
- 持久化 session timeline 是 append-only 的。Branch、compact、retry 和 replay 工作应追加 entry 或 event，而不是改写旧 turn 事实。
- Provider adapter 不拥有 agent loop、审批流或 workspace mutation policy。
- Context assembly 可以用脱敏 audit input 调用 safe-read skill，但 audit log 不应保存完整用户 prompt。
- 审批重放必须绑定 workspace、精确 approved input 和 agent identity。
- Gateway ingestion 只负责入队，不直接调用模型、任务、插件或工具。
- Self-modify proposal 使用单独的 proposal/apply/rollback 路径，不代表普通写权限。

## 失败报告

多数 runtime 路径会给内部调用方返回结构化 report，并给 CLI 渲染面向人的摘要。Report 应足以解释工作为什么停止，同时不能保存 prompt 文本或 secret。常见停止条件包括策略拒绝、等待审批、迭代预算耗尽、guardrail halt、provider error、command timeout 和本地 store 错误。
