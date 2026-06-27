# Architecture

Ikaros is a persona-first local agent workspace. The core boundary is: agent
use cases orchestrate turns, execution governs tool calls, provider adapters
only handle external service wire formats, and local state stays under
`IKAROS_HOME` by default.

This document describes the runtime contract. It is not a module inventory.
When code moves between files, this page should change only if ownership,
calling context, persistent state, or user-visible behavior changes.

## Terms

- Agent use case: code in `ikaros-agent` that drives chat, tasks, schedules,
  gateway drain, body frames, and agent-loop reports for one command or worker
  tick.
- Host assembly: the boundary that loads config, resolves `AgentInstance`,
  builds `RuntimeLocation`, and wires `ExecutionSession`, sandbox-backed
  `ExecutionEnv`, and `SkillRegistry` for runtime callers.
- Harness: the policy, approval, audit, and execution-session boundary for
  every governed tool call.
- Tool contracts: the shared skill registry, tool descriptor, process,
  filesystem, network, and audit shapes from `ikaros-execution::toolkit`.
- Sandbox: the concrete local, dry-run, Docker, workspace, and governed network
  execution backends from `ikaros-execution::sandbox`.
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

- `ikaros-core`: shared config, paths, task types, redaction, errors, agent
  profiles, persona/emotion primitives, and pure agent config resolution.
- `ikaros-protocol`: stable versioned wire and session protocol types shared by
  CLI, TUI, gateway, local API, replay, and external integration surfaces.
- `ikaros-state`: durable local state owner for session timelines, memory, RAG
  indexes, automation metadata, and gateway queues.
- `ikaros-execution`: governed execution owner for policy, approvals, audit,
  sandbox, process/filesystem/network execution, reusable tool contracts,
  coding primitives, and task/plugin dispatch.
- `ikaros-providers`: provider owner for model, embedding, voice, vision/image,
  web, registry, and governance adapters.
- `ikaros-host`: composition root for config/path/agent instance/provider/store
  setup, execution environment wiring, diagnostics, persona file management,
  approval resolution, and host-owned adapters.
- `ikaros-agent`: application use cases for chat, context assembly, coding
  workflow, task loop, schedules, gateway drain, body status, agent loop,
  agent handoff, and session runner logic. It receives assembled dependencies
  rather than loading config or resolving host resources itself.
- `ikaros-skills`: built-in skill implementations exposed through shared tool
  contracts and governed by execution policy. Organize skills internally by
  groups or packs until a real reuse boundary justifies another crate.
- `ikaros-surfaces`: external entry points: local API, MCP, gateway adapters,
  webhooks, body dashboard, and service-manager integration. It must not expose
  local state stores.
- `ikaros-terminal`: terminal/TUI screen models, input, rendering, slash
  commands, status, body rendering, and timelines. It must not own execution,
  provider setup, config loading, or local stores.
- `ikaros-cli`: thin `clap`, dispatch, and terminal adapter for the `ikaros`
  binary. It should parse flags, call host/agent/surfaces/terminal APIs, and
  render terminal output; it should not own host assembly, gateway queue logic,
  or protocol schemas.

The current Cargo workspace contains the 11 architecture crates above. Former
implementation crates have been folded into these architecture crates or
removed: `ikaros-api`, `ikaros-automation`, `ikaros-body`, `ikaros-coding`,
`ikaros-context`, `ikaros-gateway`, `ikaros-harness`, `ikaros-mcp`,
`ikaros-memory`, `ikaros-models`, `ikaros-rag`, `ikaros-runtime`,
`ikaros-sandbox`, `ikaros-service`, `ikaros-session`, `ikaros-soul`,
`ikaros-toolkit`, `ikaros-tui`, and `ikaros-voice`. They are no longer
workspace members.

## Architecture Invariants And Open Hardening

Use these rules when hardening or moving code across crate seams:

- Keep `ikaros-cli` as the thin command and terminal adapter. Move reusable
  command behavior into agent, surfaces, terminal, host, or execution crates
  instead of growing CLI modules.
- Route API server code through `ikaros-surfaces`; do not let it become a
  second host assembly layer or own CLI, terminal, or gateway queue behavior.
- Route terminal render/input code through `ikaros-terminal`; do not let
  terminal UI code assemble runtime dependencies or drain gateway queues.
- Keep stable wire/session/event shapes in `ikaros-protocol` before exposing
  them as shared API, terminal, replay, MCP, gateway, or adapter contracts.
- Keep gateway queue stores and leases in `ikaros-state::gateway`, route
  adapter/webhook APIs through `ikaros-surfaces::gateway`, and keep the deleted
  legacy gateway crate out of the workspace.
- Keep application use cases in `ikaros-agent` over dependencies assembled by
  `ikaros-host`, CLI, or surfaces. Agent code must not load config, resolve
  agents, build registries, or select stores.
