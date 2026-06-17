# 记忆模型

Ikaros 记忆默认保存在本地，并以带类型的 record 形式存储。

## Record 类型

支持的记忆类型：

- `User`
- `Project`
- `Task`
- `Persona`
- `Relationship`
- `Knowledge`

Record 包含 scope、content、timestamp、tag、source、结构化 `source_ref`、confidence 和 sensitivity flag。疑似 secret 内容在 append 或 update 前会被拒绝。

`source_ref` 可以指向 session turn、session entry、skill call 或 manual note。Runtime memory lifecycle 写入派生记忆时会用它关联来源 turn，但不会把 session store 变成 memory 数据库。

## Lifecycle 和策略

`MemoryProvider` 实现必须显式处理 lifecycle hook：`turn_start`、`prefetch`、`sync_turn`、`pre_compress`、`session_switch` 和 `delegation_observation`。确实不需要副作用的调用方应使用 `NoopMemoryProvider`。

`MemoryScore`、`MemoryPolicy` 和 `MemoryJournal` 定义了 promotion、demotion、forget、skipped write 和 quota 决策的本地边界。`JsonlMemoryJournal` 会把这些决策写入 `memory_journal.jsonl`。Runtime chat 会自动记录 `sync_turn` append 和 skipped-write 决策，然后对受影响的 memory scope 应用配置的 policy。Promote 和 demote action 会给 record 标记 `policy-promoted` 或 `policy-demoted`；forget action 会删除低于 forget threshold 的 record，或者删除被 `max_records_per_scope` 淘汰的 record。Journal 是 lifecycle/audit primitive，不是 memory store 的替代品。这轮 policy pass 只覆盖当前 turn 写入的 record 和相关 relationship scope，不是全库 memory compactor。Memory store mutation 和 journal 写入仍是分开的本地写；跨 memory store 与 journal 的事务/回放一致性仍是 hardening 工作。

## 后端

默认 JSONL 路径：

```text
IKAROS_HOME/memory/memory.jsonl
```

SQLite 路径：

```text
IKAROS_HOME/memory/memory.sqlite
```

在配置中选择后端：

```yaml
memory:
  backend: sqlite
```

## 关系记忆

关系记忆以普通 `Relationship` record 存储，但有独立 CLI：

```bash
ikaros relationship remember "Prefer concise updates" --scope user
ikaros relationship show --scope user
ikaros relationship forget --id <id>
```

Chat 可以在脱敏和去重后学习明确的偏好、称呼和 "remember this" 语句。使用 `--no-relationship-learning` 可以在单轮中关闭写入。

Relationship CLI 是 `MemoryKind::Relationship` 的便利入口，不是第二套记忆数据库。

## Harness 边界

记忆写入和删除都通过 harness skill。因此显式 memory 命令和 runtime 创建的 task/relationship record 都会经过策略、审计、脱敏和 secret 拒绝。
