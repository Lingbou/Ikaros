# Ikaros Runtime Architecture

Status: first vertical-slice decisions locked on 2026-08-11 and implemented on
2026-08-12. This is a living architecture record: sections describing the
current vertical slice reflect the implementation, while explicitly marked
future capabilities remain design direction rather than shipped behavior.

The completed model-input and Memory foundation stage is specified in
[MODEL_INPUT_AND_MEMORY_DESIGN.md](MODEL_INPUT_AND_MEMORY_DESIGN.md). Gates 0–9
are complete, including the Runtime-owned `IKAROS.md` Identity Core, the
independent explicitly managed Memory Store/RPC/typed Desktop bridge and its
real Settings page, deterministic bounded Memory recall, and the final offline
plus real-DeepSeek verification/audit Gate. No standalone Memory maintenance,
Memory backup/export/import, or Memory transfer surface is part of the current
stage; this does not remove the existing offline `state.db` safety backup.

## Product boundary

Ikaros is a local, general-purpose Agent kernel. It is not a coding-agent
runtime hidden behind an Electron UI. Coding is one workload among ordinary
conversation, file processing, research, automation, and Skill-defined work.

The first implementation is intentionally much smaller than that eventual
product boundary. Its purpose is to prove one real, stateful Agent loop without
hard-coding the kernel around a single UI or a coding-only tool set.

The design synthesis is:

> Use Hermes's long-lived process shape, Pi's small Agent loop, and Codex App
> Server's client protocol boundary to build an Ikaros-owned general Agent
> kernel.

Reference snapshots used during design:

- Pi's loop snapshot at `00eb2d1515e18e00940139f5e6568230a071f33c` informed
  the original design. Its current local harness snapshot at
  `46bb9a2c3bdb296b0d2179f7309ec6b79a7f3106` was also audited for the
  built-in Tool behavior below: small loop, typed events, provider/context
  separation, mutation queues, steer/follow-up, and an explicit settled
  terminal state.
- Hermes Agent at `c0106e50e7ecedb3ce34e785d949725dc4e0e457`: an Electron-supervised,
  long-lived Python backend with multiple sessions, SQLite recovery, providers,
  tools, memory, and Skills.
- Codex App Server: initialization handshake, Thread/Turn/Item concepts,
  quickly acknowledged commands, asynchronous notifications, steer, and
  interrupt.
- OpenCode at `38e10eb1408feb700021b8e8766fb0ab41bf84e2`: separate Provider,
  Model, transport, and public projections; reuse one OpenAI-compatible
  transport for configured providers; normalize streaming and provider errors.
- Demiurge Agent at `222533ed1abb0c56f4099472b78c50ff1dab6091`:
  separate provider profiles from protocol transports and express DeepSeek as
  an OpenAI-chat profile rather than a separate implementation.

Pi harness's `read`, `write`, `edit`, and `bash` Tool primitives are useful
foundational system capabilities. They are not the boundary of a
general-purpose Agent.

## Process topology and lifetime

```text
Electron renderer (presentation only)
        |
        v
typed preload API
        |
        v
Electron main: RuntimeHost / process supervisor
        |
        v
loopback WebSocket JSON-RPC
        |
        v
long-lived Python ikaros-runtime
        |- Scheduler and per-Branch queues
        |- TurnDriver / AgentLoop
        |- ModelInputPlanner
        |- ContextBuilder
        |- ProviderRegistry
        |- ToolRegistry and ToolExecutor
        |- ExecutionPolicy
        `- SQLite event journal and projections
```

The Electron main process starts the runtime once for the application lifetime:

```text
python -m ikaros_runtime serve \
  --host 127.0.0.1 \
  --port 0 \
  --token <random-per-launch-token> \
  --parent-pid <electron-pid>
```

The runtime binds only to loopback. Standard output is reserved for a single
machine-readable readiness record containing the selected port; standard error
is used for logs. Business traffic never uses stdio.

For the first version, closing Ikaros stops the runtime cleanly. The runtime is
not yet a Windows Service, login item, or independently discoverable daemon.
Changing pages, changing Threads, or closing a renderer must not restart the
runtime or implicitly cancel an active Run. SQLite is the canonical recovery
source after the application is started again.

Desktop and a future CLI are clients of the same versioned, language-neutral
protocol. The renderer continues to depend on `AgentClient`; transport and
process supervision stay outside React.

### Versioned protocol contract

`protocol/spec.py` is the sole hand-maintained registry for protocol version,
RPC method names, persisted Journal Event discriminators, initialization
capabilities, and provider-facing Tool IDs. It deterministically generates the
committed JSON manifest and Desktop literal unions. Python and Desktop consume
the same `protocol/golden-trace.json`; Desktop runs it through the production
method/Event parsers, including scope-mirror validation for `turnId`, `runId`,
and `itemId`. Unknown method/Event names or invalid discriminated payloads fail
at the wire boundary and cannot silently advance the ordered event cursor.

The provider Tool ID is `process_run`. The dotted `process.run` spelling is a
presentation label and ScriptedProvider command syntax, not a machine protocol
identifier.

## Runtime home and configuration

All Runtime-owned local files live below the current user's Ikaros home:

```text
~/.ikaros/
  config.yaml          provider/model configuration and Skill enablement
  state.db             canonical SQLite journal and projections
  memory.db            independent long-term Memory records and revisions
  runtime.lock         process-lifetime exclusive ownership of this home
  backups/             verified offline state snapshots, created on demand
  skills/              user-installed <name>/SKILL.md directories
  logs/                created only if persistent file logging is enabled
