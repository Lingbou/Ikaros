# 架构

Ikaros 是 persona-first 的本地 Agent Runtime。核心边界是：runtime 编排 turn，harness 执行工具，provider adapter 只处理模型
wire format，本地状态默认留在 `IKAROS_HOME`。

本文描述 runtime 契约，不是源码文件清单。只有所有权、调用上下文、持久化状态或用户可见行为变化时，才需要更新本页。

## 术语

- Runtime：为一次命令或 worker tick 驱动 chat、task、schedule、gateway drain、
  body frame 和 agent-loop report 的代码。
- Host assembly：加载 config、解析 `AgentInstance`、构造 `RuntimeLocation`，
  并为 runtime caller 组装 `ExecutionSession`、`ExecutionEnv` 和 `SkillRegistry`
  的边界。
- Harness：所有工具的策略、审批、审计和执行边界。
- Provider：连接模型、embedding、TTS 或 ASR API 的 adapter。
- Transport：某类 provider 的 wire-format 描述。
- Model stream event：标准化后的模型增量，例如 text、reasoning、tool-call start/update/end、usage、error 或 done。
- Agent event：runtime 为 session、turn、user、model、tool、approval、memory lifecycle、audit anchor、context、
  error 和 turn-end 节点发出的 typed event。
- Session store：append-only 的 session、turn、event、approval、durable continuation queue 和 replay 持久化边界。
  当前实现是本地 SQLite，并用 FTS5/trigram index 支持 session entry 搜索。
- Context bundle：一个 turn 使用的 token-budgeted context section 集合，包含已解析 reference，以及说明新增、删除、压缩内容的 diff。
- Coding turn context：受控 coding workflow 使用的 workspace、git state、mode、permission profile、
  instructions、test command 和 session/turn identity。
- Agent profile：persona 和策略 overlay。
- Agent instance：运行身份，包含 `agent_id`、workspace、state dir、session policy、auth scope 和 route bindings。
- Context source：reference、history、memory、RAG、relationship 或 persona 这类可组装进模型 turn 的上下文来源。

## Crate

- `ikaros-core`：共享配置、路径、任务类型、脱敏、错误、agent profile 和 `AgentInstance` 身份模型。
- `ikaros-session`：`SessionId`、`TurnId`、typed `AgentEvent`、append-only session entry、`SessionStore`、
  `SessionWriter`、SQLite `state.db`、replay/search/branch/continuation 查询，以及 SQLite operational
  report、WAL checkpoint、backup、repair、prune 和 vacuum helper。
- `ikaros-context`：context bundle、prompt builder/section、reference、provider-aware token budget、
  quota-based compaction、context diff、context engine descriptor，以及第一版 provider-backed LLM summary
  request builder。
- `ikaros-runtime`：诊断、聊天、任务、计划任务、gateway drain、body frame、agent handoff、
  `AgentRuntime`、`AgentHarness` 和 context 编排。
- `ikaros-host`：host 侧组装层，负责 `RuntimeLocation`、`AgentInstance` 解析、
  `ExecutionSession`、`SkillRegistry`、skill environment、runtime `ExecutionEnv`
  和 provider/search 网络出口 allowlist。
- `ikaros-harness`：策略判断、审批请求、审计日志、`ExecutionSession`、`ExecutionEnv`、
  受控 network egress policy、技能执行、插件、guardrail 和任务 runner。
- `ikaros-memory`：JSONL/SQLite 记忆 store、`MemoryProvider` lifecycle、
  memory policy/journal primitive 和 provider registry。
- `ikaros-rag`：本地文件摄取、chunk 存储、检索和本地 embedding primitive。远程
  embedding HTTP 只在受 harness 治理的 RAG skill 中通过 `ExecutionEnv` 实现。
- `ikaros-mcp`：受 harness 托管的 MCP stdio server、JSON-RPC request/response helper、
  tool schema 转换和一次性 stdio probe 解析。
