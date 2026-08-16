# Ikaros Runtime

The Ikaros Runtime is a long-lived local Python process supervised by the
Electron main process. The implemented architecture and current vertical-slice
boundary are recorded in [DESIGN.md](DESIGN.md). The active, strictly serial
model-input and long-term-Memory stage is specified separately in
[MODEL_INPUT_AND_MEMORY_DESIGN.md](MODEL_INPUT_AND_MEMORY_DESIGN.md); its status
table distinguishes completed Gates from proposed capabilities.

Ikaros Desktop starts one Runtime for the application lifetime and connects to
it through authenticated loopback WebSocket JSON-RPC. Standard output carries
only the machine-readable readiness record used to discover the ephemeral
port; standard error carries diagnostics, and business traffic never uses
stdio.

## Protocol contract

`src/ikaros_runtime/protocol/spec.py` is the source of truth for protocol
version 2, Journal Event schema 4, the 26 post-initialize RPC methods, the 11
persisted Journal Event types, capabilities, and provider-facing Tool IDs. The
deterministic generator
commits both `protocol/runtime-protocol.json` and Desktop's
`src/shared/generated/runtimeProtocol.ts`; CI-style verification is available
without rewriting either file:

```powershell
uv run --project runtime python -m ikaros_runtime.protocol.generate --check
```

`protocol/golden-trace.json` is consumed by both Python and Desktop tests. It
covers initialize, catalog/history pages, every Journal Event discriminator,
both `item.completed` payload families, Tool Call/Result Items, usage, and Run
settlement. Desktop parses the fixture with its production wire parsers, so an
unknown method/Event or malformed discriminated payload fails before it can
advance the live Journal cursor. Machine protocol capabilities expose
`process_run`; `process.run` remains only a Desktop label and deterministic
slash-command syntax.

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
- exact Provider-reported model usage captured once per completed Provider Step
  in `model.response_finished`, projected into rebuildable SQLite
  `model_usages`, and aggregated by `usage.read` for the Desktop Profile metrics
  and 52-week activity chart;
- a Provider-neutral `ModelInputPlanV1` between the Agent loop and
  `ContextBuilder`, with versioned Output Style, Runtime-owned `IKAROS.md`
  Identity Core, and frozen Run Skill Catalog instruction blocks, explicit
  empty Context Data, Provider-default generation options, and the actual
  bounded input budget; the resulting OpenAI wire body remains locked by a
  two-Step Golden Test;
- a bundled, read-only `src/ikaros_runtime/resources/IKAROS.md` loaded through
  `importlib.resources`. `turn.start` freezes its version 1
  `ikaros-identity`/`runtime_identity` Instruction Block into the Submission
  Frame. Every Step uses the frozen block; recovery fails on release-Identity
  drift instead of substituting mutable configuration or Memory;
- Gate 4 requires no Session-schema reset. Completed pre-Gate-4 history with a
  null `identityCore` stays readable, but unfinished legacy execution is not
  resumed under a different identity: queued Runs fail with
  `identity_core_changed` before a Provider call and running Runs settle through
  the existing `runtime_interrupted` recovery path. No compatibility fallback
  injects the current Identity into an old Run;
- durable Gate 2 input auditing: `SubmissionFrameV1` and `RunManifestV1`
  persisted when a Turn is submitted, one frozen `ContextSnapshotV1` per Run,
  and one `StepManifestV1` plus terminal response metadata per Provider Step;
  `model.input_prepared` / `model.response_finished` form the persisted Step
  lifecycle;
- Gate 3 bounded history selection: complete prior Turns are read newest-first
  in 32-Turn pages under a 48,000-character limit, with 12,000 characters
  reserved during the first Step for current-Run growth. The first Turn that
  does not fit becomes the omission boundary; later Steps load frozen Item IDs
  plus current-Run Items instead of scanning the Branch;