```

A missing `skills/` directory is an empty Skill catalog. `memory.db` uses its
own schema version 1, connection, transactions, revision records, and operation
receipts. It shares only the Runtime-home process lock with `state.db`; it uses
no SQLite `ATTACH`, cross-database foreign key, or two-phase commit. Startup
requires both schema version 1 and the exact canonical table/index DDL; a
same-version structural drift is rejected rather than silently accepted.

During pre-release development, `state.db` uses an explicit reset-only schema
policy. An empty database is created atomically at canonical database schema
version 8. A non-empty unversioned database or any different `user_version`
fails startup with `reset required`; the Runtime never migrates or silently
deletes it. A developer may explicitly remove `state.db` and its WAL/SHM files
only after the owning Runtime has stopped. `config.yaml`, `skills/`, Desktop
preferences, and the separately owned `memory.db` are independent and
are not removed by a conversation-state reset. Every incompatible persistence
or Event-payload change during this pre-release phase uses this destructive
reset policy rather than a migration or upcaster. Durable release migrations
remain a future compatibility commitment rather than a partial framework in
V1.

Offline maintenance uses the same `runtime.lock` as the server and never starts
the Runtime application or loads `config.yaml`. `storage check` and `storage
backup` open an existing database without initializing or changing it;
`storage repair-projections` opens it read-write only after exclusive ownership
is established. A missing or incompatible `state.db` is reported rather than
created by a maintenance command.

Backup uses SQLite's Backup API so committed WAL frames are included in one
consistent standalone database. A repair always publishes a verified
pre-repair backup first, then rebuilds only disposable projections in one
transaction. Projection foreign-key damage is allowed in that recovery
snapshot, while SQLite page integrity and a strict, contiguous Journal remain
mandatory. Repair must leave every Journal row and the Event sequence
high-water mark unchanged, and its result must pass page, foreign-key, and
replay checks. Backup destinations are never overwritten.

This maintenance surface is only a safety foundation for future compaction.
V1 does not add a checkpoint/base sequence, a compacted Event, Journal row
deletion, or a second conversation source of truth.

On Windows, `~/.ikaros` resolves below the user's profile directory in the same
way as `~/.codex`. V1 does not create a separate credential store or encrypted
secret file. API keys and optional custom-header values are stored as plain
YAML values in `config.yaml`. This is an explicit simplicity trade-off: the
runtime must never print the full configuration, include it in diagnostics, or
copy those values into `state.db`, events, exceptions, or logs, but it does not
claim encryption at rest.

Exactly one Runtime process may own a Runtime home. It acquires `runtime.lock`
before reading configuration, opening SQLite, or running crash recovery and
holds the lock until all server handlers, Agent work, and database handles have
closed. A rapidly restarted Desktop may wait briefly for the previous Runtime
to release the lock; a second live owner never emits readiness and never
mutates the journal. The lock file is stable and is not unlinked on release, so
POSIX processes cannot accidentally lock different inodes. Operating-system
handle cleanup makes the lock recoverable after a crash.

V1 keeps the configuration surface intentionally small. `config.yaml` contains
only a schema version, provider/model records that the user has actually added,
and an optional `skills.disabled` list. There is no global default provider or
model. Host, ephemeral port, launch token, and parent PID are launch arguments;
global serial execution and `FullAccessPolicy` are fixed V1 behavior, not
configuration switches. UI theme, language, layout, and username remain in
Desktop-owned preferences.

When no configuration has been saved, a missing `config.yaml` is equivalent to
an empty provider map with no disabled Skills. The file is created on the first
real Provider/model configuration write or the first persisted Skill-disable
operation; the Runtime does not prepopulate DeepSeek, Custom providers, model
rows, or Skill names.

`config.yaml` is capped at 256 KiB on both read and write. A mutation is
serialized and size-checked before its temporary file is atomically replaced;
any validation, serialization, size, or replace failure leaves both the prior
file and the in-memory configuration unchanged.

Conceptually:

```yaml
version: 1

providers:
  deepseek:
    type: openai_compatible
    preset: deepseek
    base_url: <deepseek-default>
    api_key: <plain-text-key>
    models:
      <model-id>:
        display_name: <model-label>
        enabled: true
        supports_tools: true

  myprovider:
    type: openai_compatible
    display_name: My Provider
    base_url: https://api.example.com/v1
    api_key: <optional-plain-text-key>
    models:
      <model-id>:
        display_name: <model-label>
        enabled: true
        supports_tools: true
    headers: {}          # optional, normally omitted

skills:
  disabled:
    - <skill-name>
```

Model records are explicit configuration and are never synthesized from
frontend fixture data. For the built-in DeepSeek profile, Desktop may call
`provider.discover_models` with a user-entered API key. The Runtime validates a
temporary, non-persistent DeepSeek transport configuration, requests the
upstream `/models` endpoint, and returns a bounded list of model IDs/display
names without persisting or projecting the credential. The user chooses rows
from that result before `provider.configure` writes the Provider and models.
Custom Provider models remain user-entered in V1; model discovery is not a
generic Provider operation.

The empty configuration contains no configured Provider or model rows; the
built-in DeepSeek summary remains visible as an unconfigured preset. A Custom
Provider's `headers` field is a simple optional string map. It has no separate
secret type, persistence subsystem, merge UI, or lifecycle; when the map is
absent or empty, it has no effect on request construction.

The client always sends the selected `provider_id/model_id` with `turn.start`.
The Desktop may visually preselect the sole available model, but the protocol
still carries the explicit reference. With zero configured models, submission
is unavailable and the Runtime returns `model_not_configured` if called. A
missing or unknown reference returns `model_selection_required`. The Runtime
never infers a model from map order or persists a hidden global default.

## Authority boundary

The implemented vertical slice makes the Runtime authoritative for:

- Threads, each Thread's default Branch, Turns, Runs, Items, and their ordering;
- Agent-loop and provider execution;
- `process_run`, `read`, `write`, and `edit` validation, serial execution,
  cancellation, and normalized results;
- the fixed `FullAccessPolicy` execution-policy snapshot;
- Provider/model configuration and selection;
- safe Skill discovery and diagnostics, global enablement, and immutable
  per-Run descriptor snapshots; and
- canonical persisted Agent state.

Additional Branch operations and future Tool families must also be
Runtime-owned when they are implemented. They are not capabilities of the
current vertical slice.

Clients own presentation-only state such as theme, UI language, transient
selection, layout, and local username display. A client may project runtime
records into convenient UI shapes, but those shapes are not the wire protocol.

## Domain model

```text
Thread                   one conversation, optionally carrying workspace metadata
                         and nullable archivedAt lifecycle state
  -> Branch              one immutable conversation path; V1 creates the default only
    -> Turn              one user request and all work caused by it
      -> Run             one execution attempt
        -> Item          message or tool call/result record in the current slice
