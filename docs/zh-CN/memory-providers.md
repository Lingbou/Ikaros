# 记忆 Provider

记忆 provider 边界用于保持本地记忆为默认，并统一本地 store、registry 和 provider lifecycle。MVP 里可执行的 append/search/update/delete 仍由本地 JSONL 或 SQLite store 完成。

Provider API 是生命周期边界，不只是数据库抽象。Runtime turn 可以在固定阶段通知 memory，而不需要知道当前本地实现是 JSONL 还是 SQLite。

## 当前状态

已实现 provider：

- 本地 JSONL
- 本地 SQLite

`LocalMemoryStore` 实现 `MemoryProvider`。`NoopMemoryProvider` 是显式 disabled 实现，供调用方明确表示不需要 memory side effect。外部 provider 记录目前只是 descriptor 元数据，可以描述后续集成，但不是当前 runtime 的可执行 provider。

Registry state：

- `active`：可用 provider。
- `disabled`：存在 descriptor 元数据，但不可执行。
- `blocked`：配置不可用。

内置本地 provider 始终 active。外部 provider descriptor 在远程 adapter 启用前只是元数据；声明它不会重定向本地写入。`ikaros config validate` 会拒绝启用的外部 provider。

Memory policy 边界包括：

- `MemoryScore`：recency、relevance、frequency、source-strength、confidence 和 sensitivity 输入。
- `MemoryPolicy`：promote、demote、forget 和 per-scope quota 阈值。
- `MemoryJournal`：append-only 的策略/action 记录。
- `JsonlMemoryJournal`：本地 `memory_journal.jsonl` 实现。

Journal 是 memory lifecycle 决策的审计和 replay 辅助。Runtime chat 会把 `sync_turn` append 或 skipped-write 决策写入 journal；只有存在受影响的 core memory scope 时，才会继续记录对应的 promote、demote、forget 和 quota eviction 决策。Quota eviction 会以带 quota reason 的 `forget` action 写入 journal。Projection render、candidate accept/reject、working-memory expiry 和 supersession event 也会写入 journal，方便 debug 工具解释 projection surface 为什么变化、哪条 scratchpad record 为什么消失、哪条旧 memory 被替换。它不替代 memory store，也不代表外部 memory provider 已经可执行。当前 policy pass 是 turn-scoped，不是全库 compactor。

检查 provider 状态：

```bash
ikaros memory provider list
ikaros memory provider active
ikaros memory provider show local-jsonl
ikaros doctor
```

## Lifecycle

`MemoryProvider` 不只是 store/search 接口，还定义 turn 生命周期：

- `turn_start`
- `prefetch`
- `sync_turn`
- `pre_compress`
- `session_switch`
- `delegation_observation`

Trait 不再提供隐藏的默认 noop 方法。每个 provider 都必须实现全部 hook；确实不需要副作用时，调用方必须显式选择 `NoopMemoryProvider`。Runtime chat turn 会在 turn start 和 turn end 触发 memory lifecycle。

Lifecycle context：

- `turn_start`：在 context assembly 前接收 session id、agent id 和 user input。
- `prefetch`：接收 `MemoryQuery` 以及可选 session/agent id；本地 provider 会映射为 search。
- `sync_turn`：turn 结束后接收 session id、agent id、user input 和 assistant output。本地 provider 会在内容可安全存储时写入脱敏后的 session working-memory record，并带上 `MemoryRef::SessionTurn`；它不会把普通 turn summary 提升成长期 `Task` memory。如果发现脱敏 secret 标记，则报告 skipped write，而不是让 chat turn 失败。
- `pre_compress`：接收 session id、agent id 和目标 context budget。
- `session_switch`：接收 old/new session id 和 agent id。
- `delegation_observation`：记录 parent/child agent id 和 delegated work summary。

每个 lifecycle hook 返回 `MemoryLifecycleReport`，包含 phase、records-read、records-written 和 notes。Noop report 只来自显式 provider 实现，不再是 trait fallback。