- `ikaros-models`：`ModelProvider`、`ModelTransport`、provider registry descriptor、
  结构化 provider profile、
  prompt-cache policy metadata、model context profile、mock、OpenAI-compatible、Anthropic、Ollama、
  streaming、多模态 content block、tool-call normalization、retry/error classification、
  health state、用量日志和请求治理。
- `ikaros-protocol`：CLI、TUI、gateway、本地 API、replay 和外部集成界面共享的版本化 wire 类型。
- `ikaros-gateway`：本地 inbox/outbox store，以及内置 `GatewayFrame` 协议类型。
- `ikaros-voice`：mock 和 OpenAI-compatible TTS/ASR provider。
- `ikaros-skills`：通过 harness 暴露的内置技能。
- `ikaros-cli`：命令行入口和终端输出。
- `ikaros-body`：body/status/frame 合约和简单渲染器。
- `ikaros-automation`、`ikaros-service`、`ikaros-coding`、`ikaros-soul`：对应领域的支持
  crate。`ikaros-coding` 负责 repo scan、guarded patch、结构化 patch failure、
  turn diff tracking、code review、coding turn report、self-modify record 和
  test-command analysis。

## Runtime Flow

大多数入口遵循同一条路径：

1. CLI 或 worker 解析 `IKAROS_HOME`、workspace 和请求的 agent id。
2. Host assembly 加载 config，并解析 `AgentInstance`，得到 `agent_id`、
   profile overlay、workspace、state dir、session policy、auth scope 和 route
   bindings。
3. Host assembly 创建 harness session、runtime execution environment、skill
   environment、skill registry 和 runtime location。Runtime 代码再创建本轮需要的
   provider、context 和 session writer。
4. 模型 turn 通过 `AgentRuntime` 执行；默认实现是 `HarnessAgentRuntime`。Chat 和 task agent-loop 入口会包一层
   `AgentHarness`，由它管理 phase、调用方传入的 turn id，并在配置了 `SessionStore` 时处理 durable continuation queue。
   Gateway task drain、计划任务 task execution 和 agent-loop handoff 现在也会携带显式 session id、turn id 和 source
   metadata 进入 session-aware task agent-loop path。Runtime 会发出 typed `AgentEvent`。调用方可以挂上
   `AgentEventSink`，把事件持久化到 `ikaros-session`；现有 CLI 和 worker caller 仍可继续消费最终 report。
5. 工具 dispatch 必须经过 `ExecutionSession` 和 `ExecutionEnv`，runtime 代码不能绕过
   harness 直接碰 host API，也不能重复实现 host assembly。
6. Harness 评估策略，记录审计事件，然后执行、请求审批或拒绝。
7. Runtime 从同一条 turn 路径归约出 CLI、body、schedule、gateway、chat 或 agent caller 可用的报告。

Chat 和 task 的 agent-loop 执行现在已经走 stateful harness 路径。Gateway task drain、计划任务 task execution 和
agent-loop handoff 也会带着显式 session source metadata 进入这条路径，因此它们的 agent-loop event 和 continuation 状态可以与
gateway/schedule evidence 落在同一个 `state.db` timeline 中。
Durable continuation queue 是恢复和 replay 边界，还不是完整 scheduler。它现在会记录 lease、attempt count、status reason、
requeue status、terminal status、取消 request/acknowledgement evidence、worker lease timeout summary、
第一版可恢复 tool-result continuation 和面向用户的 debug query 数据。正在运行的 durable message continuation 会轮询外部取消状态，
但可配置 worker coordination、更细的 tool-result 调度策略和 scheduler 级
terminal accounting 仍属于 runtime hardening。


## Agent Identity

Profile 和 instance 必须分开理解。

Profile 描述 agent 应该怎样工作：mode、persona overlay、context source 和普通策略默认值。Instance 描述谁在运行，以及状态属于哪里。一个配置好的
instance 可以选择某个 profile，同时拥有自己的 workspace、state dir、toolset allowlist、model/provider override、
session policy、auth scope 和 route bindings。

