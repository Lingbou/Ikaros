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

Schedule records include redacted task text, optional agent profile, next run
time, optional recurrence interval, enabled state, retry/backoff policy, grace
period, timezone label, bounded run history, last run metadata, and delivery
targets. The schedule file is rewritten through a sibling temporary file and
atomic rename so a worker crash or failed write does not truncate the previous
job file.

Each executed job also writes redacted session evidence into the resolved
agent's `state.db`. The automation JSONL file remains the schedule source of
truth; session replay records the runtime request, result, delivery summary, and
failed-run events for debugging and cross-entry continuity.

## Commands

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

`run-due` processes due jobs once. `worker` polls in a local process. Neither installs a daemon by itself.
Per-job failures are isolated: a failed scheduled job is reported as a failed
run and does not stop later due jobs in the same worker tick.

## Execution

Due jobs are converted into runtime task executions through the session-aware
task agent-loop path. The scheduler supplies a schedule-derived session id, turn
id, and session source, so typed events can share the same `state.db` timeline
as schedule request/result/delivery evidence. Policy, approvals, audit logging,
memory writes, and agent profile overlays still apply.

Default delivery writes a redacted local Markdown report. `--delivery local-file` and
`--delivery gateway-outbox` can be repeated to choose one or both delivery targets.

Failed one-shot jobs can be retried before they are disabled. `--retry-max-attempts`
counts the initial attempt plus retries, and `--retry-backoff-seconds` schedules
the next attempt after a failed run. `--grace-period-seconds` prevents stale jobs
from running after the accepted lateness window has passed. `--timezone` is stored
as metadata for operators; timestamps remain RFC3339 UTC values.

The session id is derived from the schedule job id, and each run gets a
schedule-scoped turn id. Replay entries include the scheduled task as a user
message and the redacted task result/delivery evidence as a runtime entry.
