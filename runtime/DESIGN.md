# Ikaros Runtime Architecture

Status: first vertical-slice decisions locked on 2026-08-11. This document
records the intended kernel boundary; it is not a claim that the runtime has
already been implemented.

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

- Pi at `00eb2d1515e18e00940139f5e6568230a071f33c`: small loop, typed
  events, provider/context separation, steer/follow-up, and an explicit settled
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

Pi's default `read`, `write`, `edit`, and `bash` tools are useful foundational
system capabilities. They are not the boundary of a general-purpose Agent.

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

Model records are explicit configuration. V1 never calls an upstream `/models`
endpoint and never synthesizes model rows from frontend mock data. The empty
configuration contains no Provider or model rows; each row appears only after
the user adds it. A Custom Provider's `headers` field is a simple optional
string map. It has no separate secret type, persistence subsystem, merge UI, or
lifecycle; when the map is absent or empty, it has no effect on request
construction.

The client always sends the selected `provider_id/model_id` with `turn.start`.
The Desktop may visually preselect the sole available model, but the protocol
still carries the explicit reference. With zero configured models, submission
is unavailable and the Runtime returns `model_not_configured` if called. A
missing or unknown reference returns `model_selection_required`. The Runtime
never infers a model from map order or persists a hidden global default.

## Authority boundary

The runtime is authoritative for:

- Threads, Branches, Turns, Runs, Items, and their ordering;
- Agent-loop and provider execution;
- tool validation, execution, cancellation, and normalized results;
- execution policy;
- Skills and the context injected from them;
- canonical persisted Agent state.

Clients own presentation-only state such as theme, UI language, transient
selection, layout, and local username display. A client may project runtime
records into convenient UI shapes, but those shapes are not the wire protocol.

## Domain model

```text
Project?                optional organizational workspace
  -> Thread             one conversation
    -> Branch           one immutable path through edited/forked history
      -> Turn           one user request and all work caused by it
        -> Run          one execution, retry, or recovery attempt
          -> Item       message, tool call/result, artifact, and similar record
```

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
  -> ToolExecutor runs the command and emits lifecycle events
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
capability inference. It also performs no model discovery request. In
particular, model capabilities must not be guessed from name substrings. Any
reasoning replay field or Tool support is declared by the model/profile data.

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

Runtime operations should expose provider configuration separately from the
model catalog, for example `provider.list`, `provider.configure`,
`provider.remove`, `provider.test`, `model.list`, and `model.set_enabled`.
`configured` means valid configuration exists; `ready` means an explicit test
or real request succeeded. The current Mock `deepSeekConnected` Boolean is not
an authoritative runtime concept.

V1 has no disabled Provider state. Removing a Custom Provider deletes its full
entry from `config.yaml`, including its API key, headers, and models. The
operation is rejected while an active Run is using that Provider; otherwise it
does not need a replacement-default workflow because no global default exists.
The Desktop action should therefore be labelled Remove rather than Disconnect.
The built-in DeepSeek preset itself cannot be removed. Disconnecting DeepSeek
only clears its configured API key and leaves the preset and explicit model
records available for later reconfiguration.

API keys and custom header values may cross the authenticated loopback channel
only in a write-only configuration command. They are written to
`~/.ikaros/config.yaml`, but never enter the SQLite event journal, conversation
events, logs, list responses, exceptions, or UI projections. Responses expose
only redacted configuration state. Header names and values reject CR/LF, and
header merging is case-insensitive. No dedicated header-secret system is part
of V1.

The runtime caches HTTP clients per effective provider configuration and closes
them on replacement or shutdown. Connect, response-header, and stream-idle
timeouts are distinct. A request may be retried only before any content or Tool
Call delta has been emitted; transparent retry after streaming begins would
duplicate output. Provider failures normalize to stable categories such as
authentication, rate limit, context overflow, invalid request, timeout,
network, server, cancelled, protocol, and unknown, with safe optional status,
request ID, and retryability metadata.

A deterministic `ScriptedProvider` remains separate from this adapter and is
used for Agent-loop and event-ordering tests without credentials or network
access.

## Built-in tools and Skills

The ToolRegistry is extensible even though the first runnable gate is small.
Local file primitives such as read, write, and edit can coexist with a general
process/command tool. The first completion gate only requires real command
execution; it does not turn command execution into the product boundary.

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

A Skill is an instruction and resource bundle that may contain references,
assets, and scripts. Loading a Skill does not import third-party Python into the
long-lived runtime, and scripts are not expanded into one model ToolDescriptor
per file.

Instead, Skill instructions tell the Agent when and how to invoke a script via
the same general process/command tool used elsewhere:

```text
Skill discovery and selection
  -> read and inject SKILL.md instructions
  -> Agent requests the general command tool
  -> ToolExecutor starts the Skill script as a child process
  -> normalize stdout, stderr, exit status, and errors as ToolResult
  -> append the result and return it to the Agent loop
```