解析顺序：

1. 如果请求名称匹配 `agent.instances.<id>`，runtime 从该配置创建 `AgentInstance`。
2. 如果没有匹配的 instance，则把请求名称当作 agent profile 解析。
3. 如果调用方没有传名称，则使用 `agent.default`。
4. 如果默认 profile 缺失，则回退到内置 `build` profile。

调用方应把解析后的 `AgentInstance` 传给 harness session。策略 overlay 和审批重放绑定
instance identity；persona 文本本身不能授予权限。

## Local State

默认状态目录是 `~/.ikaros`，也可以用 `IKAROS_HOME` 覆盖。

JSONL 仍可用于简单本地 store，因为容易检查和恢复。SQLite 是 session timeline
的权威 store，也可用于 memory 和 RAG index 这类更大的本地 store。MVP 不依赖远程服务。

状态归属：

- `state.db`：session metadata、append-only session entry、已持久化的 chat/agent-loop event、gateway/schedule
  evidence、approval record、durable continuation queue、FTS5/trigram search index、branch/compact/retry
  marker、coding turn event 和 replay 数据。内置 chat turn 会通过按 turn 的 `SessionWriter` transaction 写入
  user/assistant entry；gateway 和 schedule worker 也会把 request/result/delivery evidence 映射进同一个 store。
  普通聊天历史、搜索、summary、replay 和 workbench timeline 都从这里投影出来，不再拥有独立 `chat/` store。Memory lifecycle 和
  audit log 仍有自己的 store，session 中只保留明确且脱敏的 evidence。运维 helper 可以报告 journal mode、integrity、WAL
  checkpoint 状态、search-index 可用性和 write policy；debug 命令可以 checkpoint WAL、backup/repair artifact、
  prune 已结束 session 并 vacuum 数据库。
- `memory/`：本地记忆记录、memory policy journal 数据和 memory provider registry 元数据。
- 聊天历史视图：从 `state.db` replay 投影生成；普通 chat turn 不写单独的 `chat/` mirror。
- `rag/`：本地 RAG 文件、chunk 和 embedding index。
- `audit/`：策略决策、审批记录、用量日志和迁移备份。
- `automation/`：计划任务元数据和投递报告。
- `gateway/`：本地消息路由的 inbox/outbox 记录、worker lease/retry/dead-letter metadata 和 sibling lock file。
- `browser/`：本地 browser supervisor profile、启动状态和浏览器 runtime metadata。
- `logs/trace.jsonl`：CLI、API 和本地诊断使用的结构化 tracing event。
- `skills/`：本地安装的插件和 marketplace 元数据。
- `agents/`：instance 使用默认状态根目录时的 per-agent state dir。

## Boundaries

- Persona 影响 prompt 和上下文，不影响策略。
- Agent profile 是 persona/policy overlay；`AgentInstance` 才是运行身份，并且可以为 chat、TUI、coding、task
  agent-loop、doctor 和 provider inspect 路径覆盖 toolset 和 model/provider 设置。
- `ModelProvider` 生成/stream 文本；`ModelTransport` 描述 provider wire format；`ProviderRegistry` 解析用于
  inspect 和 runtime 规划的本地 descriptor metadata；`ModelStreamEvent` 标准化 provider delta；`AgentRuntime`
  拥有 turn loop 并发出 `AgentEvent`。
- OpenAI-compatible provider quirks 会通过静态 `ProviderProfile` spec catalog 解析。Registry 和 request
  builder 共用这份 decision，读取 output default、context metadata、temperature/reasoning/message/tool-schema
  policy、额外 request body 行为和 prompt-cache policy。Anthropic 会把 prompt cache read/write usage 映射进
  `TokenUsage`，让 status 和 audit 视图能把 cache accounting 与普通输入/输出 token 分开解释。
