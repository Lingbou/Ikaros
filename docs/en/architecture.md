# Architecture

Ikaros is a persona-first local Agent Runtime. The core boundary is: runtime orchestrates turns, the harness executes tools, provider adapters only handle model wire formats, and local state stays under `IKAROS_HOME` by default.

This document describes the runtime contract. It is not a module inventory.
When code moves between files, this page should change only if ownership,
calling context, persistent state, or user-visible behavior changes.

## Terms

- Runtime: code that resolves config, agent identity, stores, providers, context,
  and the harness session for one command or worker tick.
- Harness: the policy, approval, audit, and execution boundary for every tool.
- Provider: an adapter that talks to a model, embedding, TTS, or ASR API.
- Transport: the wire-format description for a provider family.
- Model stream event: a normalized model delta such as text, reasoning, tool-call
  start/update/end, usage, error, or done.
- Agent event: the typed runtime event emitted for session, turn, user, model,
  tool, approval, memory lifecycle, audit anchor, context, error, and turn-end
  milestones.
- Session store: the append-only session, turn, event, approval, and replay
  persistence boundary, including durable continuation queue records. The
  current implementation is local SQLite with FTS5 and trigram indexes for
  session entry search.
- Context bundle: the token-budgeted set of context sections used for a turn,
  plus parsed references and a diff explaining what was added, removed, or
  compressed.
- Coding turn context: the workspace, git state, mode, permission profile,
  instructions, test commands, and session/turn identity used by the controlled
  coding workflow.
- Agent profile: persona and policy overlay.
- Agent instance: runtime identity with `agent_id`, workspace, state directory,
  session policy, auth scope, and route bindings.
- Context source: references, history, memory, RAG, relationship, or persona
  data that may be assembled into a model turn.

## Crates

- `ikaros-core`: shared config, paths, task types, redaction, errors, agent profiles, and the `AgentInstance` identity model.
- `ikaros-session`: `SessionId`, `TurnId`, typed `AgentEvent`, append-only
  session entries, `SessionStore`, `SessionWriter`, SQLite `state.db`, and
  replay/search/branch/continuation reads.
- `ikaros-context`: context bundles, sections, references, provider-aware
  token budgets, quota-based compaction, and context diffs.
- `ikaros-runtime`: diagnostics, chat, tasks, schedules, gateway drain, body frames, agent handoff, `AgentRuntime`, `AgentHarness`, and context orchestration.
- `ikaros-harness`: policy decisions, approvals, audit logs, `ExecutionSession`, `ExecutionEnv`, skill execution, plugins, guardrails, and the task runner.
- `ikaros-memory`: JSONL/SQLite memory stores, `MemoryProvider` lifecycle,
  memory policy/journal primitives, and provider registry metadata.
- `ikaros-rag`: local file ingestion, chunk storage, retrieval, and embedding providers.
- `ikaros-models`: `ModelProvider`, `ModelTransport`, model context profiles, mock, OpenAI-compatible, Anthropic, Ollama, streaming, tool-call normalization, usage logging, and request governance.
- `ikaros-gateway`: local inbox/outbox store plus built-in `GatewayFrame` protocol types.
- `ikaros-voice`: mock and OpenAI-compatible TTS/ASR providers.
- `ikaros-skills`: built-in skills exposed through the harness.
- `ikaros-cli`: command-line interface and terminal rendering.
- `ikaros-body`: body/status/frame contracts and simple renderers.
- `ikaros-automation`, `ikaros-service`, `ikaros-coding`, and `ikaros-soul`: focused support crates for their named domains. `ikaros-coding` owns repo scan, guarded patching, structured patch failures, turn diff tracking, code review, coding turn reports, self-modify records, and test-command analysis.

## Runtime Flow

Most entry points follow the same path:

1. CLI or workers resolve `IKAROS_HOME`, workspace, config, and agent id/profile.
2. Runtime resolves an `AgentInstance` with `agent_id`, profile overlay, workspace, state dir, session policy, auth scope, and route bindings.
3. Runtime builds stores, provider adapters, the skill registry, context engine, and harness session.
4. Model turns run through `AgentRuntime`; the default implementation is
   `HarnessAgentRuntime`. Chat and task agent-loop entry points wrap it in
   `AgentHarness`, which owns phase, caller-provided turn ids, and durable
   continuation queue handling when a `SessionStore` is available. Gateway task
   drains, scheduled task execution, and agent-loop handoff now call the
   session-aware task agent-loop path with explicit session id, turn id, and
   source metadata. Runtime emits typed `AgentEvent` records. Callers may attach
   an `AgentEventSink` to persist those records in `ikaros-session`, while
   existing CLI and worker callers can still use the final report.
5. Tool dispatch must go through `ExecutionSession` and `ExecutionEnv`; runtime code should not touch host APIs directly.
6. The harness evaluates policy, records audit events, and either executes, asks for approval, or denies.
7. Runtime reduces the same turn path into stable reports for CLI, body,
   schedule, gateway, chat, or agent callers.

