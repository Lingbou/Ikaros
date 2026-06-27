# Agent Loop

The agent loop is the model-guided execution path in `ikaros-runtime`. It lets a
model request governed tools from the active `SkillRegistry`, receive tool
results, and continue until it returns a final answer or hits a stop condition.

The loop owns turn orchestration. It does not own provider authentication,
provider wire format, policy decisions, or host execution. Those responsibilities
belong to `ModelProvider`/`ModelTransport` and the harness.

## Scope

The loop is small by design:

- bounded iteration count
- provider-native tool calls when available
- strict fallback JSON tool-call parsing for non-native output
- governed skill dispatch
- typed `AgentEvent` and `ModelStreamEvent` records
- prompt-free audit metadata
- guardrail observation

The model never executes tools directly. Every tool call is normalized and sent
through `ExecutionSession`.

## Interface

`AgentRuntime::run_turn()` receives:

- `AgentLoopInput`: optional session id, optional turn id, optional task id,
  system prompt, and user input.
- `ModelProvider`: the configured provider adapter.
- `ExecutionSession`: policy, approval, audit, and environment context.
- `SkillRegistry`: executable tools from `ikaros-toolkit`, governed through the
  harness.
- `AgentLoopOptions`: iteration, sampling, streaming, and guardrail settings.

The default implementation is `HarnessAgentRuntime`. Callers that need a
different loop implementation should swap the runtime layer, not the provider
adapter.

`RecordingAgentRuntime` wraps any `AgentRuntime` and records the same typed
event stream it forwards to the caller's sink. It is the replay/test adapter for
callers that need a complete in-memory event trace without making
`AgentLoopReport` the source of truth.

Agent-loop system prompts are assembled through the shared `PromptBuilder`
pipeline. The base system text, strict tool-call protocol, deferred tool
disclosure guidance, and available tool manifest are separate typed prompt
sections before rendering. `TurnStart` events persist prompt metadata
(`kind`, `source`, `title`, priority, token estimate, and redaction state) but
do not persist raw prompt section content. Replay and UI callers can inspect why
tool guidance was present without scraping prompt text or storing secrets.

`AgentHarness` is the stateful wrapper above `AgentRuntime` for callers that
need a stable session id, per-turn ids, phase tracking, and continuation queues.
It owns the harness phase and can run steer, follow-up, next-turn, resume,
compact, and retry continuations, then delegates model turns to
`AgentRuntime::run_turn_with_events()`. The
returned `AgentHarnessTurn` keeps typed events first. The harness collects the
same emitted event stream it forwards to the caller's sink and uses that stream
to populate `AgentLoopReport.events` as the in-process turn snapshot. Built-in chat
and task agent-loop entry points use this wrapper. Agent-loop handoff also uses
this path and supplies a subagent session source when the caller did not provide
one. Gateway task drains and scheduled task execution also use the session-aware
task agent-loop path with gateway/schedule session ids, turn ids, and source
metadata. Direct `run_agent_loop*` helpers remain the low-level API for tests
and specialized runtimes.

The harness phase is not just a display enum. `AgentHarnessPhase` now has
concrete public operations for branch summaries, compaction markers, and retry
markers through `append_branch_summary()`, `append_compaction()`, and
`append_retry_marker()`. Each helper runs as a bounded harness phase and writes
through `SessionStore`. The session tree remains append-only; branch,
compaction, retry, and active-leaf operations add or select entries instead of
rewriting previous turns.

Continuation queues are durable when the harness is configured with a
`SessionStore`. `ikaros-session` stores queued, running, completed, failed, and
cancelled continuations in `state.db`. Claiming a continuation writes a lease,
increments its attempt count, records a status reason, and reclaims expired
running leases before selecting the next item. Failed or cancelled continuations
can be requeued with an explicit reason. The harness can claim and complete
message continuations (`steer`, `follow_up`, `next_turn`, `resume`) and
maintenance continuations (`compact`, `retry`), plus first-pass recoverable
tool-result continuations. Message continuations run a real turn. Maintenance
continuations append session entries and emit
`ContinuationStarted` / `ContinuationCompleted` / `ContinuationFailed` events
without inventing a model response. Tool-result continuations re-run the
recoverable tool through the harness session, redact the terminal payload, and
complete or fail the queued continuation without fabricating a model response.
Explicit harness cancellation writes a `ContinuationCancelled` event. A running
durable message continuation polls the store for external cancellation, cancels
its turn token, and emits an acknowledgement event when the worker stops.
`ikaros debug continuations
<session-id>` reports queue status, status reason, lease owner, lease expiry,
attempts, terminal summaries, worker-lease timeout evidence, errors, and
redacted payloads. Without a continuation store, the harness keeps the old
in-memory queues for tests and specialized one-shot callers.
This is still a continuation queue, not a complete scheduler. Poll interval
tuning, scheduler-grade worker coordination, richer tool-result scheduling
policy, and automation-facing timeout reports are still runtime hardening work.