- `AgentEvent`、session id、turn id、append-only session entry 和 replay 查询属于
  `ikaros-session`，不属于 runtime loop。
- `ikaros-protocol` 拥有 API、TUI、gateway、replay 和外部集成界面使用的持久 wire shape。
  Runtime、session 和 model crate 可以投影到这些 shape，但产品界面不应各自发明不兼容的
  event 或 state schema。
- `SessionWriter` 负责按 turn 的 session transaction。内置 chat 用它包住 session entry 和 typed event。Gateway 和
  schedule worker 会把高层 evidence entry/event 写入 `state.db`。Memory 和 audit 仍有独立 store，但 session
  evidence 必须明确且脱敏。普通 chat 不再写单独的 history mirror；session replay 就是聊天 timeline。
- `AgentEventSink` 是 event-bus 边界。`ikaros-session` 提供 noop、collecting、fan-out、逐事件持久化和按 turn
  transaction 持久化 sink，因此 runtime 可以把同一条 typed event stream 同时发给持久化、replay/test collector、UI
  observer、metrics 或 plugin observer，而不需要每个调用方重新实现 callback fan-out。
- Host assembly 属于 `ikaros-host`。新的本地入口应复用 `RuntimeHarness` 或
  `HostServices`，不要在 CLI/runtime 模块里重新实现 agent 解析、workspace scope、
  skill registry、provider egress allowlist 或 execution environment 组合。
- Gateway worker claim 消息时会记录脱敏后的 lease owner、lease expiry 和 attempt count。处理失败会清除 lease，并根据 retry
  budget 重新入队或移动到 `DeadLettered`。
- 内置 chat turn 会一起 commit session entry 和 chat event。provider 或本地后处理失败时，会保留 user entry、脱敏后的 error
  event 和 failed turn-end event，供 replay/debug 调用方使用。
- `session_id` 标识持久化 timeline；`task_id` 是 task/report 元数据，不能再作为隐式 session fallback。
- Context primitive 属于 `ikaros-context`。Runtime chat 会把 relationship、显式 reference、history、memory 和
  RAG 组装成 provider-aware token-budgeted `ContextBundle`。Provider profile 和 registry metadata 会收窄可用
  context window，并选择 token estimator。OpenAI-compatible 和 mock provider 已有本地确定性 adapter；Anthropic 和
  Ollama 在精确 native tokenizer library 接入前仍使用显式 fallback adapter。Runtime chat 会通过
  `ContextEngineRegistry` 解析 context engine；当前 registry 暴露 deterministic
  local compressor descriptor 和
  `llm-summary` descriptor。`llm-summary` engine 会构造脱敏后的 provider-backed summary request，并把 provider
  summary 转成 runtime compaction evidence；更深的 semantic compression 质量和
  fallback policy 仍是后续 hardening。
- Prompt assembly 也属于 `ikaros-context` 的边界。Runtime chat 会把 context bundle、persona、policy、compression
  notice 和 tool guidance 转成 typed `PromptSection`，再渲染最终 system prompt。`ContextDiff` 只会持久化
  `PromptSectionMetadata`，供 replay/debug/UI 查询：kind、title、source、priority、token 估算和 redaction 状态。完整
  prompt section content 只作为内存里的 renderer 输入，不作为 session evidence 落库。
- `ContextReference` 当前会解析安全本地引用：`@file:path:line-line`、`@folder:path`、`@git:rev`、`@diff` 和
  `@staged`。路径必须留在 workspace 内。`@url:` 会通过 session `NetworkEgress` 抓取，并受配置里的精确 host allowlist 约束。
- Chat attachment 是模型 content block，不是绕过 runtime 的外部文件。CLI 的 `--image`、
  `--audio`、`--file` 和 workbench `/attach` 会把 workspace 内的本地路径解析成有大小上限的
  data URL；URL 和 data URL attachment 会在 provider 支持时作为 provider content 传入。