- Skills V0, including safe one-level discovery below
  `~/.ikaros/skills/<name>/SKILL.md`, catalog diagnostics, global
  enable/disable state, immutable enabled-descriptor snapshots per Run, and
  provider context containing only each enabled Skill's name, description, and
  location;
- explicitly managed Memory V0 in independently durable `~/.ikaros/memory.db`:
  schema version 1 records/revisions/idempotency receipts, Global/Workspace
  scopes, `memory.create/correct/forget/list/get`, bounded preview pagination,
  optional verified Session Item provenance, authenticated JSON-RPC, and a
  typed Desktop bridge. Correct/Forget use optimistic revisions and durable
  receipts; Forget removes stored content and derived digests while retaining
  a queryable tombstone. Memory is not injected into model input and has no
  Renderer management page yet;
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
`process.run` and keeps the three file Tool names unchanged.

Skills V0 discovers metadata, exposes catalog diagnostics and global
enablement, freezes enabled descriptors into each Run, and injects that
descriptor catalog. It does not yet select Skills per task, enforce a catalog
budget, preload Skill bodies, produce a dedicated Skill execution Item, or
attribute and aggregate Skill script executions. A relevant `SKILL.md` is read
on demand through the ordinary `read` Tool, and a declared script is invoked
through the ordinary `process_run` Tool rather than imported into the Runtime.

The Runtime does not yet provide web-search, browser, or attachment Tools,
generate first-class Artifact/file-change records, fork Branches, retry or
resume Runs, or implement interactive permission approval. Desktop's fixed
scenario fixtures, slash-command examples, and
permission/recovery/Artifact demonstrations therefore remain mock-only UI
surfaces rather than Runtime-backed capabilities. Provider health remains
`unknown`, and automatic model discovery is DeepSeek-only; Custom
OpenAI-compatible Provider models are entered manually.

Providers that do not report stream usage remain usable, but their calls are
not estimated or added to Token totals. Profile metrics and activity include
only Provider-reported usage. The Skills settings page exposes real catalog and
enablement state, but no Skill execution statistics exist yet.

`FullAccessPolicy` automatically permits registered Tools; it is not a sandbox
or a security guarantee. The Runtime is tied to the Desktop application
lifetime and is not yet a Windows Service, login item, or independently
discoverable daemon.

Durable cross-Thread Memory now supports explicit Create/Correct/Forget,
Session Item provenance, restart-safe mutation receipts, and tombstone reads.
The Desktop Settings page provides real lazy-loaded filters, cursor pagination,
provenance status, correction conflict handling, and Forget confirmation.
Model recall is not implemented. Standalone Memory maintenance, backup, and
import/export are deferred rather than prerequisites.
Identity Core, persistent
input Frames/Manifests, and `bounded-history-v1` remain independent from Memory;
their boundaries and the remaining implementation order are defined in
[MODEL_INPUT_AND_MEMORY_DESIGN.md](MODEL_INPUT_AND_MEMORY_DESIGN.md).

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
independent `state.db` and `memory.db` databases on startup, while `config.yaml`
remains absent until the user saves a real provider/model configuration or
changes persisted Skill enablement.

During pre-release development, conversation storage uses canonical SQLite
schema version 7 and is intentionally reset-only. Incompatible `state.db`
schema versions fail with `reset required`; the Runtime does not carry
old-schema migrations or silently delete data.
After stopping the owning Runtime, developers may explicitly remove
`state.db`, `state.db-wal`, and `state.db-shm` to create the current schema on
the next start. This reset never includes `config.yaml`, `skills/`, Desktop
preferences, or the separately owned `memory.db`. An incompatible Memory schema
fails startup with `memory database schema is incompatible`; the Runtime does
not migrate, reset, or delete it automatically.

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
replays only the disposable Thread/Branch/Turn/Run/Run-input/Item/model-Step/
model-usage projection tables in one rollback-safe transaction. It accepts
broken projection foreign keys when
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