```

`Project` is a Desktop presentation concept, not a Runtime resource or API.
Desktop deduplicates the optional `Thread.workspace` summaries into Project
groups. Project and ordinary chats use the same Thread/Turn/Run path; a
workspace root supplies the default `cwd` for `process_run` when the Tool Call
does not provide one and resolves relative `read`, `write`, and `edit` paths.
Branch fork, retry/recovery links, Artifact records, and additional Item kinds
remain compatible with this hierarchy but are not yet exposed by the first
Runtime slice.

Inside the Agent loop, a Run contains zero or more Steps. A Step is one model
request plus the tool calls and results needed before the next model request.
`Step` is an internal orchestration concept and does not replace the user's
Turn in the public model.

### Session history and durable Memory

V1 has no separate Session entity; `Thread` is the product conversation
boundary. `state.db` is the canonical record of what happened in a Thread,
including its Branches, Turns, Runs, Items, Provider-Step outcomes, usage,
sequenced Events, and the four persisted audit records: Submission Frame, Run
Manifest, Context Snapshot, and per-Step Manifest.

Provider context is a bounded projection of that complete Session history, not
the history itself. `HistorySelectorV1` walks prior Turns newest-first in pages
of 32, keeps only a contiguous suffix of complete Turn groups, freezes that
selection for the Run, and records the first omitted Turn as a boundary. The
UI continues to page the complete Thread history independently. Later Tool
Steps reload frozen Items by ID and append only current-Run Items.

Durable cross-Thread Memory has an explicit-management and recall foundation: active
records can be created and corrected idempotently, forgotten through a
content-redacting tombstone revision, and listed by scope/kind/state with
keyset pagination, while full current content is loaded by ID. An optional
Session Item source is verified in `state.db`; only its Item ID is accepted
from clients, and unavailable or reset Session state degrades the soft
provenance link without deleting Memory. It remains a different
authority and lifecycle from Session history, History selection, History
compaction, Identity Core, and Skills. `memory.db` is independent from Session
reset and is not a second conversation truth source. The real Settings page
manages records, while `MemoryRetrieverV1` selects bounded Global/current-
workspace records and freezes exact revisions for each Run. Memory bodies are
materialized outside `state.db` transactions and lowered through one canonical,
Runtime-owned contextual-data wrapper; only IDs, revisions, scope and character
accounting enter Session audit records. Content-integrity digests remain private
to `memory.db` and are cleared by Forget. The
detailed boundary and serial implementation Gates are defined in
[MODEL_INPUT_AND_MEMORY_DESIGN.md](MODEL_INPUT_AND_MEMORY_DESIGN.md).

An Event is not another conversation node. It describes a state transition of
a Run or Item. Every wire event carries a monotonically increasing `seq` so a
client can resume from a cursor without guessing what it missed. Every
persisted and wire Event also carries `schemaVersion`. The current Event schema
is version 5. Readers require that exact version and reject unknown versions;
there is no payload upcaster while the database itself follows the explicit
development reset policy above.

The complete persisted Event vocabulary is:

```text
thread.created
thread.renamed
thread.archived
thread.unarchived
run.state_changed
item.started
item.delta
item.completed
model.input_prepared
model.response_finished
run.settled
```

V1 represents Tool lifecycle records as typed Items instead of maintaining a
second Tool-only event hierarchy. A Tool Call is written as a running
`tool_call` Item before the side effect starts. Its terminal snapshot and the
matching `tool_result` Item are committed together, then published in journal
sequence. V1 does not emit `tool.progress`; process output is delivered once in
the bounded terminal result.

Names remain subject to a dedicated protocol specification. Their semantic
distinctions are already locked: commands are acknowledged quickly, execution
continues asynchronously, and each Run reaches exactly one settled terminal
state such as completed, failed, or cancelled.

### Thread Catalog pagination

`thread.list` is the Runtime-owned query surface for the sidebar catalogs. It
accepts optional `cursor`, `limit`, and `archived` fields. `archived` defaults
to `false`, so ordinary calls read only active Threads; `archived: true` reads
only archived Threads. The default page size is 50, the maximum is 100, and the
response is always:

```text
threads
nextCursor
hasMore
snapshotSeq
```

Rows inside each active/archived scope are ordered by `updated_at DESC, id ASC`
using the matching SQLite index.
The opaque URL-safe cursor is a canonical, versioned encoding of the final
returned row's `(updatedAt, id)` key and the archived scope. A cursor from the
other scope is invalid. The next page uses the equivalent
range-seek predicate `updated_at <= cursor.updatedAt AND
(updated_at < cursor.updatedAt OR id > cursor.id)` and reads `limit + 1` rows,
so equal timestamps neither duplicate
nor omit stable rows. Malformed, non-canonical, future-version, or out-of-bound
cursors and limits are JSON-RPC `-32602` errors.

Each page and its `snapshotSeq` are read in one SQLite read transaction.
`snapshotSeq` is the canonical Journal waterline observed for that page, so a
client can align later live/replayed Events with that page. It is deliberately
not a cross-request MVCC snapshot: another page may return a larger waterline
when a Run is active or a Thread changes between requests. Clients must not
require equality across pages or claim that one cursor freezes the whole
catalog.

### Thread lifecycle

The canonical Thread summary always carries `archivedAt`, which is `null` for
an active Thread and the archive timestamp for an archived Thread. Lifecycle
mutations are explicit Runtime commands:

```text
thread.rename({ threadId, title })
thread.archive({ threadId })
thread.unarchive({ threadId })
  -> { thread, changed, event }
```

Each real state change updates the Thread projection and appends exactly one
`thread.renamed`, `thread.archived`, or `thread.unarchived` Event in the same
SQLite transaction. Repeating the current title or archive state returns
`changed: false` and `event: null` without appending a duplicate Event.

Archiving is rejected while any Run owned by the Thread is `queued` or
`running`, preventing active work from disappearing from the Desktop catalog.
An archived Thread remains readable through `thread.get` and `turn.list`, but
cannot accept a new Turn until it is unarchived. Projection rebuild applies the
same lifecycle Events, so the active and archived catalogs are derivable from
the Journal.

### Thread metadata and history reads

The detail read surface is intentionally separate from the catalog:

```text
thread.get({ threadId })
  -> { thread, snapshotSeq }

turn.list({ threadId, branchId, cursor?, limit? })
  -> { turns, nextCursor, hasMore, snapshotSeq }
