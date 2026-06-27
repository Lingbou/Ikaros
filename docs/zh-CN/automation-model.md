# 自动化模型

计划自动化保存本地 job，并通过显式 task 命令相同的 runtime/harness 路径执行。

## 状态

Schedule 存储在：

```text
IKAROS_HOME/automation/schedules.jsonl
```

Delivery 文件位于：

```text
IKAROS_HOME/automation/deliveries/
```

Schedule record 包含脱敏后的 task text、可选 agent profile、next run time、可选 recurrence interval、enabled 状态、
retry/backoff policy、grace period、timezone label、有上限的 run history、last run 元数据和 delivery target。
Schedule 文件会先写同目录临时文件，再用 atomic rename 替换，因此 worker 崩溃或写入失败不会截断旧 job 文件。

每次 job 执行还会把脱敏后的 session evidence 写入解析后的 agent `state.db`。Automation JSONL 文件仍然是 schedule 的事实来源；
session replay 记录 runtime request、result、delivery summary 和 failed-run event，便于调试和跨入口连续性。

## 命令

```bash
ikaros schedule add "summarize project status" --at now
ikaros schedule add "summarize project status" --at now --delivery local-file --delivery gateway-outbox
ikaros schedule add "summarize project status" \
  --at now \
  --retry-max-attempts 3 \
  --retry-backoff-seconds 60 \
  --grace-period-seconds 300 \
  --timezone UTC
ikaros schedule list
ikaros schedule run-due --dry-run
ikaros schedule run-due --limit 5
ikaros schedule worker --once
ikaros schedule enable <id>
ikaros schedule disable <id>
ikaros schedule delete <id>
```

`run-due` 处理一次 due job。`worker` 在本地进程中轮询。它们都不会自己安装 daemon。
单个 job 失败会被隔离成 failed run report，不会阻塞同一次 worker tick 里后续 due job。

## 执行

Due job 会通过 session-aware task agent-loop path 转换为 runtime task execution。
Scheduler 会提供 schedule 派生的 session id、turn id 和 session source，因此 typed
event 可以和 schedule request/result/delivery evidence 落在同一个 `state.db` timeline
中。Policy、approval、audit logging、memory write 和 agent profile overlay 仍然适用。

默认 delivery 写入脱敏后的本地 Markdown report。`--delivery local-file` 和 `--delivery gateway-outbox` 可以重复传入，用来选择一个或两个投递目标。

失败的一次性 job 可以在禁用前重试。`--retry-max-attempts` 表示初次执行加重试在内的总尝试次数，`--retry-backoff-seconds` 表示失败后多久再次尝试。
`--grace-period-seconds` 用来防止错过太久的 stale job 继续运行。`--timezone` 作为运维元数据保存，实际时间戳仍使用 RFC3339 UTC。

Session id 由 schedule job id 派生，每次运行有 schedule-scoped turn id。Replay entry 会把计划任务写成 user message，
并把脱敏后的 task result / delivery evidence 写成 runtime entry。
