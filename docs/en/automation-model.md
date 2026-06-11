# Automation Model

Scheduled automation stores local jobs and runs them through the same runtime/harness path as explicit task commands.

## State

Schedules live at:

```text
IKAROS_HOME/automation/schedules.jsonl
```

Delivery files live under:

```text
IKAROS_HOME/automation/deliveries/
```

Schedule records include redacted task text, optional agent profile, next run time, optional recurrence interval, enabled state, last run metadata, and delivery targets.

## Commands

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

`run-due` processes due jobs once. `worker` polls in a local process. Neither installs a daemon by itself.

## Execution

Due jobs are converted into runtime task reports and executed through the deterministic harness task runner. Policy, approvals, audit logging, memory writes, and agent profile overlays still apply.

Default delivery writes a redacted local Markdown report. `--delivery local-file` and `--delivery gateway-outbox` can be repeated to choose one or both delivery targets.