- Context assembly 会为每个 turn 发出 `ContextDiff` agent event。payload 包含 budget、context section、prompt
  section metadata、已解析 reference，以及新增、删除、压缩内容的 token 估算。
- Context compaction 会保护 relationship 事实和显式 reference。protected context 无法装入模型推导出的 budget 时，turn 会以
  context-limit 错误失败，而不是静默丢掉用户请求的上下文。
- `MemoryProvider` 暴露 turn_start、prefetch、sync_turn、pre_compress、session_switch 和
  delegation_observation lifecycle hook。Trait 不再隐藏默认 noop；确实不需要副作用时，调用方必须显式选择 `NoopMemoryProvider`。
- `MemoryScore`、`MemoryPolicy` 和 `MemoryJournal` 属于 `ikaros-memory`。Runtime chat 会把 `sync_turn`
  working-memory append/skipped-write 决策写入 journal；当 lifecycle report 关联到受影响的 core memory scope 时，
  才会应用配置的 promote/demote/forget/quota policy action，并把这些决策写入同一 journal。普通 turn summary 不会提升成长期
  `Task` memory。
- Relationship memory 是 `ikaros-memory` 里的 `MemoryKind::Relationship`；
  relationship CLI 只是 memory store 的便利入口，不是第二套记忆系统。
- 工具执行属于 harness 和 `ExecutionEnv`，不属于模型 provider 或 UI。`ikaros-host`
  会用 harness backend 组合出当前配置对应的 runtime environment。进程执行会清空
  宿主环境、恢复一小组基线 allowlist、叠加显式 request env，并对敏感 env 诊断脱敏。
  Sandbox debug report 解释当前 dry-run/workspace/network-restricted 矩阵，以及启用配置时的
  Docker-backed container 第一版。这个 container backend 会通过 `docker run --network none`
  执行进程，但它不是 VM、多租户边界或完整 OS sandbox。
- 本地 API、MCP、browser/CDP、web、vision 和 image-generation 界面都是 runtime、
  harness、session 和 provider 边界上的 adapter。它们必须复用 `NetworkEgress`、
  `ExecutionEnv`、provider governance、audit 和 session evidence，不能绕过 policy
  开侧门。
- Browser CDP 的 HTTP discovery 走受治理的 `NetworkEgress`；在更严格的 browser
  supervisor sandbox 完成前，页面自身网络请求仍由浏览器进程执行。文档和 UI 必须把这个区别写清楚。
- Web search 和 extract 是显式受治理的 skill。Search 可以使用内置 DuckDuckGo HTML
  provider，也可以使用配置的 Brave、Bing、SerpAPI 和 Tavily-compatible endpoint；
  extract 抓取单个 URL，并返回有大小上限、脱敏后的 citation text。
- Coding workflow 是受 harness 治理的 skill。它会构造 `CodingTurnContext`、git baseline、repo map、change plan、可选
  patch attempt、turn diff、test matrix evidence、review、iteration plan、loop report 和 final report。Git
  baseline 会在可用时记录 HEAD、branch/detached 状态、clean/dirty/not-git/unknown 状态，以及
  staged/unstaged/untracked 标记；没有 fixture 时，git status snapshot 通过 session `ProcessRunner` 路径采集。Mode
  policy 是显式的：`plan` 和 `review` 保持只读，`test` 只允许通过 harness process 路径运行 test matrix，`edit`
  可以应用明确请求的候选 patch，`self_modify` 在进入专用 self-modify 审批路径前会被普通 `code workflow` 拒绝。Workspace
  instruction 会从 `IKAROS.md` 和 `.ikaros/instructions.md` 读取，并在进入 prompt 或 event 前脱敏。设置
  `--model-loop` 时，配置的 model provider 会返回严格 JSON candidate patch；审批后的执行路径会把 model request/response
  metadata、token-budget stop、cancellation stop、patch attempt、test evidence、review 和 loop termination
  写入 `state.db`，供 `debug coding-turn` 查询。Terminal-first 的 `code plan`、`code apply`、`code test`、
  `code review`、`code rollback` 和 workbench `/code ...` 都路由到同一套 workflow。