```

`thread.get` returns the canonical Thread projection, including its default
Branch and optional Workspace. `turn.list` requires an explicit Branch so a
future multi-Branch Thread cannot silently mix histories. Both methods are pure
projection reads: they do not append Events, update recency, or rebuild history
from Journal payloads.

The first `turn.list` page selects the newest Turns. Its wire array is then
returned in chronological `ordinal ASC` order, allowing the Desktop to prepend
older pages without reversing individual Items. The default page size is 50 and
the maximum is 100. The canonical URL-safe cursor is versioned, bound to the
requested Thread and Branch, and carries the oldest immutable Branch ordinal in
the page. A later page seeks to `ordinal < cursor.ordinal`; a cursor from another
Thread or Branch is invalid.

Pagination selects Turn rows before hydrating child records. Every Run for each
selected Turn is returned, and every Run contains its complete materialized
Item sequence, including messages, Tool Calls, Tool Results, partial/failed
states, and structured `data`. This deliberately does not use provider-context
queries, which filter records for a different purpose. Turn selection, Run and
Item hydration, and `snapshotSeq` all share one SQLite read transaction. The
`runs_turn_history_idx` index keeps child hydration proportional to the selected
page rather than all historical Runs.

### ACK, cancellation, recovery, and replay semantics

`turn.start` commits the user Item and a `queued` Run before constructing its
response. The Scheduler synchronously reserves that persisted Run and places
the reservation in its execution queue at that point, but cannot execute it
yet. A worker that reaches an unacknowledged reservation waits there, so a
later Turn whose ACK completes first cannot overtake the earlier persisted
Turn or build context before its assistant Item exists. The server first
attempts to send the JSON-RPC response, then publishes the committed initial
events and activates the reservation. The post-ACK work runs from `finally`,
so a socket failure while sending the ACK does not strand an accepted Turn. A
reservation is visible to cancellation and clean shutdown; either operation
terminalizes and wakes the queue slot. This closes both the cross-client cancel
window and reverse-ACK execution reordering without introducing a deadlock.

Desktop assigns separate stable `clientRequestId` values to `thread.create`
and `turn.start`. SQLite stores them behind unique indexes, so retrying either
command after a transport failure returns the original Thread or Turn/Run
instead of creating a duplicate. The matching ID is also written into the
canonical `thread.created` or first user `item.completed` event. A client first
replays every page after its pre-command `seq`: a matching canonical event
proves acceptance, while an explicit JSON-RPC error for that command proves
rejection. A successful replay with no match does not prove rejection because
a command accepted on another connection may still be in flight and not yet
visible to the replay. After a timeout, socket close, or send failure, the same
idempotent command therefore keeps retrying with capped backoff even when replay
is reachable; the submission remains queued and its draft is not restored while
acceptance is ambiguous. Live and replay delivery may race, but the request ID
and client-side continuation gate ensure that a newly created Thread starts one
logical Turn. Only a confirmed rejection restores the draft and permits a new
submission.

`run.cancel` follows the same ACK-first boundary. Runtime validation and the
current Run status determine the response, while cooperative cancellation and
terminal event publication happen after the response attempt. In particular,
`accepted: true` means that the cancellation request was accepted for a Run
that was non-terminal at that observation point. It is not a promise that the
canonical terminal status will be `cancelled`: the provider may naturally
complete and commit first while the ACK is in flight. Completion is allowed to
win that race. Clients must use the single durable `run.settled` event as the
outcome and treat repeated cancellation of an already terminal Run as
`accepted: false`.

Terminalization updates every active message or Tool Item, the Turn, and the
Run and appends `run.settled` in one SQLite transaction. Partial assistant text
or a running Tool therefore cannot appear complete when its Run is cancelled
or fails, and a partial terminal state cannot survive a failed transaction. A
partial unique index on the journal enforces at most one `run.settled` per Run;
terminalization is also idempotent so completion, cancellation, shutdown, and
recovery can safely race.

On clean shutdown the Scheduler stops accepting work, cooperatively cancels the
active Run, terminalizes queued and reserved Runs as `cancelled`, and waits for
the worker before SQLite closes. On startup, a Run left `running` by an
unclean process exit becomes `failed` with reason code `runtime_interrupted`,
preserving any partial assistant text. Runs left `queued` are scheduled in
their original journal sequence. Terminal Runs are never recovered or rerun.

Every journal event has one global monotonically increasing `seq`. The Runtime
buffers concurrently published events until it can notify subscribers in a
contiguous sequence. Live delivery remains an optimization: after connection,
reconnection, or any observed gap, a client calls `event.replay` from its last
contiguous cursor and follows `nextAfterSeq` until `hasMore` is false. Duplicate
sequences are ignored, and a client must not advance its projection cursor
across a gap. Consequently a dropped notification or failed replay attempt
cannot authorize the client to skip canonical history; replay is retried from
the same cursor.

Offline replay is deliberately stricter than UI decoding. Every Event must use
the current schema version and exact payload shape; required nullable fields
must still be present. Event-envelope IDs, nested record IDs, immutable fields,
state transitions, and affected projection rows must agree. Unknown Event
types, missing or extra fields, malformed values, sequence gaps, and a deleted
tail that disagrees with SQLite's Event high-water mark all fail check/repair.
There is no legacy fallback or default-field synthesis during projection
rebuild.

The first safe text delta in a provider Step is persisted and published
immediately. Later safe deltas are coalesced until 256 characters accumulate or
the 50 ms window is observed by an arriving chunk; pending text is synchronously
flushed at provider completion, Tool Call boundaries, cancellation, and provider
failure. There is no background journal writer, so commit and publication stay
ordered. Completed Items, terminal Run state, branch/fork decisions, retry links,
and other semantic records are appended canonically. Projections are rebuildable
from the SQLite journal; existing history is not rewritten when a Branch, retry,
or future compaction record is added. V1 does not yet define or emit such a
compaction record.

## Scheduling and Agent loop

The first version has a global execution concurrency of one. Each Branch still
has a serial queue behind a global Scheduler so later concurrency is a capacity
change, not a protocol rewrite. Tool calls are also executed serially in the
first slice.

```text
client submits user input
  -> runtime atomically appends Turn, Run, SubmissionFrameV1, and RunManifestV1
  -> Scheduler reserves the persisted Run
  -> server attempts the command ACK
  -> Scheduler activates the Run
  -> MemoryRetrieverV1 reads Global/current-workspace candidates outside the
     state.db transaction and freezes bounded exact-revision metadata
  -> first prepare_model_step pages backward and freezes bounded ContextSnapshotV1;
     later Steps load frozen Item IDs plus current-Run Items
  -> Runtime persists StepManifestV1 and emits model.input_prepared
  -> Runtime re-materializes the exact frozen Memory revisions; Forget aborts
     before the Provider sees a new Step
  -> ModelInputPlanner creates a structurally immutable ModelInputPlanV1 from
     the frozen Submission Frame, versioned Output Style, frozen IKAROS.md
     Identity Core, frozen Skill Catalog, bounded Memory Context Data, frozen
     ContextItems, and separate Tool definitions
  -> ContextBuilder deterministically renders that Plan into ProviderRequest
  -> provider streams assistant output or requests a tool
  -> ToolRegistry resolves and validates the call
  -> ExecutionPolicy returns allow under FullAccessPolicy
  -> ToolExecutor runs the registered Tool and emits lifecycle events
  -> normalized ToolResult is appended and returned to the model
  -> Runtime emits exactly one model.response_finished for the prepared Step;
     any Provider-reported usage is projected only from this Event
  -> the model may continue another Step
  -> final assistant Item is completed
  -> Run emits exactly one settled terminal event
