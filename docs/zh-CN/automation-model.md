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

Due job 会转换为 runtime task report，并通过确定性的 harness task runner 执行。Policy、approval、audit logging、memory write 和 agent profile overlay 仍然适用。

默认 delivery 写入脱敏后的本地 Markdown report。`--delivery local-file` 和 `--delivery gateway-outbox` 可以重复传入，用来选择一个或两个投递目标。