- Keep `ikaros-host` focused on environment, registry, provider, sandbox, and
  store assembly. Host should not own agent-loop behavior or terminal rendering.
- Keep `ikaros-skills` as one crate with internal groups or packs until a real
  reuse seam justifies another crate.
- Keep `ikaros-protocol` internally modular as protocol families grow, but keep
  provider clients, store implementations, agent behavior, and CLI details out
  of it.
- Treat remaining CLI thinning, surface dependency narrowing, terminal
  hardening, provider/context/sandbox hardening, and acceptance-smoke coverage
  as open hardening work against the current 11-crate graph, not as migration
  work from deleted crates.

## Agent Flow

Most entry points follow the same path:

1. CLI or workers resolve `IKAROS_HOME`, workspace, and the requested agent id.
2. Host assembly loads config and resolves an `AgentInstance` with `agent_id`,
   profile overlay, workspace, state dir, session policy, auth scope, and route
   bindings.
3. Host assembly builds the harness session, execution environment, skill
   environment, skill registry, and runtime location. Agent code then builds
   the turn-specific provider, context, and session writer pieces.
4. Model turns run through `AgentRuntime`; the default implementation is
   `HarnessAgentRuntime`. Chat and task agent-loop entry points wrap it in
   `AgentHarness`, which owns phase, caller-provided turn ids, and durable
   continuation queue handling when a `SessionStore` is available. Gateway task
   drains, scheduled task execution, and agent-loop handoff now call the
   session-aware task agent-loop path with explicit session id, turn id, and
   source metadata. Agent execution emits typed `AgentEvent` records. Callers may attach
   an `AgentEventSink` to persist those records in `ikaros-state::session`, while
   existing CLI and worker callers can still use the final report.
5. Tool dispatch must go through `ExecutionSession` and the attached
   `ExecutionEnv`. Agent code should not touch host APIs directly, bypass
   `ikaros-execution::toolkit`, or reimplement host assembly.
6. The harness evaluates policy, records audit events, and either executes,
   asks for approval, or denies.
7. Agent use cases reduce the same turn path into stable reports for CLI, body,
   schedule, gateway, chat, or agent callers.

Chat and task agent-loop execution now use the stateful harness path. Gateway
task drain, scheduled task execution, and agent-loop handoff also enter that path
with explicit session source metadata, so their agent-loop events and
continuation state can land in the same `state.db` timeline as their
gateway/schedule evidence.
The durable continuation queue is a recovery and replay boundary, not yet a
full scheduler. It now records leases, attempt counts, status reasons, requeue
status, terminal status, cancellation request/acknowledgement evidence,
worker-lease timeout summaries, first-pass recoverable tool-result
continuations, and user-facing debug query data. Running durable message
continuations poll for external cancellation, but configurable worker
coordination, richer tool-result scheduling policy, and scheduler-grade
terminal accounting are still agent-loop hardening work.

## Agent Identity

Profiles and instances are intentionally separate.

Profiles describe how an agent should behave: mode, persona overlay, context
sources, and ordinary policy defaults. Instances describe who is running and
where state belongs. A configured instance may select a profile while providing
its own workspace, state directory, toolset allowlist, model/provider override,
session policy, auth scope, and route bindings.

Resolution order:

1. If the requested name matches `agent.instances.<id>`, host assembly creates an
   `AgentInstance` from that entry.
2. If no instance matches, the name is resolved as an agent profile.
3. If no name is passed, `agent.default` is used.
4. If the default profile is missing, host assembly falls back to the built-in
   `build` profile.

Callers should pass the resolved `AgentInstance` into harness sessions. Policy
overlays and approval replay use the instance identity; persona text alone must
not grant permissions.

## Local State

Default state lives under `~/.ikaros` or `IKAROS_HOME`.

JSONL remains available for simple local stores because it is inspectable and
easy to recover. SQLite is the authoritative store for session timelines and is
available for larger local stores such as memory and RAG indexes. Remote
services are not required for the MVP.

State ownership:

- `state.db`: session metadata, append-only session entries, persisted
  chat/agent-loop events, gateway and schedule evidence, approval records,
  durable continuation queue records, FTS5/trigram search indexes,
  branch/compact/retry markers, coding turn events, and replay data.
  Built-in chat turns write user/assistant entries through a turn-scoped
  `SessionWriter` transaction. Gateway and schedule workers also map their
  request/result/delivery evidence into the same store. Memory lifecycle and
  audit logs still keep their own stores, with selected session evidence rather
  than full prompt-bearing duplication. Ordinary chat does not write a separate
  history mirror; session replay is the chat timeline. Operational helpers can
  report journal mode, integrity, WAL checkpoint state, search-index
  availability, and write policy; debug commands can checkpoint WAL, back up or
  repair an artifact, prune ended sessions, and vacuum the database.