`AgentLoopOptions::with_hooks()` installs observer-only `AgentLoopHooks` for
provider request/response and tool call boundaries. Hook payloads carry
redacted metadata and event anchors, not raw prompt or tool secrets. Hook
failures are recorded as runtime error events and do not mutate or stop the
turn. Durable facts should still be read from the typed `AgentEvent` stream and
persisted session timeline; hooks are an extension boundary for telemetry,
policy observation, UI, and replay diagnostics.

Callers that need durable timelines should call `run_turn_with_events()` with an
`AgentEventSink`. `ikaros-session` provides `PersistingAgentEventSink` for
per-event writes, `PersistingAgentTurnSink` for turn-scoped transaction writes
into the local SQLite `SessionStore`, `CollectingAgentEventSink` for replay/test
and live UI collectors, and `FanoutAgentEventSink` for delivering the same
typed event stream to persistence plus observers. Runtime wrappers should use
these sinks instead of implementing local callback fan-out.

`session_id` is the persistence identity for the event timeline. `turn_id`
identifies one persisted turn inside that timeline. Callers may supply a turn id
when they need chat session entries, projected history records, and agent events
to share the same turn identity. `task_id` remains task/report metadata. If no session id is
supplied, the loop creates a fresh `SessionId` for that turn instead of reusing a
global fallback session.

`AgentHarnessConfig` may also carry a caller-supplied `turn_id`. Chat uses that
to keep append-only session entries, projected history records, and agent events
on the same turn. This is a one-turn override: after that turn runs, continuation
turns receive fresh ids unless the caller explicitly supplies another one. Task
agent-loop runs let the harness create a fresh turn id inside the task session.
Callers can clone the harness cancellation token or call `AgentHarness::cancel()`
to abort the next provider request or any planned tool calls that have not
started yet, or to drop an in-flight tool future that is still awaiting
completion.

Default options:

- `max_iterations = 4`
- `max_tokens = 512`
- `temperature = 0.2`
- `stream = false`
- default guardrail settings
- a fresh cancellation token

## Turn Sequence

Each iteration follows the same order:

1. Check the cancellation token before issuing a provider request.
2. Build a model request with system prompt, user input, prior assistant output,
   tool definitions, and prior tool results.
3. Invoke the `before_provider_request` hook, then ask the provider for a
   normal or streaming response.
4. Invoke the `after_provider_response` hook and normalize the provider response
   into text, stream, tool-call, usage, error,
   and done records.
5. Prefer provider-native tool calls when present.
6. If no native tool call exists, parse the fallback JSON protocol from text.
7. If a final answer is present, stop with `FinalAnswer`.
8. Check cancellation again before dispatching planned tool calls.
9. Emit `ToolCallStarted`, invoke the `before_tool_call` hook, then dispatch
   normalized tool calls through `ExecutionSession`.
10. Emit tool lifecycle events for each tool result, then invoke the
   `after_tool_call` hook with the redacted result status. Normal dispatches
   emit `ToolCallOutputDelta` followed by `ToolCallCompleted` or
   `ToolCallFailed`; cancelled calls emit `ToolCallCancelled`. If cancellation
   is requested after the model returns a tool plan but before dispatch begins,
   the runtime emits `ToolCallCancelled` for each planned call and does not
   invoke the skill. If cancellation is requested while a tool future is already
   in flight, the runtime drops that future, emits `ToolCallCancelled`, and
   stops the turn with `Cancelled`.
11. Append tool results to the next model turn in the model's original tool
    call order, even when a parallel batch completed out of order.
12. Observe guardrails and iteration budget before continuing.

Provider-native tool call ids are preserved when the provider supplies them, so
tool result history can be sent back in the provider's preferred shape.

Tool scheduling is driven by harness metadata, not by provider adapters. Each
`SkillDescriptor` exposes an `execution_mode` and optional `timeout_ms`. The
runtime executes contiguous `parallel` tool calls concurrently and preserves the
original call order when appending tool results to the next model request.
`sequential` calls are executed alone. Safe read and shell read tools default to
parallel; tools with write, network, remote, destructive, secret, or
self-modification risk default to sequential unless a descriptor explicitly
narrows or changes the policy.

## Stop Reasons

The loop can stop because:

- a final answer was produced
- the iteration budget was reached
- policy denied a requested tool
- a requested tool needs approval
- a guardrail halted execution
- a provider error was observed
- a cancellation, compaction, tool error, or context limit stopped the turn

Task and agent commands can opt into the loop with `--agent-loop`. Non-stream chat uses it by
default; `--no-agent-loop` forces a single provider call.

Structured reports use these stop reasons:

- `FinalAnswer`
- `IterationBudget`
- `PolicyDenied`
- `WaitingForApproval`
- `GuardrailHalt`
- `Cancelled`
- `ProviderError`
- `Compacted`
- `ToolError`
- `ContextLimit`

Transport and local store failures may still return command errors when no
complete report can be built. When the runtime can emit an event before
returning, provider failures are also surfaced as typed error events.

## Tool Calls

Preferred path:

1. Provider receives native tool definitions.
2. Provider returns native tool calls.
3. Runtime normalizes them.
4. Harness dispatches them.

Fallback path:

```json
{"tool_calls":[{"id":"optional_call_id","name":"tool_name","input":{}}]}
```

