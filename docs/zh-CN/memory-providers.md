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
- `sync_turn`：turn 结束后接收 session id、agent id、user input 和 assistant output。本地 provider 会在内容可安全存储时写入脱敏后的 `Task` turn-summary record，并带上 `MemoryRef::SessionTurn`；如果发现脱敏 secret 标记，则报告 skipped write，而不是让 chat turn 失败。
- `pre_compress`：接收 session id、agent id 和目标 context budget。
- `session_switch`：接收 old/new session id 和 agent id。
- `delegation_observation`：记录 parent/child agent id 和 delegated work summary。

每个 lifecycle hook 返回 `MemoryLifecycleReport`，包含 phase、records-read、records-written 和 notes。Noop report 只来自显式 provider 实现，不再是 trait fallback。

## Runtime Context

Chat context assembly 通过 harness safe-read skill 使用 memory。Skill 用真实本地 query 执行，但写入脱敏 audit input，因此 audit log 不保存完整 prompt。Relationship memory 是 `MemoryKind::Relationship`；普通 memory section 会排除这种 kind，因为它会单独渲染进 relationship section。两条路径都不能绕过 policy。

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

- 本地 memory 和 RAG 仍是默认路径。
- 外部 provider 配置不会自动替代本地 store。
- 在真实 adapter 实现前，外部 provider descriptor 不是 runtime 能力。
- Secret-like memory 内容会被拒绝或脱敏。
- Memory record 可以携带结构化 `MemoryRef`，例如 session turn、session entry、skill call 或 manual note。
- Relationship、task、project 和 knowledge memory 不应静默分叉到多个 provider。

## 失败处理

不支持的本地 backend 会在 store/provider 构造时失败。启用外部 provider 在当前 runtime 中是非法配置。疑似 secret 的记录会在存储前被拒绝。Provider lifecycle 失败应返回给调用方，不应静默丢弃写入。
