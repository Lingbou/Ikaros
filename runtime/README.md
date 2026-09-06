# Ikaros Runtime

Ikaros is a local Agent Runtime supervised by Electron main. One authenticated
loopback WebSocket carries JSON-RPC requests and sequenced events. Standard output
contains only the readiness record; diagnostics use standard error.

The current implementation includes the daily-use Alpha and the first stage of
[the long-task plan](LONG_TASK_PLAN.md): model capacity, frozen Run budgets, and
managed commands. Automatic context compression, in-flight steering, and
completion checking are still planned. See [DESIGN.md](DESIGN.md) for architecture
and [MODEL_INPUT_AND_MEMORY_DESIGN.md](MODEL_INPUT_AND_MEMORY_DESIGN.md) for model
input and Memory boundaries.

## Implemented features

- SQLite Threads, optional workspaces, rename/archive/unarchive, paginated history,
  ordered event replay, and idempotent Thread/Turn submission.
- Streaming conversations through DeepSeek, Custom OpenAI-compatible providers,
  and a deterministic ScriptedProvider used for tests.
- Provider/Model settings, DeepSeek model discovery, model enablement, and editable
  model capacity. The initial defaults are a 32,768-token context window and a
  4,096-token output reserve; users should set the actual limits of their model.
- Per-Run limits, defaulting to **100 model calls and 60 minutes**. The Run freezes
  these values at submission. Its deadline includes tools and provider waits;
  reaching either limit settles the Run with an explicit reason.
- A provider-neutral model input plan and one current persistence format:
  `RunConfig`, `ContextRevision`, and per-call `StepInput`. Context accounting uses
  a conservative token estimate and reserves output capacity; billing totals
  remain based only on Provider-reported usage.
- Ordinary new Turns after failure. Persisted tool calls/results and a separate
  Runtime status block explain completed operations, incomplete work, and unknown
  effects. Partial assistant messages are excluded. Historical calls are input
  facts and are never automatically executed again.
- Managed shell commands and bounded UTF-8 `read`, atomic `write`, and exact-match
  `edit`. The file inspector reads current disk content and separately retrieves
  immutable write/edit operation differences.
- Runtime-owned `IKAROS.md` identity, frozen Skill descriptors with lazy `SKILL.md`
  reading, and explicit Memory Create/Correct/Forget with provenance. Bounded
  Global/current-workspace Memory recall freezes exact revisions for each Run.
- Provider-reported usage and daily activity through `usage.read`.

A workspace saved on a Thread supplies the default command working directory and
the base for relative file paths. Projects are Desktop groupings of these Threads,
not another Runtime execution hierarchy. One scheduler worker runs one Run at a
time. Commands may remain active while that Run performs other model/tool work.

## Managed commands

The provider-facing tools are:

| Tool | Inputs | Result |
| --- | --- | --- |
| `process_start` | `command`, optional `cwd` | Stable `processId` and current state; starting is not proof of success. |
| `process_read` | `processId`, optional `cursor` | Current state and a bounded output page. |
| `process_wait` | `processId`, optional `cursor`, `timeoutMs` | Wait for exit, or return `running` when the wait expires. The process remains alive. |
| `process_stop` | `processId`, optional `cursor` | Stop the owned process tree and return its final state. |

`process_run` has been removed. The Agent passes a trusted Run/Step/Item/Thread
execution context to every tool. Repeating the same start Item and arguments
returns the existing command; reusing that Item with different arguments fails.
The model cannot supply ownership IDs to start or adopt another Run's command.

Each Run can register 32 commands, with at most 4 active simultaneously. Each
stream retains at most 64 KiB; output is continuously drained after that limit.
Reads return at most 16 KiB and use Unicode-character cursors. `process_wait`
defaults to 1 second and permits at most 60 seconds per call. Windows uses
kill-on-close Job Objects; POSIX uses process groups. Root exit also cleans up
remaining descendants, including children holding output pipes open.

`process.recorded` persists intent before spawn, running status, bounded output
snapshots, and terminal facts. A rebuildable `process_sessions` projection holds
the latest record. Output snapshots are limited to four per second, and unchanged
full buffers are not repeatedly recorded. Credential guards span stream chunks
before content reaches tool results or storage.

