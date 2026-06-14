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

Each executed job also writes redacted session evidence into the resolved
agent's `state.db`. The automation JSONL file remains the schedule source of
truth; session replay records the runtime request, result, delivery summary, and
failed-run events for debugging and cross-entry continuity.

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

The session id is derived from the schedule job id, and each run gets a
schedule-scoped turn id. Replay entries include the scheduled task as a user
message and the redacted task result/delivery evidence as a runtime entry.
