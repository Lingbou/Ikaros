# Context 引擎

Context 边界控制哪些本地状态可以进入一次模型 turn。它负责结构化 context section、reference 解析、token budget、prompt section，
以及解释本轮为什么看到这些 context 的 diff record。最终 system prompt 是这些结构化 section 的 renderer 输出；UI 和 replay 代码应检查
section，而不是反向解析 prompt 字符串。`PromptBuildReport::system_messages_for_prompt_cache()` 会把 cache-stable 的
persona、policy 和 tool guidance 与动态 context section 拆开，让 provider adapter 只给稳定前缀打 prompt-cache 标记。
Prompt metadata 还会记录稳定前缀的确定性 hash、message 数和估算 token，replay/debug 可以判断可缓存前缀是否变化，同时不保存 prompt 原文。

## 所有权

`ikaros-context` 拥有可复用 primitive：

- `ContextBundle`
- `ContextSection`
- `ContextReference`
- `ContextBudget`
- `ContextDiff`
- `PromptBuilder`
- `PromptSection`
- provider-aware token estimator adapter
- `PriorityContextEngine`
- `TrajectoryCompressor`
- `LlmSummaryCompressor`，它会准备 provider summary request，并把脱敏后的 provider summary 转成带预算的 `ContextCompressionReport`

`ikaros-runtime` 负责围绕这些 primitive 编排。Runtime chat 仍会调用 harness safe-read skill 获取 memory 和 RAG，在
workspace 内解析本地 reference，添加 runtime/tool guidance，通过 `PromptBuilder` 渲染最终 system prompt，在支持的请求路径里把
cache-stable 和动态 system prompt layer 作为独立 model message 发送，并发出 session event。默认 context engine 是
`deterministic`；`--context-engine llm-summary` 会显式启用 provider-backed summary compressor。未知 engine
名称会 fail fast，不会静默降级。

这样的拆分让 context 计量和 replay/debug 数据可以复用，同时避免 context crate 依赖 runtime、harness 或模型 provider。

## Section

当前 chat context section：

- relationship
- references
- history
- memory projection
- working memory
- retrieved memory
- RAG

`system`、`developer` 和 `tool_results` section kind 已作为协议形状预留，但当前 chat prompt 还没有把它们作为独立 budgeted section 使用。

每个持久化 `ContextSection` 还会携带 contract：

- `trust_level`
- `source_kind`
- `injection_reason`
- `freshness`
- `scope`

默认映射：

- `relationship`：高信任，来源是已接受记忆，稳定新鲜度，用户 scope，用于关系核心事实。
- `references`：高信任，来源是显式 reference，当前新鲜度，workspace scope，用于用户明确引用的上下文。
- `history`：中等信任，来源是 session history，近期新鲜度，session scope，用于最近 episode history。
- `memory_projection`：高信任，来源是 memory projection，稳定新鲜度，用户 scope，用于已接受记忆投影。
- `working_memory`：中等信任，来源是 working memory，当前新鲜度，session scope，用于 session working memory。
- `retrieved_memory`：中低信任，来源是 retrieved memory，检索新鲜度，用户 scope，
  由显式 `memory_search` 或 `--memory-search-limit` 注入。
- `RAG`：中低信任，来源是 RAG index，检索新鲜度，workspace scope，用作显式 reference retrieval。

这份 contract 是 event payload 的一部分。Replay、debug 和 UI 调用方可以解释某段可见 context 来自已接受记忆、session
scratchpad/history、显式本地 reference，还是带 citation 的 RAG 片段。

## Prompt Section

`ContextSection` 记录组装了哪些本地上下文。`PromptSection` 记录这些上下文、persona、policy、compression guidance 和 tool
guidance 如何进入实际 system prompt。

每个 prompt section 携带：

- `kind`
- `title`
- `content`
- `source`
- `priority`
- `estimated_tokens`
- `redaction`

当前 chat prompt section kind 包括 persona、policy、relationship、references、history、memory projection、
working memory、retrieved memory、RAG、context compression 和 tool guidance。`content` 只存在于内存里的 renderer
输入。持久化的 `ContextDiff`、audit、replay 和 debug 输出使用 `PromptSectionMetadata`，只包含 kind、title、source、
priority、token 估算和 redaction 状态。疑似 secret 的内容在进入 renderer 前会脱敏，完整 prompt section content 不会作为
session evidence 落库。

可选的本地 context section 只有在有内容时才会生成 prompt section。空的 relationship、reference、history、memory 或 RAG
输入仍会体现在 context 计量里，但不会以 `none` block 渲染进 system prompt，也不会出现在 prompt section metadata 中。

普通 chat 默认不会执行长期 memory search。默认 memory surface 是已接受的 projection 加 session working memory。只有调用方传入
`--memory-search-limit`，或工具显式执行 `memory_search` 时，retrieved memory 才会进入上下文。