```

`ModelInputPlanner` consumes the frozen Submission Frame, while
`ContextSnapshotV1`, `RunManifestV1`, and per-Step `StepManifestV1` provide the
durable audit boundary. Selection uses `bounded-history-v1`: a 48,000 Unicode
character total limit, a 12,000-character first-Step current-Run reserve,
complete-Turn atomic selection, and deterministic omission metadata. Later
Steps may use capacity beyond that reserve, but actual total input may never
exceed 48,000 characters. The Runtime loads its read-only `IKAROS.md` resource
with `importlib.resources`, freezes the version 1 `ikaros-identity` Instruction
Block into each Submission Frame at `turn.start`, and reuses that frozen input
for every Step in the Run. Recovery validates the frozen block against the
current release Identity and fails on drift rather than silently substituting
new content. Memory retrieval remains a separate low-authority input: its body
is absent from Session audit records, exact references are frozen in the Context
Snapshot, and the ContextBuilder accepts only the canonical Runtime wrapper.
Detailed limits and failure semantics are recorded in
[MODEL_INPUT_AND_MEMORY_DESIGN.md](MODEL_INPUT_AND_MEMORY_DESIGN.md).

Gate 4 requires no `state.db` schema reset. Completed pre-Gate-4 history whose
frozen `identityCore` is null remains readable and replayable. It does not make
unfinished legacy execution compatible: a queued null-Identity Run fails with
`identity_core_changed` before any Provider call, while startup recovery settles
an already-running Run as `runtime_interrupted`. The Runtime neither substitutes
the current release Identity into those Runs nor provides a compatibility
fallback.

The provider boundary must not leak provider-specific request or streaming
formats into the domain or protocol.

## Provider architecture

V1 implements one real protocol adapter, not one implementation per configured
provider:

```text
ProviderRegistry
  |- ScriptedProvider                    deterministic tests
  `- OpenAICompatibleAdapter             real streaming transport
       |- built-in DeepSeek profile
       `- user-defined Custom profiles
```

DeepSeek is the only built-in vendor profile in V1. A Custom provider means an
OpenAI Chat Completions-compatible endpoint; it is not an arbitrary protocol or
dynamically loaded provider plugin. DeepSeek and Custom profiles share message
conversion, streaming, Tool Call assembly, timeouts, cancellation, retry rules,
and error normalization.

The internal adapter contract is provider-neutral and streaming-first:

```python
class ProviderAdapter(Protocol):
    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]: ...
```

`ProviderRequest` contains the selected provider/model reference, normalized
messages, Tool definitions, and model settings. It does not contain UI state or
DeepSeek request objects. Adapter output is a fixed internal event vocabulary,
including at least:

```text
response.started
text.delta
reasoning.delta                 optional
tool_call.arguments_delta
tool_call.completed
usage.updated                   optional when the endpoint reports it
response.completed
```

Streaming Tool Call fragments are accumulated by call index/ID. Arguments are
parsed only after the call is complete, then validated against the registered
Tool schema. Invalid or incomplete JSON becomes a structured protocol error and
must never reach ToolExecutor.

An OpenAI-compatible response may contain assistant narration followed by Tool
Calls in the same provider Step. The Agent loop persists both under one Step and
rebuilds them as one provider message containing `content` plus `tool_calls` on
the next request or after SQLite recovery. Text emitted after Tool Calls remains
an invalid event order. This mixed narration/Tool-Call behavior is covered by
offline Agent, storage-replay, and adapter regression tests; the live DeepSeek
smoke described below proves the file-Tool chain, not that particular ordering.

Both JSON-RPC and upstream SSE use strict JSON: non-standard constants,
non-finite numbers (including exponent overflow), integers outside JavaScript's
safe range `[-(2^53-1), 2^53-1]`, and excessively deep nesting are rejected
before values cross a protocol boundary. Outgoing wire and journal serialization
enforces the same numeric rules instead of emitting values that JavaScript would
turn into `NaN`, `Infinity`, or an imprecise integer.

### Provider and model records

Secret-bearing configuration and client-visible projections are distinct
types even though the former is stored plainly in `~/.ikaros/config.yaml`. The
conceptual configuration is:

```text
ProviderConfig
  id
  display_name
  adapter_kind = openai_compatible
  origin = builtin | custom
  base_url
  api_key?
  headers?

ModelConfig
  provider_id
  id                         stable ID and value sent to the upstream API in V1
  display_name
  enabled
  supports_tools
  reasoning_field?           for compatible endpoints such as reasoning_content
```

The built-in DeepSeek profile fixes its provider ID, display name, adapter kind,
and default Base URL. The user supplies its credential and explicit model
records. A Custom profile uses the existing Desktop fields: provider ID,
display name, Base URL, optional API key, optional custom headers, and one or
more user-entered model IDs/display names. An empty API key emits no
`Authorization` header so local unauthenticated compatible endpoints remain
usable.

V1 does not need OpenCode's online model database, dynamic SDK downloads,
provider plugins, OAuth, prices, model variants, or broad vendor-specific
capability inference. Its only discovery operation is the explicit DeepSeek
`provider.discover_models` request described above; Custom Provider models are
entered manually. Model capabilities are not guessed from name substrings.
Tool support is declared by the configured model/profile data, while compatible
`reasoning_content` is normalized and replayed by the adapter when present.

The public projection never includes API keys or custom header values:

```text
ProviderSummary
  id
  display_name
  origin
  configured
  credential_configured
  health = unknown | ready | error

ModelSummary
  provider_id
  id
  display_name
  enabled