- Tool lifecycle 使用 typed event：`ToolCallStarted`、`ToolCallOutputDelta`、`ToolCallCompleted`、
  `ToolCallFailed` 和 `ToolCallCancelled`。Approval event 会携带 tool anchor，方便 UI、replay 和 audit view
  对齐请求和工具调用。
- Agent-loop observer hook 覆盖 provider request/response 和 tool start/end 边界。Hook payload 是已脱敏元数据；
  typed event 和持久化 session timeline 仍是 durable observation surface。
- Tool scheduling 由 descriptor 决定。相邻的 parallel tool call 可以并发执行；sequential call 单独执行；per-tool
  timeout failure 会通过同一套 lifecycle event stream 报告，并带结构化 timeout metadata。Cancellation 会在 provider
  request 前、等待 provider request 时、已规划 tool call 启动前和 tool future 运行中检查；已规划但未启动的调用会被记录为 cancelled，
  不会执行。
- 稳定的产品界面协议类型放在 `ikaros-protocol`；gateway 自己的 inbox/outbox
  存储模型仍留在 `ikaros-gateway`。
- Self-modify 是单独审批路径，不是普通写权限。
- 当前 coding workflow 已经是 provider-backed 受控 loop，但仍是 pre-MVP。它现在已有 deterministic、mock-model 和
  provider-loop replay fixture、多轮 patch/test/review evidence、test-matrix event、顶层 `code ...` 与
  workbench `/code ...` 交互命令、基于持久化 turn diff evidence 的 rollback，以及 malformed range、quoted/space
  path 截断、ambiguous anchor、already-applied hunk、生成式 malformed corpus 和 generated line-update
  roundtrip 的 parser hardening。Coding approval request 会携带
  provider/shell/write/session 的结构化 context，
  终端会渲染 `approval_scope`、`coding_progress` 和 `coding_result` 摘要；provider-backed turn 在等待 provider
  call 时也可以响应取消，并在继续 patch/test 前写入 `coding_loop_cancelled`。更深的 property/fuzz 覆盖仍是后续 hardening。

## 不变量

- 模型响应不能直接作为 host 操作执行。Tool call 必须先标准化，再通过 `ExecutionSession` dispatch。
- Runtime event 是 turn 的 append-only 观察记录。Report 可以总结它们，但工具应优先依赖 typed event 字段，而不是解析面向人的文本。
- 持久化 session timeline 是 append-only 的。Branch、compact、retry、active leaf
  切换和 replay 会追加或选择 entry，而不是改写旧 turn 事实。
- Provider adapter 不拥有 agent loop、审批流或 workspace mutation policy。
- Context assembly 可以用脱敏 audit input 调用 safe-read skill，但 audit log 不应保存完整用户 prompt。
- 审批重放必须绑定 workspace、精确 approved input 和 agent identity。
- Gateway ingestion 只负责入队，不直接调用模型、任务、插件或工具。
- Self-modify proposal 使用单独的 proposal/apply/rollback 路径，不代表普通写权限。

## 失败报告

多数 runtime 路径会给内部调用方返回结构化 report，并给 CLI 渲染面向人的摘要。Report 应足以解释工作为什么停止，同时不能保存 prompt 文本或 secret。
常见停止条件包括策略拒绝、等待审批、迭代预算耗尽、guardrail halt、provider error、command timeout 和本地 store 错误。

当前 session replay 对已完成和失败的 chat / agent-loop turn 最可靠，也已经包含 gateway 和 schedule 的高层
request/result/delivery evidence。Memory 和 audit 仍有专用 store，所以 long-running worker 应把 `state.db` 当作主
timeline，把这些 store 当作辅助 evidence，直到它们的 lifecycle record 被完整建模。