## Token Budget

Chat context 先使用 `DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET`，然后在有模型 provider 时用 provider metadata 收窄。`ModelContextProfile` 提供：

- context window
- 默认输出 token 预留
- tokenizer kind
- metadata source

Runtime 在组装本地 context 前，还会为 persona/system prompt 预留 token。持久化的 `ContextBudget` 会记录 requested budget、
effective max tokens、used tokens、provider window、output reservation、system reservation、estimator 和
metadata source。

在 runtime chat 中，请求的 context budget 为 `0` 时，如果当前有 provider profile，就表示“使用模型推导出来的可用本地 context 窗口”。
直接调用底层库仍可构造 unbounded `ContextBudget`，但 CLI turn 不应把 `0` 理解成可以超过模型窗口。

Estimator 会根据 provider profile 的 tokenizer kind 选择。当前 adapter 仍是本地确定性实现：OpenAI-compatible 模型使用偏
ChatML 的估算器，`mock` 使用稳定的 word-count 估算器方便测试，Anthropic/Ollama 在精确 native tokenizer 接入前使用显式 fallback
heuristic adapter。持久化的 budget 会记录 adapter 名称，replay/debug 调用方可以知道本轮 context 是按哪条计量路径形成的。

## 配额和压缩

`PriorityContextEngine` 按 section 分配 effective context budget：

- relationship：10%
- 显式 references：35%
- history：20%
- memory projection、working memory 和 retrieved memory：总共 20%
- RAG：15%

`TrajectoryCompressor` 应用这套 quota policy，并记录被压缩 section 的确定性省略摘要。这些摘要用于解释哪些内容没有进入本轮 context，
还不是模型生成的语义总结。正常行为不再依赖单行 `[truncated]` 截断。

Relationship 事实和显式本地 reference 是 protected boundary。普通 quota pass 可以围绕它们压缩 history、memory 和 RAG，
但不能静默丢掉这些受保护 section。如果 protected context 本身已经超过 effective budget，assembly 会返回结构化 context-limit 错误，
而不是发出一个看起来可用但实际缺上下文的 prompt。

发生 context compaction 时，compressor 还会生成 continuation prompt，明确告诉模型哪些 section 被压缩，以及不能编造被省略的细节。持久化
chat turn 会同时写入 `ContextCompacted` event 和 `SessionEntryKind::Compaction` entry。Assistant message
会挂在 compaction entry 后面，保留 session tree 的事实链。

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

逃出 workspace 的路径会使 turn 失败。显式请求的本地路径不存在时也会失败，因为用户明确要求了这段 context。二进制或非 UTF-8 的 `@file` 目标不会让 turn 失败；
它会被表示为结构化 reference notice，包含 workspace 相对路径和 byte size。文本 `@file` 内容会在 compression 前被限制，避免显式
reference 占用超过 effective local-context token budget 的一半；被截断的 reference 会带有说明限制原因的 marker。

`@url` 会通过 session `NetworkEgress` 抓取。受控 egress policy 默认拒绝，只允许
`execution.network.allowed_hosts` 或配置 provider 默认值中的精确 host，并且只允许
`http` 或 `https` URL。被拒绝的 host 或不支持的 scheme 会使 turn 失败。成功响应进入
context 前还会继续经过 guard：本地测试 transport 缺省 content type 时可以接受；显式
content type 只接受 plain text、Markdown、JSON、XML 或 YAML。HTML 和二进制响应会被表示为 skipped reference notice。响应正文超过 64 KiB
时会跳过而不是截断塞进 prompt；URL 和响应正文在任何可见 reference notice 中都会先脱敏。

## Session Event

每个 chat turn 在 context assembly 后都会发出 `AgentEventKind::ContextDiff`。Payload 包含：

- budget
- sections
- compressed sections
- compression summary
- prompt section metadata
- 已解析 references
- before/after token 估算
- 新增、删除、压缩的 context 预览

Replay/debug/UI 应使用这个 event 检查 context，而不是解析已经渲染好的 prompt。

使用 debug CLI 可以直接查看持久化 event payload，不需要反推 prompt：

```bash
ikaros debug context-diff <session-id>
ikaros debug context-diff <session-id> --turn-id <turn-id>
```

如果 session 或指定 turn 不存在，命令会失败。JSON 输出会脱敏，并包含 estimator、模型推导的 budget、section token 计数、prompt section
的 source/priority/token 元数据、已解析 reference、compressed/protected context evidence、compaction summary、
continuation prompt，以及本轮的 context-limit error event。

## 安全

Context assembly 可以用真实本地输入调用 safe-read skill，但审计输入会脱敏。Reference content 在进入 prompt 或 session event payload 前会脱敏。

Context engine 不执行工具，不绕过策略，也不授予写权限。它只为模型 turn 准备只读信息。
