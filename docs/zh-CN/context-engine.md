# Context Engine

Context 边界控制哪些本地状态可以进入一次模型 turn。它负责结构化 context section、reference 解析、token budget、prompt section，以及解释本轮为什么看到这些 context 的 diff record。

## 所有权

可复用的 context primitive 放在 `ikaros-protocol::context`：

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
- `LlmSummaryCompressor`

`ikaros-agent::chat::context_engine` 负责围绕这些 primitive 做应用编排：读取 relationship、显式 references、history、memory、RAG，添加 agent/tool guidance，通过 `PromptBuilder` 渲染最终 system prompt，并发出 session event。

`ikaros-protocol` 只保存稳定形状，不依赖 agent、execution 或 provider。需要 config、provider、store 或 execution env 的逻辑不能放进 protocol。

## Section

当前 chat context section 包括：

- relationship
- references
- history
- memory projection
- working memory
- retrieved memory
- RAG

Replay、debug 和 UI 应消费结构化 section 与 diff，而不是反向解析 prompt 文本。