- `memory/`: local memory records, memory policy journal data, and memory
  provider registry metadata.
- Chat history, search, summaries, replay, and workbench timelines are
  projections from `state.db`; ordinary chat turns do not own a separate
  `chat/` store.
- `rag/`: local RAG files, chunks, and embedding indexes.
- `audit/`: policy decisions, approval records, usage logs, rotation archives,
  and forensic evidence.
- `automation/`: schedule metadata and delivery reports.
- `gateway/`: inbox/outbox records, worker lease/retry/dead-letter metadata,
  and sibling lock files for local message routing.
- `browser/`: local browser supervisor profiles, launch state, and browser
  runtime metadata.
- `logs/trace.jsonl`: structured tracing events for CLI, API, and local
  diagnostics.
- `skills/`: locally installed plugins and marketplace metadata.
- `agents/`: per-agent state directories when instances use the default state root.

## Boundaries

- Persona affects prompts and context, not policy.
- Agent profiles are persona/policy overlays; `AgentInstance` is the runtime identity and may
  override toolsets plus model/provider settings for chat, TUI, coding, task agent-loop, doctor, and
  provider inspection paths.
- `ModelProvider` generates/streams model output; `ModelTransport` describes provider wire format;
  `ProviderRegistry` resolves local descriptor metadata for inspection and planning;
  `ModelStreamEvent` normalizes provider deltas; `AgentRuntime` owns the turn loop and emits
  `AgentEvent`.
- OpenAI-compatible provider quirks resolve through a static `ProviderProfile`
  spec catalog. The registry and request builder share that decision for output
  defaults, context metadata, temperature/reasoning/message/tool-schema policy,
  extra request-body behavior, and prompt-cache policy. Anthropic maps prompt
  cache read/write usage into `TokenUsage` so status and audit views can explain
  cache accounting separately from ordinary input/output tokens.
- `AgentEvent`, session ids, turn ids, append-only session entries, and replay
  reads belong to `ikaros-state::session`, not to the runtime loop.
- `ikaros-protocol` owns durable wire shapes for API, TUI, gateway, replay, and
  external integration surfaces. `ikaros-agent`, `ikaros-state`, and
  `ikaros-providers` code may project into those shapes, but product surfaces
  should not invent incompatible event or state schemas.
- `SessionWriter` owns turn-scoped session transactions. Built-in chat uses it
  for session entries and typed events. Gateway and schedule workers write
  high-level evidence entries/events into `state.db`. Memory and audit remain
  separate stores, with session evidence kept explicit and redacted. Ordinary
  chat does not write a separate history mirror; session replay is the chat
  timeline.
- `AgentEventSink` is the event-bus boundary. `ikaros-state::session` provides
  no-op, collecting, fan-out, per-event persisting, and turn-transaction
  persisting sinks, so agent code can emit one typed event stream to
  persistence, replay/test collectors, UI observers, metrics, or plugin
  observers without reimplementing callback fan-out in each caller.
- Host assembly belongs to `ikaros-host`. New local entry points should reuse
  `RuntimeHarness` or `HostServices` instead of rebuilding agent resolution,
  workspace scope, skill registry, provider egress allowlists, or execution
  environment composition in CLI or agent modules.
- Gateway workers claim messages with a redacted lease owner, lease expiry, and
  attempt count. Failed processing clears the lease and either requeues the
  message or moves it to `DeadLettered` after the retry budget.
- Built-in chat turns commit session entries and chat events together. Failed
  provider or local post-processing turns keep the user entry, a redacted error
  event, and a failed turn-end event for replay/debug callers.
- `session_id` identifies persisted timelines. `task_id` is task/report
  metadata and must not be used as an implicit session fallback.
- Context primitives live in `ikaros-protocol::context`. Agent chat assembles
  relationship, explicit references, history, memory, and RAG into a
  provider-aware token-budgeted `ContextBundle`. Provider profiles and registry
  metadata cap the usable context window and select the token estimator.
  OpenAI-compatible and mock providers have deterministic local adapters;
  Anthropic and Ollama still use explicit fallback adapters until exact native
  tokenizer libraries are wired in. Agent chat resolves engine selection
  through `ContextEngineRegistry`, which currently exposes a deterministic local
  compressor descriptor and an `llm-summary` descriptor. The `llm-summary`
  engine builds a redacted provider-backed summary request and turns the
  provider summary into agent compaction evidence; deeper semantic compression
  quality and fallback policy remain hardening work.
- Prompt assembly also uses `ikaros-protocol::context`. Agent chat turns the context
  bundle, persona, policy, compression notice, and tool guidance into typed
  `PromptSection` records before rendering the final system prompt. `ContextDiff`
  persists only `PromptSectionMetadata` for replay/debug/UI callers: kind, title,
  source, priority, estimated token count, and redaction state. Full prompt
  section content remains an in-memory render input and is not stored as session
  evidence.