Chat and task agent-loop execution now use the stateful harness path. Gateway
task drain, scheduled task execution, and agent-loop handoff also enter that path
with explicit session source metadata, so their agent-loop events and
continuation state can land in the same `state.db` timeline as their
gateway/schedule evidence.
The durable continuation queue is a recovery and replay boundary, not yet a
full scheduler. It now records leases, attempt counts, status reasons, requeue
status, terminal status, cancellation request/acknowledgement evidence,
worker-lease timeout summaries, and user-facing debug query data. Running
durable message continuations poll for external cancellation, but configurable
worker coordination, tool-result continuations, and scheduler-grade terminal
accounting are still runtime hardening work.

## Agent Identity

Profiles and instances are intentionally separate.

Profiles describe how an agent should behave: mode, persona overlay, context
sources, and ordinary policy defaults. Instances describe who is running and
where state belongs. A configured instance may select a profile while providing
its own workspace, state directory, session policy, auth scope, and route
bindings.

Resolution order:

1. If the requested name matches `agent.instances.<id>`, runtime creates an
   `AgentInstance` from that entry.
2. If no instance matches, the name is resolved as an agent profile.
3. If no name is passed, `agent.default` is used.
4. If the default profile is missing, runtime falls back to the built-in
   `build` profile.

Callers should pass the resolved `AgentInstance` into harness sessions. Policy
overlays and approval replay use the instance identity; persona text alone must
not grant permissions.

## Local State

Default state lives under `~/.ikaros` or `IKAROS_HOME`.

JSONL remains the default local storage format because it is inspectable and easy to recover. SQLite is available for larger local stores such as memory, chat history, and RAG indexes. Remote services are not required for the MVP.

State ownership:

- `state.db`: session metadata, append-only session entries, persisted
  chat/agent-loop events, gateway and schedule evidence, approval records,
  durable continuation queue records, FTS5/trigram search indexes,
  branch/compact/retry markers, coding turn events, and replay data.
  Built-in chat turns write user/assistant entries through a turn-scoped
  `SessionWriter` transaction. Gateway and schedule workers also map their
  request/result/delivery evidence into the same store. Memory lifecycle and
  audit logs still keep their own stores, with selected session evidence rather
  than full prompt-bearing duplication.
- `memory/`: local memory records, memory policy journal data, and memory provider registry metadata.
- `chat/`: chat history and session summaries.
- `rag/`: local RAG files, chunks, and embedding indexes.
- `audit/`: policy decisions, approval records, usage logs, and migration backups.
- `automation/`: schedule metadata and delivery reports.
- `gateway/`: inbox/outbox records and sibling lock files for local message routing.
- `skills/`: locally installed plugins and marketplace metadata.
- `agents/`: per-agent state directories when instances use the default state root.

## Boundaries

- Persona affects prompts and context, not policy.
- Agent profiles are persona/policy overlays; `AgentInstance` is the runtime identity.
- `ModelProvider` generates/streams model output; `ModelTransport` describes provider wire format; `ModelStreamEvent` normalizes provider deltas; `AgentRuntime` owns the turn loop and emits `AgentEvent`.
- `AgentEvent`, session ids, turn ids, append-only session entries, and replay
  reads belong to `ikaros-session`, not to the runtime loop.
- `SessionWriter` owns turn-scoped session transactions. Built-in chat uses it
  for session entries and typed events. Gateway and schedule workers write
  high-level evidence entries/events into `state.db`. Memory, audit, and legacy
  chat-history writes remain separate stores, with session evidence kept
  explicit and redacted.
- Built-in chat turns commit session entries and chat events together. Failed
  provider or local post-processing turns keep the user entry, a redacted error
  event, and a failed turn-end event for replay/debug callers.
- `session_id` identifies persisted timelines. `task_id` is task/report
  metadata and must not be used as an implicit session fallback.
- Context primitives live in `ikaros-context`. Runtime chat assembles
  relationship, explicit references, history, memory, and RAG into a
  provider-aware token-budgeted `ContextBundle`. Provider metadata caps the
  usable context window and selects the token estimator. OpenAI-compatible and
  mock providers have deterministic local adapters; Anthropic and Ollama still
  use explicit fallback adapters until exact native tokenizer libraries are
  wired in.
- `ContextReference` currently parses and locally resolves safe references:
  `@file:path:line-line`, `@folder:path`, `@git:rev`, `@diff`, and `@staged`.
  Paths must stay under the workspace. `@url:` is parsed but not fetched until
  network policy is wired into context assembly.
- Context assembly emits a `ContextDiff` agent event for the turn. The payload
  includes the budget, sections, parsed references, and added/removed/compressed
  token estimates.
- Context compaction protects relationship facts and explicit references. If
  protected context cannot fit the model-derived budget, the turn fails with a
  context-limit error instead of silently dropping the requested context.
- `MemoryProvider` exposes turn_start, prefetch, sync_turn, pre_compress, session_switch, and delegation_observation lifecycle hooks. The trait does not hide default noop methods; callers that need no memory side effect must choose `NoopMemoryProvider` explicitly.
- `MemoryScore`, `MemoryPolicy`, and `MemoryJournal` belong to `ikaros-memory`.
  Runtime chat records `sync_turn` working-memory append/skipped-write
  decisions. When a lifecycle report references affected core memory scopes, it
  applies configured promote/demote/forget/quota policy actions and records
  those decisions in the same journal. Ordinary turn summaries are not promoted
  into long-term `Task` memory.
