# 上下文压缩调研与 Ikaros 建议

调研日期：2026-09-13。本文只采用维护方自己的文档或源代码作为行为依据；外部项目的默认值和实现细节不能直接当作 Ikaros 的产品规格。

## 当前 Ikaros 基线

Ikaros 已经有一套适合接入压缩的输入审计骨架：`RunConfig` 冻结模型容量，`ContextRevision` 保存历史分组、历史 Item 引用、Memory 引用、遗漏和预算，`StepInput` 记录每次模型调用实际使用的 revision 与预算，`ModelInputPlanner.build_plan` 负责把这些引用物化为 Provider 请求。相关实现位于 [`runtime/src/ikaros_runtime/run_input.py`](src/ikaros_runtime/run_input.py)（`ContextRevision`、`StepInput`、`build_step_input`）和 [`runtime/src/ikaros_runtime/agent/model_input.py`](src/ikaros_runtime/agent/model_input.py)；调用循环位于 [`runtime/src/ikaros_runtime/agent/loop.py`](src/ikaros_runtime/agent/loop.py)（准备 Step、构建 plan、请求 Provider）。

目前预算不足会抛出 `ContextBudgetExceededError("context_budget_exceeded")`，Run 结束；当前实现没有语义摘要，也没有修改已有 revision 的压缩记录。这个边界和下一阶段目标在 [`MODEL_INPUT_AND_MEMORY_DESIGN.md`](MODEL_INPUT_AND_MEMORY_DESIGN.md) 和 [`LONG_TASK_PLAN.md`](LONG_TASK_PLAN.md) 中明确写出。Ikaros 已有 `history_read`，能按同一 Thread 读取已落盘的历史片段；这适合做压缩后的证据回溯，但不等于自动压缩。

## Codex（OpenAI）一方实现

来源：OpenAI Codex 开源仓库 README（项目身份）以及 `codex-rs/core/src/compact.rs`（实现），均为官方仓库：