Progressive-disclosure bridge tools follow an additional contract. Deferred
tools are not callable merely because the model can guess their names. The model
must first call `tool_search` and receive the target in the returned descriptors,
or call `tool_describe` for the target. Only then may it call that disclosed
deferred tool through `tool_call` in the same execution session. The target tool
still goes through harness policy, approval, audit, and `ExecutionEnv`.

Final answer:

```json
{"final_answer":"..."}
```

The fallback parser only accepts the canonical top-level JSON object shown
above. It does not accept fenced JSON, embedded JSON, top-level arrays, or alias
keys such as `tools`, `calls`, `tool_call`, `function_call`, `args`,
`arguments`, `answer`, or `response`. `final_answer`, each tool `name`, and any
present tool `id` must be non-empty after trimming whitespace. Each iteration
records the parse strategy in the report.

Parse strategies reported by the loop are:

- `provider_native_tool_calls`
- `json_fallback`
- `plain_text`

`repaired` is currently always false. Broad JSON repair was removed before MVP
so the runtime contract stays narrow.

## Report Contract

`AgentLoopReport` contains:

- stop reason
- final content
- provider and model names
- token usage
- whether streaming was used
- stream chunks when streaming is enabled
- typed events emitted during the turn
- iteration count
- tool-call diagnostics
- tool results

Tool result summaries and outputs are produced by the harness. They should be
redacted before surfacing to users or audit output.

Tool lifecycle event payloads include the normalized tool name, provider tool
call id when present, a redacted input snapshot, output summary/delta, status,
execution mode, timeout, and a stable tool-event anchor used by approval and
audit evidence. Successful harness dispatches also emit an `AuditAnchor` event
that binds the tool-event id, harness call id, audit event id, audit kind, and
audit path. Secrets must be redacted before those payloads enter reports or
persisted session events. A descriptor timeout turns that tool call into a
failed tool lifecycle result with structured timeout metadata, including
timeout duration and start/end timestamps; it does not let the runtime bypass
`ExecutionSession` or `ExecutionEnv`. Cancellation requested before a planned
call starts produces a `ToolCallCancelled` payload and stops the turn with
`Cancelled`; cancellation while a provider request or tool future is in flight
also stops the turn and drops the pending future. Process-backed local tools
rely on `kill_on_drop` in the local `ExecutionEnv` process runner.

`AgentLoopReport.events` is an in-process snapshot of the events emitted during
the current turn. The durable fact source is the `ikaros-session` event stream
when a persisting sink is attached. Replay, gateway, schedule, and UI paths
should read the session store instead of reconstructing timelines from human
output.

The built-in chat path uses `PersistingAgentTurnSink`. Agent-loop chat and
single-call chat selected with `--no-agent-loop` both write user/assistant
`SessionEntry` records. Single-call chat also emits a minimal typed event
timeline: session start, turn start, user message, normalized model stream
events, context diff, and turn end. The context diff payload records the
provider-aware token budget, sections, explicit references, compressed sections,
and added/removed/compressed context estimates for the turn. When context is
compacted, chat also writes a `ContextCompacted` event and a compaction session
entry before the assistant entry. Post-turn evidence such as `MemoryLifecycle`
and `AuditAnchor` may appear after `TurnEnd`; consumers should use event kinds
rather than assuming the last event is always the turn end.

Those session entries and chat agent events for one turn commit or roll back
together. Chat history is projected from the same `state.db` replay; core
memory records, memory candidates, and audit writes remain separate stores for
now. Memory sync writes safe redacted turn context into session working memory
with `MemoryRef::SessionTurn`; ordinary turn summaries are not promoted into
long-term `Task` memory. Automatic relationship observations enter the
candidate inbox instead of core memory. The session timeline only stores the
high-level lifecycle evidence. The local memory journal records the matching
`sync_turn` append/skipped-write decision and any turn-scoped promote, demote,
forget, or quota policy action so debug callers can inspect memory lifecycle
behavior without reading the memory store directly.
Approval requests
created by a persisting agent-loop turn are double-written into the session
approval table with redacted request data; later approve, deny, or execute
decisions update the same session approval record and emit `ApprovalResolved`.

Provider failures and local post-processing failures are recorded before the
turn is reported as failed. A failed chat turn keeps the user `SessionEntry`,
emits an `Error` event with a redacted message and phase, and ends with a
failed `TurnEnd` event so replay/debug callers do not lose the timeline.

Ordinary chat turns do not write a separate history mirror. User/assistant
entries, model stream events, context diff, memory lifecycle evidence, and audit
anchors are committed through the session store. Replay, workbench history,
search, and context assembly read `state.db` session replay as the single chat
timeline.

## Invariants

- The prompt may describe tools, but only the harness registry defines what can
  be executed.
- Tool definitions include name, description, input schema, and risk level.
- A denied or approval-waiting tool call stops the loop; the loop does not try a
  different tool to bypass policy.
- Guardrails observe repeated failures and lack of progress after each tool
  dispatch.
- The fallback JSON protocol is the strict text envelope for providers that do
  not expose native tool calls. Provider-native tool calls remain the preferred
  path.