- `ContextReference` currently resolves safe local references:
  `@file:path:line-line`, `@folder:path`, `@git:rev`, `@diff`, and `@staged`.
  Paths must stay under the workspace. `@url:` is fetched through the session
  `NetworkEgress` boundary and obeys the configured exact-host allowlist.
- Chat attachments are model content blocks, not out-of-band files. CLI
  `--image`, `--audio`, `--file`, and workbench `/attach` resolve local paths
  under the workspace into bounded data URLs; URL and data-URL attachments are
  passed through as provider content when the selected provider supports them.
- Context assembly emits a `ContextDiff` agent event for the turn. The payload
  includes the budget, context sections, prompt section metadata, parsed
  references, and added/removed/compressed token estimates.
- Context compaction protects relationship facts and explicit references. If
  protected context cannot fit the model-derived budget, the turn fails with a
  context-limit error instead of silently dropping the requested context.
- `MemoryProvider` exposes turn_start, prefetch, sync_turn, pre_compress, session_switch, and
  delegation_observation lifecycle hooks. The trait does not hide default noop methods; callers that
  need no memory side effect must choose `NoopMemoryProvider` explicitly.
- `MemoryScore`, `MemoryPolicy`, and `MemoryJournal` belong to `ikaros-state::memory`.
  Agent chat records `sync_turn` working-memory append/skipped-write
  decisions. When a lifecycle report references affected core memory scopes, it
  applies configured promote/demote/forget/quota policy actions and records
  those decisions in the same journal. Ordinary turn summaries are not promoted
  into long-term `Task` memory.
- Relationship memory is `MemoryKind::Relationship` in `ikaros-state::memory`; the
  relationship CLI is a convenience façade over the memory store, not a second
  memory system.
- Tool governance belongs to `ikaros-execution::harness`, reusable tool
  contracts belong to `ikaros-execution::toolkit`, and concrete
  filesystem/process/network adapters belong to `ikaros-execution::sandbox`.
  Model providers and UI layers must not execute tools
  directly. `ikaros-host` composes the configured execution environment from the
  sandbox backends. Process execution clears the ambient environment, restores a
  small baseline allowlist, applies explicit request env, and redacts sensitive
  env diagnostics. The sandbox debug report explains the current
  dry-run/workspace/network-restricted matrix and the Docker-backed container
  first slice when configured. That container backend runs process execution
  through `docker run --network none`, but it is not a VM, multi-tenant
  boundary, or complete OS sandbox.
- Local API, MCP, browser/CDP, web, vision, and image-generation surfaces are
  adapters over agent, execution, session, and provider boundaries. They must
  reuse `NetworkEgress`, `ExecutionEnv`, provider governance, audit, and
  session evidence instead of opening side channels around policy.
- Browser CDP HTTP discovery goes through governed `NetworkEgress`, while the
  browser process performs page network I/O until a stricter browser supervisor
  sandbox exists. Docs and UI must describe that distinction clearly.
- Web search and extract are explicit governed skills. Search may use the
  built-in DuckDuckGo HTML provider or configured Brave, Bing, SerpAPI, and
  Tavily-compatible endpoints; extract fetches one URL and returns bounded,
  redacted citation text.
- Coding workflow execution is a governed skill. It builds a
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
- Stable product-facing protocol types live in `ikaros-protocol`; gateway-local
  inbox/outbox storage lives in `ikaros-state::gateway`, while external
  adapter/webhook surfaces live in `ikaros-surfaces::gateway`.
- Self-modification is a separate approval-gated path, not an ordinary write permission.
- The current coding workflow is now a provider-backed controlled loop, but it
  is still pre-MVP. It has deterministic, mock-model, and provider-loop replay
  fixtures, multi-iteration patch/test/review evidence, test-matrix events, and
  parser hardening for malformed ranges, quoted/space-truncated paths,
  ambiguous anchors, already-applied hunks, generated malformed corpus cases,
  and generated line-update roundtrips. Terminal-first coding commands are
  available from both `ikaros code ...` and the workbench `/code ...` command,
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

Most agent paths return structured reports for internal callers and render a
human-oriented CLI summary. Reports should include enough information to explain
why work stopped without storing prompt text or secrets. Common stop conditions
are policy denial, waiting for approval, iteration budget, guardrail halt,
provider error, command timeout, and local store errors.

Session replay is currently strongest for completed and failed chat/agent-loop
turns, and it also contains high-level gateway and schedule request/result/
delivery evidence. Memory and audit still have dedicated stores, so long-running
workers should treat `state.db` as the primary timeline and those stores as
supporting evidence until their lifecycle records are fully modeled.
