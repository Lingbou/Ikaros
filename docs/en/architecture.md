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
  persistence boundary. The current implementation is local SQLite with FTS5
  and trigram indexes for session entry search.
- Context bundle: the token-budgeted set of context sections used for a turn,
  plus parsed references and a diff explaining what was added, removed, or
  compressed.
- Agent profile: persona and policy overlay.
- Agent instance: runtime identity with `agent_id`, workspace, state directory,
  session policy, auth scope, and route bindings.
- Context source: references, history, memory, RAG, relationship, or persona
  data that may be assembled into a model turn.

## Crates

- `ikaros-core`: shared config, paths, task types, redaction, errors, agent profiles, and the `AgentInstance` identity model.
- `ikaros-session`: `SessionId`, `TurnId`, typed `AgentEvent`, append-only
  session entries, `SessionStore`, `SessionWriter`, SQLite `state.db`, and
  replay/search/branch reads.
- `ikaros-context`: context bundles, sections, references, provider-aware
  token budgets, quota-based compaction, and context diffs.
- `ikaros-runtime`: diagnostics, chat, tasks, schedules, gateway drain, body frames, agent handoff, `AgentRuntime`, and context orchestration.
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
- `ikaros-automation`, `ikaros-service`, `ikaros-coding`, and `ikaros-soul`: focused support crates for their named domains.

## Runtime Flow

Most entry points follow the same path:

1. CLI or workers resolve `IKAROS_HOME`, workspace, config, and agent id/profile.
2. Runtime resolves an `AgentInstance` with `agent_id`, profile overlay, workspace, state dir, session policy, auth scope, and route bindings.
3. Runtime builds stores, provider adapters, the skill registry, context engine, and harness session.
4. Model turns run through `AgentRuntime`; the default implementation is `HarnessAgentRuntime`.
   Runtime emits typed `AgentEvent` records. Callers may attach an
   `AgentEventSink` to persist those records in `ikaros-session`, while existing
   CLI and worker callers can still use the final report.
5. Tool dispatch must go through `ExecutionSession` and `ExecutionEnv`; runtime code should not touch host APIs directly.
6. The harness evaluates policy, records audit events, and either executes, asks for approval, or denies.
7. Runtime reduces the same turn path into stable reports for CLI, body,
   schedule, gateway, chat, or agent callers.

Chat, task execution, scheduled jobs, gateway drains, and agent handoffs reuse this path.

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
  FTS5/trigram search indexes, branch/compact/retry markers, and replay data.
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
  Runtime chat records `sync_turn` append/skipped-write decisions, then applies
  configured promote/demote/forget/quota policy actions to affected scopes and
  records those decisions in the same journal.
- Relationship memory is `MemoryKind::Relationship` in `ikaros-memory`; the
  relationship CLI is a convenience façade over the memory store, not a second
  memory system.
- Tool execution belongs to the harness and `ExecutionEnv`, not the model provider or UI.
- Gateway protocol types live inside `ikaros-gateway`; there is no separate protocol crate.
- Self-modification is a separate approval-gated path, not an ordinary write permission.

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