- Relationship memory is `MemoryKind::Relationship` in `ikaros-memory`; the
  relationship CLI is a convenience façade over the memory store, not a second
  memory system.
- Tool execution belongs to the harness and `ExecutionEnv`, not the model provider or UI.
- Coding workflow execution is a governed harness skill. It builds a
  `CodingTurnContext`, git baseline, repo map, change plan, optional patch
  attempt, turn diff, test matrix evidence, review, iteration plan, loop report,
  and final report. The git baseline records HEAD, branch/detached state,
  clean/dirty/not-git/unknown state, and staged/unstaged/untracked flags when
  available. The mode policy is explicit: `plan` and `review` stay read-only,
  `test` may run the test matrix through the harness process path, `edit` may
  apply an explicitly requested candidate patch, and `self_modify` is rejected
  by ordinary `code workflow` until it enters the dedicated self-modify approval
  path. Workspace instructions are loaded from `IKAROS.md` and
  `.ikaros/instructions.md` and redacted before entering prompts or events.
  With `--model-loop`, the configured model provider returns strict JSON
  candidate patches; approved execution records model request/response metadata,
  token-budget stops, cancellation stops, patch attempts, test evidence, review,
  and loop termination into `state.db` for `debug coding-turn`. The
  terminal-first `code plan`, `code apply`, `code test`, `code review`, and
  `code rollback` commands route into this same workflow. Git status snapshots
  are collected through the session `ProcessRunner` path when a fixture is not
  present.
- Tool lifecycle uses typed events: `ToolCallStarted`,
  `ToolCallOutputDelta`, `ToolCallCompleted`, `ToolCallFailed`, and
  `ToolCallCancelled`. Approval events carry tool anchors so UI, replay, and
  audit views can line up the request with the tool invocation.
- Agent-loop observer hooks cover provider request/response and tool start/end
  boundaries. Hook payloads are redacted metadata; typed events and persisted
  session timelines remain the durable observation surface.
- Tool scheduling is descriptor-driven. Adjacent parallel tool calls may run
  concurrently, sequential calls run alone, and per-tool timeout failures are
  reported through the same lifecycle event stream with structured timeout
  metadata. Cancellation is checked before and while awaiting provider requests,
  before planned tool calls start, and while tool futures are in flight; planned
  but unstarted calls are reported as cancelled, not executed.
- Gateway protocol types live inside `ikaros-gateway`; there is no separate protocol crate.
- Self-modification is a separate approval-gated path, not an ordinary write permission.
- The current coding workflow is now a provider-backed controlled loop, but it
  is still pre-MVP. It has deterministic, mock-model, and provider-loop replay
  fixtures, multi-iteration patch/test/review evidence, test-matrix events, and
  parser hardening for malformed ranges, quoted/space-truncated paths,
  ambiguous anchors, already-applied hunks, generated malformed corpus cases,
  and generated line-update roundtrips. Terminal-first coding commands are
  available from both `ikaros code ...` and the chat REPL `/code ...` wrapper,
  including rollback from persisted turn diff evidence. Coding approval requests
  now carry structured provider/shell/write/session context and the terminal
  renders `approval_scope`, `coding_progress`, and `coding_result` summaries.
  Provider-backed coding turns can be cancelled while awaiting a provider call;
  cancellation records `coding_loop_cancelled` before later patch/test steps run.
  Deeper property/fuzz coverage remains future hardening.

## Invariants

- A model response is never trusted as an instruction to execute host operations.
  Tool calls must be normalized and dispatched through `ExecutionSession`.
- Runtime events are append-only observations of a turn. Reports may summarize
  them, but tooling should prefer typed event fields over parsing human text.
- Persisted session timelines are append-only. Branch, compact, retry, active
  leaf switching, and replay add or select entries instead of mutating old turn
  facts.
- A provider adapter must not own the agent loop, approval flow, or workspace
  mutation policy.
- Context assembly may call safe-read skills with redacted audit input, but the
  audit log must not store full user prompts.
- Approval replay must bind the workspace, exact approved input, and agent
  identity.
- Gateway ingestion only queues work. It must not call models, tasks, plugins, or
  tools directly.
- Self-modify proposals use a dedicated proposal/apply/rollback path and do not
  imply general write permission.

## Failure Reporting

Most runtime paths return structured reports for internal callers and render a
human-oriented CLI summary. Reports should include enough information to explain
why work stopped without storing prompt text or secrets. Common stop conditions
are policy denial, waiting for approval, iteration budget, guardrail halt,
provider error, command timeout, and local store errors.

Session replay is currently strongest for completed and failed chat/agent-loop
turns, and it also contains high-level gateway and schedule request/result/
delivery evidence. Memory and audit still have dedicated stores, so long-running
workers should treat `state.db` as the primary timeline and those stores as
supporting evidence until their lifecycle records are fully modeled.
