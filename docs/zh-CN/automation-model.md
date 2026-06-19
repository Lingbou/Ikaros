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

Schedule record 包含脱敏后的 task text、可选 agent profile、next run time、可选 recurrence interval、enabled 状态、last run 元数据和 delivery target。

每次 job 执行还会把脱敏后的 session evidence 写入解析后的 agent `state.db`。Automation JSONL 文件仍然是 schedule 的事实来源；session replay 记录 runtime request、result、delivery summary 和 failed-run event，便于调试和跨入口连续性。

## 命令

```bash
ikaros schedule add "summarize project status" --at now
ikaros schedule add "summarize project status" --at now --delivery local-file --delivery gateway-outbox
ikaros schedule list
ikaros schedule run-due --dry-run
ikaros schedule run-due --limit 5
ikaros schedule worker --once
ikaros schedule enable <id>
ikaros schedule disable <id>
ikaros schedule delete <id>
```

`run-due` 处理一次 due job。`worker` 在本地进程中轮询。它们都不会自己安装 daemon。

## 执行

Due job 会通过 session-aware task agent-loop path 转换为 runtime task execution。
Scheduler 会提供 schedule 派生的 session id、turn id 和 session source，因此 typed
event 可以和 schedule request/result/delivery evidence 落在同一个 `state.db` timeline
中。Policy、approval、audit logging、memory write 和 agent profile overlay 仍然适用。

默认 delivery 写入脱敏后的本地 Markdown report。`--delivery local-file` 和 `--delivery gateway-outbox` 可以重复传入，用来选择一个或两个投递目标。

Session id 由 schedule job id 派生，每次运行有 schedule-scoped turn id。Replay entry 会把计划任务写成 user message，并把脱敏后的 task result / delivery evidence 写成 runtime entry。
