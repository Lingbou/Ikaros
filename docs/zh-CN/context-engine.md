# Context 引擎

Context 边界控制哪些本地状态可以进入一次模型 turn。它不是 prompt 字符串拼接器；它负责结构化 context section、reference 解析、token budget，以及解释本轮为什么看到这些 context 的 diff record。

## 所有权

`ikaros-context` 拥有可复用 primitive：

- `ContextBundle`
- `ContextSection`
- `ContextReference`
- `ContextBudget`
- `ContextDiff`
- provider-aware token estimator adapter
- `PriorityContextEngine`
- `TrajectoryCompressor`

`ikaros-runtime` 负责围绕这些 primitive 编排。Runtime chat 仍会调用 harness safe-read skill 获取 memory 和 RAG，在 workspace 内解析本地 reference，渲染最终 system prompt，并发出 session event。

这样的拆分让 context 计量和 replay/debug 数据可以复用，同时避免 context crate 依赖 runtime、harness 或模型 provider。

## Section

当前 chat context section：

- relationship
- references
- history
- memory
- RAG

`system`、`developer` 和 `tool_results` section kind 已作为协议形状预留，但当前 chat prompt 还没有把它们作为独立 budgeted section 使用。

## Token Budget

Chat context 先使用 `DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET`，然后在有模型 provider 时用 provider metadata 收窄。`ModelContextProfile` 提供：

- context window
- 默认输出 token 预留
- tokenizer kind
- metadata source

Runtime 在组装本地 context 前，还会为 persona/system prompt 预留 token。持久化的 `ContextBudget` 会记录 requested budget、effective max tokens、used tokens、provider window、output reservation、system reservation、estimator 和 metadata source。

在 runtime chat 中，请求的 context budget 为 `0` 时，如果当前有 provider profile，就表示“使用模型推导出来的可用本地 context 窗口”。直接调用底层库仍可构造 unbounded `ContextBudget`，但 CLI turn 不应把 `0` 理解成可以超过模型窗口。

Estimator 会根据 provider profile 的 tokenizer kind 选择。当前 adapter 仍是本地确定性实现：OpenAI-compatible 模型使用偏 ChatML 的估算器，`mock` 使用稳定的 word-count 估算器方便测试，Anthropic/Ollama 在精确 native tokenizer 接入前使用显式 fallback heuristic adapter。持久化的 budget 会记录 adapter 名称，replay/debug 调用方可以知道本轮 context 是按哪条计量路径形成的。

## 配额和压缩

`PriorityContextEngine` 按 section 分配 effective context budget：

- relationship：10%
- 显式 references：35%
- history：20%
- memory：20%
- RAG：15%

`TrajectoryCompressor` 应用这套 quota policy，并记录被压缩 section 的确定性省略摘要。这些摘要用于解释哪些内容没有进入本轮 context，还不是模型生成的语义总结。正常行为不再依赖单行 `[truncated]` 截断。

Relationship 事实和显式本地 reference 是 protected boundary。普通 quota pass 可以围绕它们压缩 history、memory 和 RAG，但不能静默丢掉这些受保护 section。如果 protected context 本身已经超过 effective budget，assembly 会返回结构化 context-limit 错误，而不是发出一个看起来可用但实际缺上下文的 prompt。

发生 context compaction 时，compressor 还会生成 continuation prompt，明确告诉模型哪些 section 被压缩，以及不能编造被省略的细节。持久化 chat turn 会同时写入 `ContextCompacted` event 和 `SessionEntryKind::Compaction` entry。Assistant message 会挂在 compaction entry 后面，保留 session tree 的事实链。

## Reference

Parser 识别：

```text
@file:path:line-line
@folder:path
@git:rev
@diff
@staged
@url:https://example.test
```

本地 reference 解析受 workspace 限制：

- `@file` 读取当前 workspace 下的文件，并可选择行范围。
- `@folder` 列出 workspace folder 的直接子项。
- `@git` 使用本地 `git show --stat --oneline`。
- `@diff` 使用本地 `git diff -- .`。
- `@staged` 使用本地 `git diff --cached -- .`。

逃出 workspace 的路径会使 turn 失败。显式请求的本地路径不存在时也会失败，因为用户明确要求了这段 context。

`@url` 目前只解析，不抓取。网络 context reference 需要等 network policy 边界接入后才能执行。

## Session Event

每个 chat turn 在 context assembly 后都会发出 `AgentEventKind::ContextDiff`。Payload 包含：

- budget
- sections
- compressed sections
- compression summary
- 已解析 references
- before/after token 估算
- 新增、删除、压缩的 context 预览

Replay/debug/UI 应使用这个 event 检查 context，而不是解析已经渲染好的 prompt。

## 安全

Context assembly 可以用真实本地输入调用 safe-read skill，但审计输入会脱敏。Reference content 在进入 prompt 或 session event payload 前会脱敏。

Context engine 不执行工具，不绕过策略，也不授予写权限。它只为模型 turn 准备只读信息。