All command execution, including Skill scripts, shares argument validation,
working-directory and environment handling, timeouts, cancellation, process
tree cleanup, output truncation, lifecycle events, and error normalization. The
runtime may attribute a script path back to its Skill for UI and audit records;
that attribution does not create a separate executor.

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

Full access means a third-party Skill script can exercise the current operating
system user's authority. This is an explicit development-version trade-off, not
a sandbox or security guarantee.

The current SQLite schema version is 5. Each Run snapshots
`execution_policy = full_access`, and each Item has structured `data_json` for
Tool Call arguments and normalized results. Rebuilding projections from the
journal restores these records and the provider context. A bounded Agent loop
persists all calls from a provider Step before serial execution, returns every
result under the original provider call ID, and stops a provider that exceeds
the maximum Step count.

## Desktop projection

The current renderer's `AgentEvent` type remains a UI projection rather than
being frozen as the wire schema. Expected mappings include:

| Desktop concept | Runtime source |
| --- | --- |
| project and ordinary conversation lists | Project and Thread projections |
| streaming response | message Item lifecycle events |
| Stop | Run cancellation command and terminal event |
| tool card | tool Item lifecycle events |
| edited earlier user message | immutable Branch fork |
| Turn Navigator | Turn/Item projection used only for navigation |
| retry or recovery | a new Run linked to the previous Run |
| artifacts and file changes | typed Item/event projections |
| provider/model settings | runtime capability and model catalog |
| theme, language, username | client-only UI state |

The runtime sends stable semantics and original content, never pretranslated
Chinese or English labels. Fixed phrases such as tool status and Run state are
translated by renderer i18n; user and model message content is not translated.

The current mock cannot be integrated by changing one constructor alone. The
production boundary requires separate wire DTOs, a `RuntimeClient`, and a
projection reducer that converts sequenced runtime records into renderer
records. In particular:

- `ScenarioId`, seeded scenarios, fixed timestamps, and `playScenario` are mock
  concepts and do not enter the runtime contract.
- the current `AgentEvent` records are whole renderable cards, streaming text is
  represented by replacement, and `RunStatus` mixes Turn and Run state; these
  are not wire events;
- the current store keeps one global active Run and cancels the Mock client when
  creating or selecting a conversation. Runtime integration must track Runs by
  `runId` and `threadId`; navigation changes visibility, not task lifetime;
- Branch IDs, history, and fork relationships become runtime-authoritative
  instead of being created locally by the renderer;
- the current Composer defaults to `ask`, while its access and model selectors
  are local visual state that is not included in `sendDraft`. Runtime
  integration must make `Full access` the effective V1 policy and remove any
  implication that the current mock selector controls execution;
- provider/model forms are currently ephemeral mock state. Credentials and
  provider catalogs become runtime concerns only when real integration begins;
  theme, UI language, font, layout, and username presentation stay client-side.
- the Custom Provider form already describes an OpenAI-compatible endpoint but
  currently discards Base URL, API key, and custom headers on submit. Production
  integration sends those fields once to `provider.configure` and retains only
  a redacted `ProviderSummary` in renderer state;
- current hard-coded DeepSeek model rows are mock data and must not become the
  runtime catalog. Provider/model summaries come from the Runtime.
- the current Composer's `Ikaros`, `DeepSeek`, and `local` model choices are
  local Mock strings and are not sent with a prompt. Production Composer choices
  come from `model.list` and `turn.start` carries the selected Provider/model
  reference; there is no global default block in `config.yaml`.
- the current Custom action labelled Disconnect actually removes an in-memory
  row. Production UI calls the Runtime's destructive `provider.remove` and uses
  Remove wording; DeepSeek remains a built-in preset whose disconnect action
  clears only its API key.

## First runnable vertical slice

The first version is deliberately limited to one real conversation path. It is
complete when all of the following are demonstrated end to end:

1. Electron `RuntimeHost` starts one Python runtime and keeps it alive across
   renderer navigation.
2. A client connects through the versioned protocol and uses one Thread and its
   default Branch.
3. The same conversation completes at least two sequential user Turns, and the
   later provider request receives the prior completed conversation context.
4. The selected built-in DeepSeek profile or a configured Custom
   OpenAI-compatible profile streams a response and can request the general
   command tool through the same adapter.
5. The runtime executes the command serially, captures a normalized ToolResult,
   returns it to the model, and the model produces a final assistant answer.
6. The Run and Item lifecycle reaches a coherent settled state and is projected
   correctly by the Desktop client.
7. The deterministic provider covers the loop and event ordering without a
   network dependency.

This gate does not require web search, browser or desktop control, durable
memory, background or scheduled tasks, messaging channels, MCP/connectors,
Subagents, a plugin marketplace, or a complex approval system. Those are later
general-Agent capability packs, not rejected product directions.