- [Codex README](https://github.com/openai/codex/blob/main/README.md)
- [`compact.rs`](https://github.com/openai/codex/blob/main/codex-rs/core/src/compact.rs)

在 `compact.rs` 中可以直接看到以下设计：

1. 压缩既有自动入口 `run_inline_auto_compact_task`，也有用户手动入口 `run_compact_task`。两者都进入 `run_compact_task_inner`，并标记 `CompactionTrigger::Auto` 或 `Manual`，以及原因和阶段（例如独立 turn 或 turn 中途）。
2. 压缩前后分别运行 `run_pre_compact_hooks` 和 `run_post_compact_hooks`；hook 可以停止压缩，停止会被记录为中断并返回终止错误。压缩尝试拥有单独的 telemetry 状态，包含 trigger、reason、phase 和状态。
3. `CompactedHistoryMetadata` 单独记录摘要消息、窗口编号、窗口 ID，以及压缩请求的 response/model 标识。注释说明持久化的 `CompactedItem` 与实时历史会使用同一组 Item ID，避免两份历史漂移。
4. 初始上下文注入是显式策略：独立/手动压缩使用 `DoNotInject`，下一个普通 turn 再完整注入；turn 中途压缩使用 `BeforeLastUserMessage`，在最后一个真实用户消息前注入 world state/初始上下文。
5. 压缩本身是一个模型调用，使用专门的摘要 prompt。若该调用遇到 `ContextWindowExceeded`，源码会从历史最前面移除一个 Item 并重试，以保留较新的上下文；若预算或重试最终失败，则落盘错误并停止，不会假装压缩成功。

这些是 Codex 当前源码的实现事实；Codex 的具体服务端上下文容量或产品端阈值不应从该仓库推导。Ikaros 应借鉴其“压缩是可审计的独立模型调用”和“中途压缩与新 turn 的初始上下文策略不同”，而不是复制内部 Rust 类型或固定数字。

## Hermes Agent（Nous Research）一方实现

来源：Hermes Agent 维护方文档与仓库：

- [Context compression and caching 文档](https://hermes-agent.nousresearch.com/docs/developer-guide/context-compression-and-caching)
- [`agent/context_compressor.py`](https://github.com/NousResearch/hermes-agent/blob/main/agent/context_compressor.py)
- [`agent/model_metadata.py`](https://github.com/NousResearch/hermes-agent/blob/main/agent/model_metadata.py)
- [`hermes_cli/config_defaults.py`](https://github.com/NousResearch/hermes-agent/blob/main/hermes_cli/config_defaults.py)
- [Hermes Agent README 的命令表](https://github.com/NousResearch/hermes-agent/blob/main/README.md#cli-vs-messaging-quick-reference)

`context_compressor.py` 的模块 docstring 将策略概括为：保护 head/tail，对中间 turns 做摘要，先裁剪工具输出，支持迭代摘要和 token-budget tail。`ContextCompressor.compress(...)` 的 docstring进一步说明：工具结果和空 echo 会先清理（即使摘要调用中止也保留这个清理结果），然后摘要中间内容；`force` 可绕过失败冷却和可行性跳过，`bypass_cooldown` 则只绕过冷却。源码还包含摘要失败冷却、单次 fallback route pin，以及在摘要 worker 停滞时只重试一次的约束。

`model_metadata.py` 提供上下文容量解析、粗略 token 估算、模型 metadata 缓存及 `MINIMUM_CONTEXT_LENGTH` 等工具；其职责是为压缩器和主循环提供预算事实，而不是把 tokenizer 估算当作精确计费。Hermes README 将 `/compress` 和 `/usage` 列为 CLI/消息入口，说明手动压缩是产品能力；这不意味着 Ikaros 必须暴露同名命令。

Hermes 的实现还表明两个风险：摘要调用需要独立的超时、取消、fallback 和失败冷却，否则压缩本身会成为新的阻塞点；压缩后的历史必须清理孤儿 tool pair，否则下一次 Provider 请求可能无效。

## 对 Ikaros 的最小实现建议

### 1. 先在现有预算错误点插入一个有界压缩尝试

最小切入点是 [`agent/loop.py`](src/ikaros_runtime/agent/loop.py) 中 `prepare_model_step` / `ModelInputPlanner.build_plan` 抛出 `ContextBudgetExceededError` 的分支。只允许当前 Run 在该分支尝试一次（后续多次压缩可作为明确的后续迭代），压缩失败仍走现在的 `context_budget_exceeded` 终结路径。这样不会改变现有正常输入路径，也不会把无限重试变成隐式行为。

### 2. 把摘要当作独立、可审计的 ModelCall

新增一个压缩调用类型或等价的 `ModelCall.kind = "compaction"`，使用冻结的 `RunConfig` 模型/Provider、独立的摘要 prompt 和有界 timeout。写入 Journal 的事件至少应包含：`runId`、调用序号、触发原因（预算/手动）、压缩前 revision、摘要输出的脱敏版本、保留的原始 Item/Turn ID、压缩后的 revision、token 估算、结果和错误。摘要调用不得执行工具，也不得把秘密或未经清洗的外部文本写入持久化摘要。

### 3. 追加 revision，而不是覆盖旧 revision

扩展现有 `ContextRevision` 为“摘要 + 原始引用范围 + 压缩元数据”，并追加新的 revision 序号。原始历史和旧 revision 永远保留；`StepInput` 只引用当前 revision。保留系统指令、当前用户请求、当前 Run 未完成的工具链，以及完整的 assistant tool call/tool result 对；允许压缩较早的已结算 turns。`history_read` 继续从原始 Journal 读取证据，不从摘要反向重建原文。

### 4. 先做确定性裁剪，再做摘要

在调用摘要模型前，先清理超大工具输出、空的 assistant echo 和无法成对的 tool item；清理必须是纯函数且可测试。摘要请求优先保留历史头部的用户目标/约束、最近完整工具链和当前用户消息；中间已结算 turns 才进入摘要。摘要返回后校验必需字段、长度和脱敏结果，摘要无收益（输出不短或没有可压缩 middle）就保留原 revision 并终结预算错误。

### 5. 明确恢复与并发边界

每个 Run 设置 `compaction_attempts` 上限（首版建议 1），取消、Provider 断线、摘要超时、摘要失败都必须可落盘并可重放状态；不能自动再次执行旧工具。压缩期间若收到 steering，按现有 Run/Step 原子规则排队到下一个可用 Step；不要让两个压缩 worker 同时提交 revision。重启后若压缩调用未结算，保留 `compaction_unknown` 或失败记录，不把半截摘要当作事实。

## 必须补的验证

- 精确触发：预算刚好够、差一个 token、摘要无收益、摘要输出过长。
- 事实保留：最初用户约束、最近工具调用与结果、失败/取消 reasonCode、workspace 和 Memory revision 均仍在下一次请求。
- 结构完整：不会产生孤儿 assistant tool call/tool result；`history_read` 能找回被摘要替代的原始 Item。
- 安全：摘要和 Journal credential scan；工具输出中的秘密、提示注入和外部文本不能改变身份、工具权限或执行策略。
- 失败竞态：摘要超时、取消、Provider 401/断线、Runtime 重启和重复提交都只产生一个压缩 revision；原始 revision 不变。
- 成本与观测：压缩请求单独计数和计时，预算记录区分主模型 token 与压缩 token；UI 明确显示“已压缩/压缩失败”，不能把摘要当作用户消息。

建议先实现一次“预算溢出 → 确定性裁剪 → 一次摘要 → 新 ContextRevision → 继续当前 Run”的竖切片，再决定是否加入 Hermes 式多次压缩、手动 `/compress` 或更复杂 fallback。这样能复用 Ikaros 已有的预算、Journal、history_read 和失败恢复语义，风险和迁移面最小。