```

Runtime operations expose Provider configuration separately from the model
catalog through `provider.list`, `provider.configure`,
`provider.discover_models`, `provider.disconnect`, `provider.remove`,
`model.list`, and `model.set_enabled`. `configured` means valid configuration
exists. `health` remains `unknown` in the current slice; a durable health-test
workflow is not implemented. No frontend Boolean is authoritative for Provider
state.

V1 has no disabled Provider state. Removing a Custom Provider deletes its full
entry from `config.yaml`, including its API key, headers, and models. The
operation is rejected while any Run is queued or running; otherwise it does not
need a replacement-default workflow because no global default exists. This
V1-wide mutation barrier keeps the protected-value set stable across Provider
streaming and Tool execution.
The Desktop action should therefore be labelled Remove rather than Disconnect.
The built-in DeepSeek preset itself cannot be removed. Disconnecting DeepSeek
deletes its saved configuration, including the API key and model records, while
the unconfigured built-in summary remains available for later reconfiguration.

API keys and custom header values may cross the authenticated loopback channel
only in a write-only configuration command. They are written to
`~/.ikaros/config.yaml`, but never enter the SQLite event journal, conversation
events, logs, list responses, exceptions, or UI projections. Responses expose
only redacted configuration state. Header names and values reject CR/LF, and
header merging is case-insensitive. No dedicated header-secret system is part
of V1.

To preserve that invariant, configuration rejects an exact credential value
that appears inside a public Provider/model field or the existing event
journal. Once configured, Thread and Turn write commands containing an exact
credential value are also rejected before an event is created.
Provider updates validate public fields against the union of credentials before
and after the mutation, so rotating or removing a credential cannot promote the
old value into a later public projection.

Credential protection follows data provenance rather than an impossible
alphabet-wide byte-ban. Immutable JSON-RPC schema, protocol constants, enum
values, and generated identifiers are not configuration values: for example, a
custom Header equal to `a`, `2.0`, or `full_access` does not make the same fixed
protocol text a secret disclosure. Client-controlled RPC IDs reject an exact
short credential and also reject a credential of eight or more characters when
embedded in a larger value. Method names are fixed dispatch selectors and are
never echoed or logged. Dynamic public Provider/model fields, Thread/Turn
inputs, and existing journal values are checked before commit. Streaming model
text, reasoning, Tool arguments, and Tool Results keep strict cross-delta
substring detection for every configured credential value.

This is a boundary on Runtime-managed transport, output, and persistence, not a
sandbox or data-loss-prevention claim. Under `Full access`, a command or Skill
script can independently read, transform, or transmit any data available to the
current operating-system user; exact-value filtering of Tool Results does not
constrain what the child process itself can do.

The runtime caches HTTP clients per effective provider configuration and closes
them on replacement or shutdown. Connect, response-header, and stream-idle
timeouts are distinct. A request may be retried only before any content or Tool
Call delta has been emitted; transparent retry after streaming begins would
duplicate output. Provider failures normalize to stable categories such as
authentication, rate limit, context overflow, invalid request, timeout,
network, server, cancelled, protocol, and unknown, with safe optional status,
request ID, and retryability metadata.

Cancellation also owns transport cleanup. If response headers and cancellation
become ready in the same event-loop turn, the completed response is explicitly
closed before cancellation wins; a leased connection is never abandoned.

A deterministic `ScriptedProvider` remains separate from this adapter and is
used for Agent-loop and event-ordering tests without credentials or network
access.

## Built-in tools and Skills

The current ToolRegistry contains four provider-facing Tools: `process_run`,
`read`, `write`, and `edit`. Command and file operations share Tool definition,
policy, cancellation, lifecycle, persistence, and result-normalization paths;
they are not separate Agent loops.

The first registered command Tool has the provider-facing function name
`process_run`. OpenAI-compatible endpoints restrict function names to letters,
digits, underscores, and hyphens, so the dot-separated product label
`process.run` is not sent upstream. Desktop displays `process.run`, and the
deterministic provider exposes `/process.run <command>` as its explicit test
syntax. The adapter and ToolRegistry keep the mapping at the provider boundary.

`process_run` accepts only `command`, optional `cwd`, and optional `timeoutMs`.
On Windows it creates the shell suspended, assigns it to a kill-on-close Job
Object, and only then resumes it; on POSIX it creates a new session/process
group. The deadline covers root-process exit and both output pipes reaching
EOF, so a root shell cannot evade timeout by exiting while a background child
keeps a pipe open. The executor drains stdout and stderr concurrently, retains
at most 64 KiB from each stream, and distinguishes normal non-zero exit,
timeout, and user cancellation. Completion, timeout, cancellation, and task
teardown all close the supervised tree; cancellation preserves bounded partial
output in a matching Tool Result before the Run settles. The child receives an
explicit allowlist of ordinary OS environment variables rather than Electron's
entire environment, so unrelated launch-time secrets are not inherited.

The file primitives deliberately reuse the stable reference-project behavior
instead of adding fuzzy or model-specific editing:

- `read(filePath, offset?, limit?)` reads UTF-8 text with 1-based line numbers,
  returns at most 2,000 lines and 50 KiB per call, truncates an individual line
  after 2,000 characters, and rejects directories, binary content, and invalid
  UTF-8. It decodes incrementally and stops reading when the requested page or
  byte budget is complete; `nextOffset` lets the model page forward without an
  unnecessary full-file scan. `totalLines` is therefore present only at EOF.
- `write(filePath, content)` creates parent directories and creates or fully
  replaces a text file. Existing BOM, newline style, and file mode are preserved
  where applicable. The replacement writes and `fsync`s a same-directory
  temporary file, verifies those temporary-file bytes against the intended
  payload with SHA-256, and only then publishes it with `os.replace`.
  `verified: true` reports that the temporary bytes matched the intended
  payload before publication and that `os.replace` succeeded; it does not claim
  that the target pathname was hashed again after replacement, and the digest
  itself is not returned to the model.
- `edit(filePath, oldString, newString, replaceAll?)` edits an existing UTF-8
  text file by exact match. The default succeeds only for one match; zero or
  multiple matches leave the file unchanged, while `replaceAll: true` explicitly
  replaces every match. Matching normalizes CRLF and lone CR to LF, then writes
  one consistent style chosen from the file's first newline (LF when no LF was
  present); BOM is preserved. Before replacement the Runtime compares the
  current bytes with the bytes originally read and
  returns `stale_content` if another process changed the file. This is a narrow
  conditional-write check, not a filesystem transaction. V1 intentionally has
  no fuzzy matching, formatter, LSP, or broader FileState subsystem.

All three file Tools serialize access to the same resolved path with a keyed
lock whose entry is reclaimed after the final holder or waiter. Existing
symlink aliases resolve to their target before locking and replacement, so an
alias and direct path share one lock and mutation preserves the symlink itself.
If cancellation arrives during a disk operation, the holder keeps the lock
until that operation settles and then reports cancellation rather than success.
Relative paths resolve against the Thread workspace when one exists and
otherwise against the Runtime working directory. Absolute paths remain allowed
under V1 Full access.
Desktop projects their structured lifecycle and result summaries, but a file
operation does not yet produce a first-class Artifact or file-change/diff Item.

Skills V0 treats a Skill as an instruction and resource bundle that may contain
references, assets, and scripts. The Runtime safely scans one directory level
below `~/.ikaros/skills` for `<name>/SKILL.md`, validates bounded UTF-8 YAML
frontmatter, rejects link-like or escaping paths, and returns both valid
descriptors and bounded diagnostics through `skill.list`.

Each descriptor contains only name, description, and canonical `SKILL.md`
location. `skill.set_enabled` persists a global disabled-name list in
`config.yaml`. At `turn.start`, the Runtime freezes every enabled descriptor
into that Run. Each Provider Step receives this frozen catalog and an
instruction to load a relevant `SKILL.md` through the ordinary `read` Tool.
The Skill body, references, assets, and scripts are not copied into the
descriptor snapshot or eagerly injected.

Skill-owned Python is never imported into the long-lived Runtime and scripts
are not expanded into one model ToolDescriptor per file. A loaded Skill may
tell the Agent to invoke a script through the same general command Tool used
elsewhere:

```text
safe Skill discovery and global enablement
  -> freeze enabled descriptors into the Run
  -> model uses read to load a relevant SKILL.md
  -> Agent requests the general command tool
  -> ToolExecutor starts the Skill script as a child process
  -> normalize stdout, stderr, exit status, and errors as ToolResult
  -> append the result and return it to the Agent loop
