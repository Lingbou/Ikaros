# Context 引擎

Context 边界控制哪些本地状态可以进入一次模型 turn。它不是 prompt 字符串拼接器；它负责结构化 context section、reference 解析、token budget，以及解释本轮为什么看到这些 context 的 diff record。

## 所有权

`ikaros-context` 拥有可复用 primitive：

- `ContextBundle`
- `ContextSection`
- `ContextReference`
- `ContextBudget`
- `ContextDiff`
- 启发式 token 估算

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

Chat context 使用 `DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET` 和启发式 token estimator。Budget 为 `0` 表示不限制。

这个 estimator 是本地确定性的，足够支撑 MVP context 计量，但它不是 provider-native tokenizer。后续 provider registry 应提供模型专属 context window 和 tokenizer adapter。

当前预算优先级：

1. relationship
2. 显式 references
3. history
4. memory
5. RAG

当某一行超过剩余 budget 时，可能会用 `[truncated]` 标记截断。被省略和截断的内容会写入 context diff。

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
- 已解析 references
- before/after token 估算
- 新增、删除、压缩的 context 预览

Replay/debug/UI 应使用这个 event 检查 context，而不是解析已经渲染好的 prompt。

## 安全

Context assembly 可以用真实本地输入调用 safe-read skill，但审计输入会脱敏。Reference content 在进入 prompt 或 session event payload 前会脱敏。

Context engine 不执行工具，不绕过策略，也不授予写权限。它只为模型 turn 准备只读信息。