A Run's completion, cancellation, deadline, or Runtime shutdown stops its command
trees and releases live buffers. Historical facts stay in SQLite. Restart changes
persisted active/start-pending commands to **unknown**, never reattaches an old
PID, and never reruns the command. Active Runs become `runtime_interrupted`;
queued Runs in the current format can execute normally. Continuing means starting
a new Turn that checks the current state.

For deterministic tests, `/process.run <command>` makes ScriptedProvider call
`process_start`, then `process_wait` until terminal, then summarize the result.
`/skill.run <name>` first reads that Skill's instructions and uses the same command
lifecycle. These are test-provider input conventions, not public tool names.

## File inspection and Memory

`file.preview` returns current UTF-8 text in pages capped at 50 KiB / 2,000 lines,
with line numbers and a file revision. Later pages must use that revision; disk
changes require refresh. Each call scans at most 8 MiB. Binary files, unsupported
encodings, and inaccessible content return explicit unavailable states. Runtime
resolves workspace paths; an outside path needs a matching same-Thread file tool
record. Preview text never becomes model input by itself.

Write/edit capture actual before/after bytes, each capped at 256 KiB / 5,000 lines.
Tool settlement and `file.change_recorded` commit together. The serialized event
is capped at 256 KiB; oversized captures retain metadata. `file.change.get` returns
that operation's capture, independently of later disk changes. Missing captures
stay unavailable. Process-created files can be previewed, but command changes do
not have captured diffs. Diff bodies are separate from model tool output.

Memory lives in its own `memory.db`. The model receives bounded exact revisions
through a contextual-data wrapper; input audit records contain references, not
Memory bodies. Correct affects later Runs, while Forget prevents sending a now
unavailable frozen revision. Memory is distinct from failed-task recovery facts.
There is no automatic Memory writer or Skill learning loop.

## Protocol and storage

`src/ikaros_runtime/protocol/spec.py` defines protocol **4**, Journal schema **7**,
29 post-initialize RPC methods, 13 persisted event types, and 7 provider tool IDs.
It generates `protocol/runtime-protocol.json` and Desktop's literal unions.
`protocol/golden-trace.json` exercises production Run execution, process facts,
file changes, history, and projection rebuilding in both Python and Desktop tests.

Session storage is **schema 10 only**. Old state formats, selectors, queued-Run
formats, and migration paths have been removed. An incompatible development
`state.db` fails startup with `state database schema is incompatible; reset required`.
There is no automatic migration or automatic deletion. Use a fresh development
Runtime home or deliberately rebuild disposable Session state while Runtime is
stopped. Configuration, Skills, Desktop preferences, and the independent
`memory.db` are separate from Session state; updating code does not reset them.

Offline maintenance acquires the same Runtime-home lock as the server:

```powershell
uv run --project runtime python -m ikaros_runtime storage check
uv run --project runtime python -m ikaros_runtime storage backup --output C:\Backups\ikaros-state.db
uv run --project runtime python -m ikaros_runtime storage repair-projections
```

Check validates canonical schema, Journal order, and reconstructed projections.
Backup includes committed WAL data through SQLite's backup API and refuses to
overwrite a destination. Projection repair creates a backup, then rebuilds derived
tables from the unchanged current-format Journal. These commands do not convert
old formats or reconstruct missing execution results.

## Development and current limits

From the repository root:

```powershell
uv sync --project runtime --locked
uv run --project runtime ruff check runtime
uv run --project runtime mypy runtime/src runtime/tests
uv run --project runtime pytest runtime/tests
uv run --project runtime python -m ikaros_runtime.protocol.generate --check
```

Tests use isolated Runtime homes. Normal Desktop launches use `~/.ikaros`.
Development requires Python 3.12 or 3.13; bundling a Python Runtime is deferred.

This stage does not implement automatic compression, in-flight user steering,
completion checking, a full process-log panel, PTY interaction, process survival
across Runs/restarts, multiple simultaneous Runs, cron, multi-Agent execution,
attachments, web/browser tools, or automatic Memory learning. An oversized context
currently stops the Run explicitly. `full_access` uses the OS user's authority and
is not a sandbox. The Runtime remains tied to Desktop's lifetime.

Earlier real-model evidence is recorded in [LIVE_VALIDATION.md](LIVE_VALIDATION.md).
The new kernel has deterministic process/recovery/deadline tests; Windows/Linux
real-model long-task acceptance remains a later milestone check.