```

Skill scripts therefore share the implemented command executor's argument
validation, working-directory and environment handling, timeouts, cancellation,
process-tree cleanup, output truncation, lifecycle events, and error
normalization.

Skills V0 does not perform task-specific Skill selection, enforce a total
catalog budget, automatically load full instructions or resources, create a
dedicated Skill Tool/Item lifecycle, or attribute and aggregate script
executions. Those are later refinements; Skill execution attribution would not
create a separate executor.

## Execution policy

The first version exposes `Full access` and does not implement an interactive
permission system:

```text
ExecutionPolicy = FullAccessPolicy
  -> registered tools execute without a permission prompt
  -> no permission.requested event is emitted
  -> no approval rules are persisted
```

The runtime keeps a thin `ExecutionPolicy` seam and records the policy selected
for a Run. A future deterministic rule engine, optional review Agent, or manual
approval flow can replace `FullAccessPolicy` without rewriting ToolExecutor.
Timeouts, cancellation, output limits, child-process cleanup, and minimal
environment construction are execution-reliability requirements even under
Full access; they are not deferred as part of the permission UI.

Full access means `process_run`, `read`, `write`, and `edit` can exercise the
current operating-system user's authority. Skill scripts invoked through
`process_run` exercise that same authority. This is an explicit
development-version trade-off, not a sandbox or security guarantee.

The current reset-only SQLite database schema is canonical version 8. Thread
projections include optional `workspace_json`, nullable `archived_at`, and an
indexed active/archived Thread Catalog ordering key; Run history hydration is
indexed by `turn_id`. Each Run snapshots `execution_policy = full_access` and
its enabled Skill descriptors, while each Item has structured `data_json` for
Tool Call arguments and normalized results. Rebuilding projections from the
journal restores these records and the provider context. `run_inputs` stores
one Submission Frame, Run Manifest, and nullable frozen Context Snapshot per
Run. `model_steps` stores one Step Manifest plus exactly one optional terminal
outcome record per prepared Step. Both tables are rebuilt from the append-only
Journal. A bounded Agent loop
persists all calls from a provider Step before serial execution, returns every
result under the original provider call ID, and stops a provider that exceeds
the maximum Step count.

## Desktop projection

The current renderer's `AgentEvent` type remains a UI projection rather than
being frozen as the wire schema. Current mappings and explicit gaps are:

| Desktop concept | Runtime source |
| --- | --- |
| project and ordinary conversation lists | Thread projections; Desktop groups non-null `Thread.workspace` values as Projects |
| rename/archive/unarchive | Runtime Thread lifecycle commands and sequenced lifecycle Events; archived Threads use a separate settings catalog |
| streaming response | message Item lifecycle events |
| Stop | Run cancellation command and terminal event |
| tool card | `process_run`, `read`, `write`, and `edit` Item lifecycle events |
| edited earlier user message | mock-only UI; no Runtime Branch-fork command yet |
| Turn Navigator | projected current Turn/Item records used only for navigation |
| retry or recovery | mock-only UI; no Runtime retry/resume command yet |
| artifacts and file changes | mock-only UI; no Runtime Artifact/file-change Item yet |
| provider/model settings | runtime capability and model catalog |
| Skills settings | `skill.list` / `skill.set_enabled` catalog, diagnostics, and global enablement |
| Memory management and recall | `memory.create` / `memory.correct` / `memory.forget` / `memory.list` / `memory.get`, the real Settings page, and deterministic bounded Runtime recall with frozen exact revisions |
| Profile Token metrics and activity | `usage.read` over the `model_usages` projection rebuilt solely from Provider-reported usage in `model.response_finished`; no text-based estimation |
| theme, language, username | client-only UI state |

The runtime sends stable semantics and original content, never pretranslated
Chinese or English labels. Fixed phrases such as tool status and Run state are
translated by renderer i18n; user and model message content is not translated.

### Current Desktop integration

The production boundary now contains separate wire DTOs, a typed preload/IPC
API, a `RuntimeClient`, and a projection reducer that converts sequenced Runtime
events into renderer records. Electron main supervises and authenticates the
Python process, reconnects the WebSocket, replays gaps, and keeps Runs alive
when renderer navigation changes. Desktop uses stable request IDs for
`thread.create` and `turn.start`, projects Runtime Threads and workspaces,
selects from `model.list`, sends the explicit Provider/model reference with
each Turn, and drives Stop through `run.cancel`.

The Runtime wire method `thread.list` is keyset-paginated and scope-bound.
Electron main traverses either the active or archived pages with a limit of
100, rejects malformed pages,
duplicate/repeating cursors, backward waterlines, and unbounded pagination,
then exposes the aggregate catalog plus the first page's `snapshotSeq` to the
renderer. Increasing page waterlines are valid. The first waterline is retained
as the conservative catch-up baseline; the bridge does not pretend multiple
page requests share an MVCC snapshot. If an existing Thread moves across a
keyset boundary after that baseline and is omitted from the aggregate scan, its
post-baseline event triggers a single-flight `thread.get` that restores the
catalog stub without eagerly loading its history.

Cold startup loads only the active catalog. The archived catalog is fetched on
demand by the General-settings management dialog. Desktop calls
`thread.rename`, `thread.archive`, and `thread.unarchive` through the same typed
preload/IPC/RuntimeClient boundary, projects their sequenced Events into the
correct catalog, clears an archived active selection, and buffers lifecycle
Events that race an archived-catalog read so stale pages cannot resurrect a
Thread in the wrong scope.

Cold Desktop startup subscribes to live events before reading the catalog,
installs that catalog at its known waterline, and performs incremental catch-up
from a non-zero known `snapshotSeq`. It does not rebuild every Thread with
`event.replay(0)`. RuntimeHost likewise initializes its ordered notification
cursor from `thread.list({ limit: 1 })`; only WebSocket reconnect uses
`event.replay`, beginning at the last established sequence.

Selecting a Thread calls typed `thread.get` and paginated `turn.list`, exhausts
the selected history pages, validates their nested Thread/Turn/Run/Item scope,
and installs the materialized UI projection in a per-Thread detail cache.
Selecting the same cached Thread does not fetch it again, and loading one
Thread never cancels a Run in another. A per-Thread, per-Run activity
projection keeps every queued/running Run identity and status independently of
the selected-history cache, so a newer queued Run can settle without hiding an
older Run that is still active.

Detail hydration registers an event buffer before issuing either read. The
`thread.get` metadata and every returned Turn retain their own query
`snapshotSeq`; multiple page requests are never treated as one MVCC snapshot.
An event at or below its Turn page's waterline is not projected onto already
materialized text a second time, while metadata changes after the independent
`thread.get` waterline still update catalog recency. Newer events are
deduplicated by `seq` and applied after the relevant snapshot. All events still
pass through submission, cancellation, activity, and gap-control handling.
This prevents both lost live updates and duplicate streaming deltas even when
later history pages observe a newer Journal tail.

Desktop projects `process_run` as `process.run` and projects `read`, `write`,
and `edit` with file-specific icons, translated fixed labels, and bounded
metadata such as path, line range, byte count, and replacement count. Write and
edit arguments containing file content or replacement text are not copied into
the visible card. These are Tool projections only; Artifact and file-change
fixtures remain mock-only.

Provider and model forms call the Runtime's configuration operations. Secret
fields cross only the write command and are not retained in renderer state;
catalog refresh uses redacted summaries. DeepSeek model discovery uses the
entered API key without first persisting it. Custom Provider removal and
DeepSeek disconnect use their distinct Runtime operations, and either action
removes the corresponding saved model rows.

The renderer's `AgentEvent` and `RunStatus` types remain UI projections rather
than wire schema. `ScenarioId`, seeded conversations, fixed timestamps,
`playScenario`, mock slash commands, permission/recovery
cards, and Artifact/file-change fixtures remain outside the Runtime contract.
The `MockAgentClient` is used only when the typed Desktop Runtime bridge is
absent and by deterministic UI tests; it does not drive the packaged Desktop's
conversation path.

The Runtime-backed Composer fixes execution to `Full access`; its access picker
is disabled rather than authorizing execution locally. Attachments and Tool
selection are unavailable. Editing an earlier message, Branch switching/fork,
retry/resume, regenerate, interactive permission decisions, and first-class
Artifact/file-change production still need dedicated Runtime commands and event
semantics before those UI surfaces can become functional.

## Implemented vertical slice

The first version deliberately implements one narrow but real conversation
path. The following have been demonstrated end to end:

1. Electron `RuntimeHost` starts one authenticated Python Runtime and keeps it
   alive across renderer navigation.
2. Desktop connects through the versioned WebSocket JSON-RPC protocol, restores
   persisted Threads, and replays the sequenced event journal.
3. The same conversation completes at least two sequential user Turns, and the
   later Provider request receives the selected prior completed conversation
   context within the frozen bounded-history budget.
4. The built-in DeepSeek profile and configured Custom OpenAI-compatible
   profiles share one streaming adapter and receive the `process_run`, `read`,
   `write`, and `edit` Tool definitions.
5. The Runtime executes requested Tools serially, captures normalized
   ToolResults, returns them to the model, and the model produces a final
   assistant answer. The most recently recorded opt-in live DeepSeek smoke
   completed
   `write -> read -> edit -> read`, verified the edited bytes on disk, and
   observed the final answer containing the edited token.
6. Run and Item lifecycles settle coherently and are projected by the Desktop;
   Stop propagates through Run and child-process-tree cancellation.
7. The deterministic Provider covers the loop and event ordering without a
   network dependency, while the opt-in live test covers the real DeepSeek path.
8. Skills V0 discovers and configures a real catalog, freezes enabled
   descriptors into each Run, exposes the catalog through Desktop settings, and
   keeps full Skill bodies lazy. The most recently recorded live DeepSeek smoke
   verified that frozen descriptor path alongside the file-Tool chain.
9. Gate 2 persists the four model-input audit objects, enforces Provider/Tool
   drift checks, records actual response model/request IDs safely, and closes
   every prepared Provider Step through `model.response_finished`.
10. Gate 3 bounds model input without changing UI history: it selects complete
    recent Turns through paged reads, preserves Tool Call/Result atomicity,
    reloads later Steps through frozen IDs, and settles pre-Provider failures as
    `context_budget_exceeded` or `model_input_unavailable`.
11. Gate 4 loads the packaged `resources/IKAROS.md` through
    `importlib.resources`, freezes its versioned `runtime_identity` block into
    every Run, and keeps Provider/model identity separate from Ikaros identity.
12. Gate 8 deterministically recalls bounded Global/current-workspace Memory,
    freezes exact revisions, rejects cross-database transaction overlap, and
    stops unsent Steps after Forget without changing Tool definitions or Policy.
13. Gate 9 revalidated the complete Desktop/Runtime path against real DeepSeek,
    including a large Tool Result, bounded history, cross-Thread Global Memory,
    Workspace isolation, Correction, Forget, hostile quoted Memory, body-free
    audit manifests, Tool pairing, and credential containment.

The live validation evidence, including credential containment checks, is
recorded in [LIVE_VALIDATION.md](LIVE_VALIDATION.md).

This slice does not implement web search, browser or desktop control,
automatic Memory extraction, background or scheduled tasks, messaging channels, MCP/connectors,
Subagents, a plugin marketplace, or a complex approval system. Those remain
later general-Agent capability packs, not rejected product directions. The
provider-neutral input plan, Gate 2 audit/freeze foundation, Gate 3 bounded
history selection, Gate 4 Identity Core, and explicitly managed durable Memory
with its Desktop management page and deterministic recall are current. The
completed Gate record and explicitly deferred capabilities are documented in
[MODEL_INPUT_AND_MEMORY_DESIGN.md](MODEL_INPUT_AND_MEMORY_DESIGN.md).
