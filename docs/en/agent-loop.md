# Agent Loop

The agent loop is the model-guided execution path in `ikaros-runtime`. It lets a model request harness skills, receive tool results, and continue until it returns a final answer or hits a stop condition.

The loop owns turn orchestration. It does not own provider authentication,
provider wire format, policy decisions, or host execution. Those responsibilities
belong to `ModelProvider`/`ModelTransport` and the harness.

## Scope

The loop is small by design:

- bounded iteration count
- provider-native tool calls when available
- strict fallback JSON tool-call parsing for non-native output
- harness skill dispatch
- typed `AgentEvent` and `ModelStreamEvent` records
- prompt-free audit metadata
- guardrail observation

The model never executes tools directly. Every tool call is normalized and sent through `ExecutionSession`.

## Interface

`AgentRuntime::run_turn()` receives:

- `AgentLoopInput`: optional session id, optional turn id, optional task id,
  system prompt, and user input.
- `ModelProvider`: the configured provider adapter.
- `ExecutionSession`: policy, approval, audit, and environment context.
- `SkillRegistry`: executable harness skills.
- `AgentLoopOptions`: iteration, sampling, streaming, and guardrail settings.

The default implementation is `HarnessAgentRuntime`. Callers that need a
different loop implementation should swap the runtime layer, not the provider
adapter.

`RecordingAgentRuntime` wraps any `AgentRuntime` and records the same typed
event stream it forwards to the caller's sink. It is the replay/test adapter for
callers that need a complete in-memory event trace without making
`AgentLoopReport` the source of truth.

`AgentHarness` is the stateful wrapper above `AgentRuntime` for callers that
need a stable session id, per-turn ids, phase tracking, and continuation queues.
It owns the harness phase and the steer, follow-up, and next-turn queues, then
delegates the actual turn to `AgentRuntime::run_turn_with_events()`. The
returned `AgentHarnessTurn` keeps typed events first and exposes
`AgentLoopReport` as the current compatibility summary. Built-in chat and task
agent-loop entry points use this wrapper; direct `run_agent_loop*` helpers remain
the low-level API for tests and specialized runtimes.

Callers that need durable timelines should call `run_turn_with_events()` with an
`AgentEventSink`. `ikaros-session` provides `PersistingAgentEventSink` for
per-event writes and `PersistingAgentTurnSink` for turn-scoped transaction
writes into the local SQLite `SessionStore`.

`session_id` is the persistence identity for the event timeline. `turn_id`
identifies one persisted turn inside that timeline. Callers may supply a turn id
when they need chat history, session entries, and agent events to share the same
turn identity. `task_id` remains task/report metadata. If no session id is
supplied, the loop creates a fresh `SessionId` for that turn instead of reusing a
global fallback session.

`AgentHarnessConfig` may also carry a caller-supplied `turn_id`. Chat uses that
to keep the chat history record, append-only session entries, and agent events
on the same turn. Task agent-loop runs let the harness create a fresh turn id
inside the task session. Callers can clone the harness cancellation token or
call `AgentHarness::cancel()` to abort the next provider request or any planned
tool calls that have not started yet, or to drop an in-flight tool future that
is still awaiting completion.

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
3. Ask the provider for a normal or streaming response.
4. Prefer provider-native tool calls when present.
5. If no native tool call exists, parse the fallback JSON protocol from text.
6. If a final answer is present, stop with `FinalAnswer`.
7. Check cancellation again before dispatching planned tool calls.
8. Dispatch normalized tool calls through `ExecutionSession`.
9. Emit tool lifecycle events:
   `ToolCallStarted`, `ToolCallOutputDelta`, `ToolCallCompleted`, or
   `ToolCallFailed`. If cancellation is requested after the model returns a
   tool plan but before dispatch begins, the runtime emits `ToolCallCancelled`
   for each planned call and does not invoke the skill. If cancellation is
   requested while a tool future is already in flight, the runtime drops that
   future, emits `ToolCallCancelled`, and stops the turn with `Cancelled`.
10. Append tool results to the next model turn.
11. Observe guardrails and iteration budget before continuing.

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

Task and agent commands can opt into the loop with `--agent-loop`. Non-stream chat uses it by default; `--no-agent-loop` forces a single provider call.

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

Final answer:

```json
{"final_answer":"..."}
```

The fallback parser only accepts the canonical top-level JSON object shown
above. It does not accept fenced JSON, embedded JSON, top-level arrays, or alias
keys such as `tools`, `calls`, `tool_call`, `function_call`, `args`,
`arguments`, `answer`, or `response`. Each iteration records the parse strategy
in the report.

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
execution mode, timeout, and a stable tool-event anchor used by approval events.
Secrets must be redacted before those payloads enter reports or persisted
session events. A descriptor timeout turns that tool call into a failed tool
lifecycle result; it does not let the runtime bypass `ExecutionSession` or
`ExecutionEnv`. Cancellation requested before a planned call starts produces a
`ToolCallCancelled` payload and stops the turn with `Cancelled`; cancellation
while a tool future is in flight produces the same lifecycle event and drops the
future. Process-backed local tools rely on `kill_on_drop` in the local
`ExecutionEnv` process runner.

`AgentLoopReport.events` is a compatibility summary for current callers. The
durable fact source is the `ikaros-session` event stream when a persisting sink
is attached. Replay, gateway, schedule, and UI paths should read the session
store instead of reconstructing timelines from human output.

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
together. Chat history, memory records, relationship learning, and audit writes
are still separate stores for now. Memory sync can write a redacted turn-summary
record with `MemoryRef::SessionTurn`; the session timeline only stores the
high-level lifecycle evidence. The local memory journal records the matching
`sync_turn` append/skipped-write decision and any turn-scoped promote, demote,
forget, or quota policy action so debug callers can inspect memory lifecycle
behavior without reading the memory store directly. Approval requests
created by a persisting agent-loop turn are double-written into the session
approval table with redacted request data; later approve, deny, or execute
decisions update the same session approval record and emit `ApprovalResolved`.

Provider failures and local post-processing failures are recorded before the
turn is reported as failed. A failed chat turn keeps the user `SessionEntry`,
emits an `Error` event with a redacted message and phase, and ends with a
failed `TurnEnd` event so replay/debug callers do not lose the timeline.

## Invariants

- The prompt may describe tools, but only the harness registry defines what can
  be executed.
- Tool definitions include name, description, input schema, and risk level.
- A denied or approval-waiting tool call stops the loop; the loop does not try a
  different tool to bypass policy.
- Guardrails observe repeated failures and lack of progress after each tool
  dispatch.
- The fallback JSON protocol is compatibility behavior. Provider-native tool
  calls remain the preferred path.
