# 记忆模型

Ikaros 记忆默认保存在本地。Runtime 会区�?durable memory、episode history �?reference retrieval�?

- Core memory：少量已接受�?typed record�?
- Memory projection：由 core memory 确定性渲染出来的 Markdown 视图�?
- Working memory：当�?session 的短�?scratchpad�?
- Candidate memory：等待提升为 core memory 的候�?inbox�?
- RAG：带 citation �?reference system，不�?memory promotion 路径�?

## Record 类型

支持的记忆类型：

- `User`
- `Project`
- `Task`
- `Persona`
- `Relationship`
- `Knowledge`

Record 包含 scope、content、timestamp、tag、source、结构化 `source_ref`、可�?observer/subject perspective
metadata、confidence、active/supersession 状态和 sensitivity flag。疑�?secret 内容�?append �?update 前会被拒绝。普�?
turn summary 不应进入长期 `Task` memory�?

`source_ref` 可以指向 session turn、session entry、skill call �?manual note。Runtime memory lifecycle
写入派生记忆时会用它关联来源 turn，但不会�?session store 变成 memory 数据库�?

## Perspective metadata

Core memory 可以选择携带 perspective�?

```bash
ikaros memory add --kind relationship --scope default \
  --observer alice --subject bob "Bob likes pancakes"
ikaros memory search --observer alice --subject bob "Bob"
```

`observer` 表示这条记忆属于谁的视角，`subject` 表示这条记忆描述谁。查询和 projection 可以用这组字段隔�?directional fact。这里吸收的�?Honcho
observer/observed representation 的边界经验，�?Ikaros 不因此依赖后台外部推理服务�?

## Lifecycle 和策�?

`MemoryProvider` 实现必须显式处理 lifecycle hook：`turn_start`、`prefetch`�?
`sync_turn`、`pre_compress`�?
`session_switch` �?`delegation_observation`。确实不需要副作用的调用方应使�?`NoopMemoryProvider`�?

`MemoryScore`、`MemoryPolicy` �?`MemoryJournal` 定义�?promotion、demotion�?
forget、skipped write、working-memory append �?quota 决策的本地边界�?
`JsonlMemoryJournal` 会把这些决策写入 `memory_journal.jsonl`。Runtime chat 会自�?
记录 `sync_turn` append �?skipped-write 决策。正�?`sync_turn` 会在
`memory/working/` 下写入脱�?working-memory record，不再追加长�?`Task` record�?
自动 relationship learning 只创�?pending memory candidate，不会把推断偏好直接提升�?
core memory。Promote �?demote action 会给已有 record 标记 `policy-promoted` �?
`policy-demoted`；forget action 会删除低�?forget threshold �?record，或者删除被
`max_records_per_scope` 淘汰�?record。Journal �?lifecycle/audit primitive，不�?
memory store 的替代品。当�?policy pass �?turn-scoped，不是全�?memory compactor�?

## 后端

默认 JSONL 路径�?

```text
IKAROS_HOME/memory/memory.jsonl
```

SQLite 路径�?

```text
IKAROS_HOME/memory/memory.sqlite
```

在配置中选择后端�?

```yaml
memory:
  backend: sqlite
```

Working memory 路径�?

```text
IKAROS_HOME/memory/working/working_memory.jsonl
```

Working memory �?session 隔离，并�?TTL 语义。查询默认不返回过期记录。可以用维护命令检查或清理�?

```bash
ikaros memory working list --session <session-id>
ikaros memory working list --session <session-id> --include-expired
ikaros memory working prune
```

`prune` 会从 scratchpad 文件中删除过�?working record，并为每条被删除�?record
写入 `working_memory_expired` journal action�?

Candidate inbox 路径�?

```text
IKAROS_HOME/memory/candidates.jsonl
```

Projection 路径�?

```text
IKAROS_HOME/memory/projections/USER.md
IKAROS_HOME/memory/projections/PROJECT.<scope>.md
IKAROS_HOME/memory/projections/MEMORY.md
```

## Projection �?Candidate

模型默认看到的长期记�?surface �?projection，不是任�?search result。可以用下面命令渲染和查看：

```bash
ikaros memory projection render --scope ikaros
ikaros memory projection show --scope ikaros
```

Projection renderer 会读�?typed core record，并排除 task summary、memory-lifecycle record、inactive
superseded record、sensitive record �?demoted record。Projection 文件是派�?artifact，可以从 memory store 重新生成�?

自动或推断出来的事实应先进入 candidate inbox，再提升�?core memory�?

```bash
ikaros memory candidate list
ikaros memory candidate accept <candidate-id> --reason "explicit user instruction"
ikaros memory candidate accept <candidate-id> \
  --supersedes <memory-id> \
  --reason "user corrected this"
ikaros memory candidate reject <candidate-id> --reason "temporary task scope"
```

创建 candidate 会向 `memory_journal.jsonl` 写入 `candidate_created` action，并带上 candidate id、kind、scope
和可用的 source reference。接�?candidate 会追�?core memory record，并刷新�?scope �?projection。拒绝的 candidate 会留�?
inbox 里，方便审计。Projection render �?candidate accept/reject 也会�?journal 写入 `projection_rendered`�?
`candidate_accepted`、`candidate_rejected` �?`superseded` action，并带上 candidate id、被替换 memory id �?
projection scope�?

## Supersession

长期记忆更新使用 supersession，而不是直接删除历史。替�?record 可以通过
`supersedes` �?`superseded_by` 把旧 record 标记�?inactive，并记录
`valid_from` �?`valid_until`。普�?`memory list` �?`memory search` 默认不返�?
inactive record；排�?supersession 历史时需要显式传�?`--include-inactive`�?
使用 `ikaros memory supersession <memory-id>` 可以解释链路任意一侧：当前 record�?
它替换的 record、替换它�?record，以及有效期时间戳。Projection 只渲�?active
record。这�?Ikaros 可以解释当前记忆为什么替换了旧事实�?

## 关系记忆

关系记忆以普�?`Relationship` record 存储，但有独�?CLI�?

```bash
ikaros relationship remember "Prefer concise updates" --scope user
ikaros relationship show --scope user
ikaros relationship forget --id <id>
```

Chat 可以在脱敏和去重后提取明确的偏好、称呼和 "remember this" 语句。“我希望你这�?..”这类短期指令不会被接受�?relationship memory。自动提取出的观察先进入
candidate inbox，只有执�?`ikaros memory candidate accept` 后才会成�?core relationship record。使�?
`--no-relationship-learning` 可以在单轮中关闭 candidate 创建�?

Relationship CLI �?`MemoryKind::Relationship` 的便利入口，不是第二套记忆数据库�?

## Harness 边界

模型或工具驱动的记忆写入和删除通过受治�?skill。CLI projection/candidate 维护�?
复用本地 store 的校验和 secret 拒绝。Runtime 创建的普�?turn 状态进�?
working memory；已接受�?candidate、显�?relationship 命令和显�?memory 命令仍然�?
core memory�?