Runtime chat 会把 `turn_start` 和 `sync_turn` report 持久化为 `MemoryLifecycle` session event。可以关联具体 turn 的 `sync_turn` report 会带上 `MemoryRef::SessionTurn`。如果本地 provider 在派生 turn summary 中发现 redaction marker，它会记录 skipped write，而不是存储 summary。普通成功 `sync_turn` 会以 working-memory append 写入 journal。

成功 `sync_turn` 后，runtime 只会对 lifecycle report 中关联到的 core memory scope 应用 `MemoryPolicy`。当前普通本地 sync 写入的是 working memory，所以通常只会产生 working-memory append journal，不会 promote、demote 或 forget core record。自动 relationship 观察会进入 pending candidate；只有 candidate 被接受后，才会成为 core relationship memory 并参与后续策略边界。存在受影响 core scope 时，同一轮也会检查对应 kind/scope 是否超过 `max_records_per_scope`。Promote/demote 决策会更新本地 tag；forget 决策会删除低分或 quota 淘汰的 record。每个 action 都会带 score 写入 `JsonlMemoryJournal`，可关联 turn 时会带上 `MemoryRef::SessionTurn`。Memory store update/delete 和 journal append 目前仍是分开的本地操作，跨 store 事务语义还要后续补齐。

查看持久化 lifecycle evidence：

```bash
ikaros debug memory-lifecycle <session-id>
ikaros debug memory-lifecycle <session-id> --turn-id <turn-id>
```

命令会读取 `state.db` 和 `memory_journal.jsonl`，报告 lifecycle phase、records read/written、`MemoryRef::SessionTurn`、skipped-write 原因、redaction 相关 note、action count 和 runtime memory policy action。

## Runtime Context

Chat context assembly 通过 harness safe-read skill 使用 memory。Projection 和 working-memory 读取是独立 safe-read tool，retrieved memory search 仍用于显式本地查询。Skill 用真实本地 query 执行，但写入脱敏 audit input，因此 audit log 不保存完整 prompt。Relationship memory 是 `MemoryKind::Relationship`；retrieved-memory section 会排除这种 kind，因为它会单独渲染进 relationship section。普通 `Task` turn summary 也会从 retrieved memory context 中排除。持久化 context event 会把 projection、working memory 和 retrieved memory 保持为独立 section，并带有各自的 trust/source metadata。两条路径都不能绕过 policy。

`ContextEngine` 负责什么时候组装和压缩 memory；`MemoryProvider` 负责 provider-specific lifecycle side effect。两者职责应保持分离：context assembly 不应直接实现远程 memory sync，memory provider 也不应构造模型 prompt。

## 配置

```yaml
memory:
  backend: jsonl
  external_providers:
    - id: team-memory
      provider: plugin
      enabled: false
      endpoint: http://127.0.0.1:8787
      api_key: "replace-with-your-provider-key"
```

保持 `enabled: false`。远程 append/search adapter 尚未实现，因此启用外部 memory provider 会被 `ikaros config validate` 拒绝。

## 边界

- 本地 memory 仍是默认路径。RAG 是本地 reference retrieval，但只有 profile 启用或用户传入 `--rag-top-k` 时才会自动注入。
- 外部 provider 配置不会自动替代本地 store。
- 在真实 adapter 实现前，外部 provider descriptor 不是 runtime 能力。
- Secret-like memory 内容会被拒绝或脱敏。
- Memory record 可以携带结构化 `MemoryRef`，例如 session turn、session entry、skill call 或 manual note。
- Runtime `sync_turn` working-memory append、skipped-write、promote、demote、forget 和 quota decision 都会写入 `MemoryJournal`。
- Relationship、task、project 和 knowledge memory 不应静默分叉到多个 provider。

## 失败处理

不支持的本地 backend 会在 store/provider 构造时失败。启用外部 provider 在当前 runtime 中是非法配置。疑似 secret 的记录会在存储前被拒绝。Provider lifecycle 失败应返回给调用方，不应静默丢弃写入。
