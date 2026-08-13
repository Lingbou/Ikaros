# Ikaros Runtime Architecture

Status: first vertical-slice decisions locked on 2026-08-11 and implemented on
2026-08-12. This is a living architecture record: sections describing the
current vertical slice reflect the implementation, while explicitly marked
future capabilities remain design direction rather than shipped behavior.

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

## Runtime home and configuration

All Runtime-owned local files live below the current user's Ikaros home:

```text
~/.ikaros/
  config.yaml          provider and model configuration, including API keys
  state.db             canonical SQLite journal and projections
  runtime.lock         process-lifetime exclusive ownership of this home
  skills/              created when user-installed Skills are supported
  logs/                created only if persistent file logging is enabled
```

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
only a schema version and provider/model records that the user has actually
added. There is no global default provider or model. Host, ephemeral port,
launch token, and parent PID are launch arguments; global serial execution and
`FullAccessPolicy` are fixed V1 behavior, not configuration switches. UI theme,
language, layout, and username remain in Desktop-owned preferences.

When no configuration has been saved, a missing `config.yaml` is equivalent to
an empty provider map. The file is created on the first real Provider/model
configuration write; the runtime does not prepopulate DeepSeek, Custom
providers, or model rows.

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
- Provider/model configuration and selection; and
- canonical persisted Agent state.

Additional Branch operations, Skills, and future Tool families must also be
Runtime-owned when they are implemented. They are not capabilities of the
current vertical slice.

Clients own presentation-only state such as theme, UI language, transient
selection, layout, and local username display. A client may project runtime
records into convenient UI shapes, but those shapes are not the wire protocol.

## Domain model

```text
Thread                   one conversation, optionally carrying workspace metadata
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

An Event is not another conversation node. It describes a state transition of
a Run or Item. Every wire event carries a monotonically increasing `seq` so a
client can resume from a cursor without guessing what it missed.

Representative event semantics are:

```text
item.started                    message or tool-call Item entered an active state
item.delta                      bounded message streaming delta
item.completed                  message, tool-call, or tool-result terminal snapshot
run.state_changed
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

Streaming deltas may be buffered or persisted in batches. Completed Items,
terminal Run state, branch/fork decisions, retry links, and other semantic
records are appended canonically. Projections are rebuildable from the SQLite
journal; existing history is not rewritten when a Branch, retry, or compaction
record is added.

## Scheduling and Agent loop

The first version has a global execution concurrency of one. Each Branch still
has a serial queue behind a global Scheduler so later concurrency is a capacity
change, not a protocol rewrite. Tool calls are also executed serially in the
first slice.

```text
client submits user input
  -> runtime appends Turn and Run
  -> Scheduler reserves the persisted Run
  -> server attempts the command ACK
  -> Scheduler activates the Run
  -> ContextBuilder builds provider-neutral context
  -> provider streams assistant output or requests a tool
  -> ToolRegistry resolves and validates the call
  -> ExecutionPolicy returns allow under FullAccessPolicy
  -> ToolExecutor runs the registered Tool and emits lifecycle events
  -> normalized ToolResult is appended and returned to the model
  -> the model may continue another Step
  -> final assistant Item is completed
  -> Run emits exactly one settled terminal event
```

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

A future Skill integration treats a Skill as an instruction and resource bundle
that may contain references, assets, and scripts. Loading a Skill will not
import third-party Python into the long-lived runtime, and scripts will not be
expanded into one model ToolDescriptor per file. Skill discovery, selection,
and instruction injection are not implemented in the first Runtime slice.

When that integration is added, Skill instructions tell the Agent when and how
to invoke a script via the same general process/command tool used elsewhere:

```text
Skill discovery and selection
  -> read and inject SKILL.md instructions
  -> Agent requests the general command tool
  -> ToolExecutor starts the Skill script as a child process
  -> normalize stdout, stderr, exit status, and errors as ToolResult
  -> append the result and return it to the Agent loop
```

Skill scripts will therefore share the implemented command executor's argument
validation, working-directory and environment handling, timeouts, cancellation,
process-tree cleanup, output truncation, lifecycle events, and error
normalization. A future Runtime may attribute a script path back to its Skill
for UI and audit records; that attribution does not create a separate executor.

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
current operating-system user's authority. Once Skill scripts are integrated
through the same executor, they will inherit that authority as well. This is an
explicit development-version trade-off, not a sandbox or security guarantee.

The current SQLite schema version is 6. Thread projections include optional
`workspace_json`. Each Run snapshots
`execution_policy = full_access`, and each Item has structured `data_json` for
Tool Call arguments and normalized results. Rebuilding projections from the
journal restores these records and the provider context. A bounded Agent loop
persists all calls from a provider Step before serial execution, returns every
result under the original provider call ID, and stops a provider that exceeds
the maximum Step count.

## Desktop projection

The current renderer's `AgentEvent` type remains a UI projection rather than
being frozen as the wire schema. Current mappings and explicit gaps are:

| Desktop concept | Runtime source |
| --- | --- |
| project and ordinary conversation lists | Thread projections; Desktop groups non-null `Thread.workspace` values as Projects |
| streaming response | message Item lifecycle events |
| Stop | Run cancellation command and terminal event |
| tool card | `process_run`, `read`, `write`, and `edit` Item lifecycle events |
| edited earlier user message | mock-only UI; no Runtime Branch-fork command yet |
| Turn Navigator | projected current Turn/Item records used only for navigation |
| retry or recovery | mock-only UI; no Runtime retry/resume command yet |
| artifacts and file changes | mock-only UI; no Runtime Artifact/file-change Item yet |
| provider/model settings | runtime capability and model catalog |
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
`playScenario`, mock slash commands, profile statistics, permission/recovery
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
   later Provider request receives the prior completed conversation context.
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

The live validation evidence, including credential containment checks, is
recorded in [LIVE_VALIDATION.md](LIVE_VALIDATION.md).

This slice does not implement web search, browser or desktop control, durable
memory, background or scheduled tasks, messaging channels, MCP/connectors,
Subagents, a plugin marketplace, or a complex approval system. Those remain
later general-Agent capability packs, not rejected product directions.
