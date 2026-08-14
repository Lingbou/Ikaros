# Ikaros Runtime

The Ikaros Runtime is a long-lived local Python process supervised by the
Electron main process. The architecture and first vertical-slice boundary are
recorded in [DESIGN.md](DESIGN.md).

Ikaros Desktop starts one Runtime for the application lifetime and connects to
it through authenticated loopback WebSocket JSON-RPC. Standard output carries
only the machine-readable readiness record used to discover the ephemeral
port; standard error carries diagnostics, and business traffic never uses
stdio.

## Implemented vertical slice

The current Desktop/Runtime path provides:

- canonical Threads in SQLite, including an optional per-Thread workspace and
  nullable `archivedAt`, separately paginated active/archived catalogs, paginated
  history and Thread detail reads, rename/archive/unarchive lifecycle commands,
  incremental event replay, restart recovery, and idempotent Thread/Turn
  creation;
- streamed, multi-Turn conversation through the deterministic
  `scripted/scripted-v1` provider or a configured DeepSeek/Custom
  OpenAI-compatible provider;
- Runtime-owned Provider and model configuration, DeepSeek model discovery,
  model enablement, DeepSeek disconnect, and Custom Provider removal;
- exact Provider-reported model usage captured per Agent Step, persisted in the
  append-only Journal and a rebuildable SQLite projection, and aggregated by
  `usage.read` for the Desktop Profile metrics and 52-week activity chart;
- the provider-facing `process_run`, `read`, `write`, and `edit` Tools under
  V1's fixed `full_access` policy. Command execution has bounded output,
  timeout, cancellation, and child-process-tree cleanup; the file Tools provide
  streaming bounded UTF-8 reads, verified atomic writes, and exact-match edits
  that reject common stale-content races, normalize mixed line endings, and
  serialize canonical and symlink-alias paths; and
- Desktop projection of messages, all four Tool families, Run state,
  cancellation, and workspace-backed Project groups.

The live DeepSeek path and its credential-leak checks are recorded in
[LIVE_VALIDATION.md](LIVE_VALIDATION.md).

Projects are not Runtime entities. Desktop derives them from the optional
workspace stored on each Thread; project and ordinary conversations otherwise
use the same Thread, Turn, Run, and Item path. A workspace directory becomes
the default working directory for `process_run` when the Tool Call does not
supply `cwd`, and the base for relative `read`, `write`, and `edit` paths.

`thread.list` returns active Threads by default; passing `archived: true` reads
the separate archived catalog. Renaming and archive-state changes are
append-only Journal transitions projected in the same SQLite transaction. An
archive request is rejected while that Thread has a queued or running Run, and
an archived Thread cannot accept a new Turn. Repeating the current title or
archive state is a no-op and does not append a duplicate Event.

## Current limits

The current Runtime registers the provider-facing `process_run`, `read`,
`write`, and `edit` Tools; Desktop maps `process_run` to the product label
`process.run` and keeps the three file Tool names unchanged. It does not yet
discover or inject Skills, provide
web-search, browser, or attachment Tools, generate first-class
Artifact/file-change records, fork Branches, retry or resume Runs, or implement
interactive permission approval. Desktop's fixed scenario fixtures,
slash-command examples, and permission/recovery/Artifact demonstrations
therefore remain mock-only UI
surfaces rather than Runtime-backed capabilities. Provider health remains
`unknown`, and automatic model discovery is DeepSeek-only; Custom
OpenAI-compatible Provider models are entered manually.

Providers that do not report stream usage remain usable, but their calls are
not estimated or added to Token totals. Profile activity insights and Skills
statistics are not exposed until those capabilities have real Runtime data.

`FullAccessPolicy` automatically permits registered Tools; it is not a sandbox
or a security guarantee. The Runtime is tied to the Desktop application
lifetime and is not yet a Windows Service, login item, or independently
discoverable daemon.

## Development

Install the locked environment from the repository root:

```powershell
uv sync --project runtime --locked
```

Run the Runtime checks:

```powershell
uv run --project runtime ruff check runtime
uv run --project runtime mypy runtime/src runtime/tests
uv run --project runtime pytest runtime/tests
```

The Runtime server is normally started by Ikaros Desktop. Its standard output
is reserved for one machine-readable readiness record; diagnostics go to
standard error.

Runtime-owned state defaults to `~/.ikaros`. Tests pass an isolated
`IKAROS_HOME`; normal Desktop launches do not override it. The Runtime creates
`state.db` on startup, while `config.yaml` remains absent until the user saves a
real provider/model configuration.

During pre-release development, conversation storage uses canonical SQLite
schema version 4 and is intentionally reset-only. Incompatible `state.db`
schema versions fail with `reset required`; the Runtime does not carry
old-schema migrations or silently delete data.
After stopping the owning Runtime, developers may explicitly remove
`state.db`, `state.db-wal`, and `state.db-shm` to create the current schema on
the next start. This reset never includes `config.yaml`.

## Offline storage maintenance

Stop Ikaros Desktop before running storage maintenance. Each command acquires
the same `runtime.lock` as the server and fails immediately while another
Runtime owns the home:

```powershell
uv run --project runtime python -m ikaros_runtime storage check
uv run --project runtime python -m ikaros_runtime storage backup --output C:\Backups\ikaros-state.db
uv run --project runtime python -m ikaros_runtime storage repair-projections
```

`storage check` opens an existing `state.db` read-only. It checks the SQLite
schema and pages, foreign keys, the strict Journal Event schema and contiguous
sequence/high-water mark, then rebuilds projections in a disposable in-memory
database and compares them with the stored query projections.

`storage backup` uses SQLite's Backup API rather than copying the main file, so
committed WAL content is included in one verified standalone database. It
refuses to overwrite an existing destination. Without `--output`, it creates a
unique file below `~/.ikaros/backups/`.

`storage repair-projections` first creates a pre-repair backup, then deletes and
replays only the disposable Thread/Branch/Turn/Run/Item/model-usage projection
tables in one rollback-safe transaction. It accepts broken projection foreign keys when
making that recovery snapshot, but the Journal must remain readable and
canonical; the repaired database must pass all integrity checks. Journal rows,
sequence state, `config.yaml`, and credentials are never rewritten by repair.
`--backup-output` selects the pre-repair backup path.

These commands do not implement Journal compaction, checkpoints, rollback, or
legacy-data migration. An incompatible schema or Event payload still requires
the explicit reset described above.

The deterministic `scripted/scripted-v1` provider supports ordinary streamed
conversation and one explicit Tool-loop smoke syntax:

```text
/process.run <command>
```

This invokes the provider-safe `process_run` Tool under V1's fixed
`full_access` policy, persists its bounded result, and asks the scripted
provider for a final answer. It exists for deterministic Runtime/Desktop tests;
real providers request `process_run`, `read`, `write`, and `edit` through their
normal tool-calling protocol. The opt-in DeepSeek smoke has exercised the real
`write -> read -> edit -> read` sequence and verified both the resulting file
and the final model answer.
