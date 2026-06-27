# Roadmap

This roadmap describes planned work for Ikaros. It is a planning document, not
a changelog.

## Current Focus

Ikaros is still pre-MVP. The runtime, session store, context engine, memory
model, RAG, coding workflow, provider layer, gateway, approvals, and audit
boundaries exist, but they are not yet tied together as one finished product.

The current product direction is terminal-first. A user should be able to start
from `ikaros`, inspect what the agent sees, approve or deny risky actions, run a
coding workflow, replay a failed turn, and understand the provider/runtime state
without reading raw JSON or separate debug files.

Recent code work is also moving large CLI and runtime files into smaller
modules. Keep that direction: split by responsibility first, and only create a
new crate when the boundary is stable enough to reuse outside its current
caller.
The new `ikaros-host` crate follows that rule: it owns host-side runtime
assembly for agent instances, locations, execution environments, skill
registries, and provider/search egress allowlists.

## What Exists Today

- `ikaros`, `ikaros tui`, `ikaros workbench`, and `ikaros chat` without
  `--message` share the same terminal workbench entry point.
- The workbench can show session, provider, context, memory, RAG, tool, approval,
  queue, timeline, replay, trace, and coding status.
- `ikaros code ...` and workbench `/code ...` route into the same governed
  coding workflow instead of maintaining two separate coding paths.
- `state.db` is the durable source for session entries, typed turn events,
  approvals, replay, search, coding turn evidence, and continuation state.
- `ikaros-host` centralizes config-driven runtime assembly instead of requiring
  each CLI, chat, gateway, or task entry point to rebuild sessions and
  registries independently.
- The first local API, MCP, browser/CDP, web search/extract, vision,
  image-generation, and multimodal attachment surfaces are now part of the
  official project surface.
- Early packaging scaffolds exist for Docker, shell/PowerShell install scripts,
  Nix, Arch, and future cargo-dist release artifacts.

These pieces are useful for development, but they are not enough to call the
project a complete TUI, sandbox, scheduler, gateway daemon, or multi-platform
agent product.

## Next Milestones

1. Finish M6.0 Terminal Workbench.
   Make the terminal workbench the normal way to use Ikaros locally. Harden
   redraw behavior, overlays, key bindings, multiline editing, selected actions,
   session resume, timeline paging, approval handling, cancellation, mouse
   input, quiet stdout routing, and coding workflow controls. Add a CI-safe
   non-TTY strategy or a PTY-backed smoke harness so terminal behavior can be
   tested reliably.
2. Finish M6.1 Provider, Context, Sandbox, And Debug Hardening.
   Complete provider cost/capability metadata, retry and fallback diagnostics,
   prompt-prefix stability tests, context compression diagnostics, SQLite export
   and integrity checks, stronger `ExecutionEnv` isolation, and structured
   debug/trace output.
3. Promote The Gateway Worker Into A Real Daemon.
   Build on the current worker locking, cooperative stop/restart, webhook
   ingestion, pairing, leases, retries, and dead-letter state. External
   adapters should feed governed inboxes and reuse the same runtime, harness,
   approval, session, and replay paths.
4. Harden MCP, Plugins, And API Surface.
   Keep MCP and plugin entry points behind the same command execution, approval,
   audit, session evidence, and network egress rules as built-in skills. Harden
   local API image/audio routes, browser/CDP controls, web provider configuration,
   and multimodal attachment capability checks.
5. Revisit Broader Capability Pools.
   Browser/computer use, richer web search, voice, multimodal input, subagents,
   self-improvement, desktop/web UI, and broader automation should come after
   the terminal workbench and runtime hardening are dependable.

## MVP Release Gates

- `cargo fmt`, clippy, tests, and documentation checks should pass through direct
  local commands.
- README files should describe the product surface. Subsystem details belong in
  `docs/en` and `docs/zh-CN`.
- Markdown should be readable by humans: short paragraphs, categorized command
  lists, stable headings, and no protocol dumps in overview pages.
- `config.yaml` validation should stay current as provider, memory, voice, MCP,
  and runtime fields change.
- Provider compatibility needs fixture coverage and a live smoke matrix for the
  supported OpenAI-compatible, Anthropic-compatible, Ollama, and local provider
  paths.
- Provider registry, live health probes, cooldown state, fallback-chain
  primitives, and capability regression tests should stay independent from the
  agent turn loop.
- Agent-loop fallback parsing should stay limited to provider-native tool calls
  plus the strict documented JSON fallback.
- Policy and approval tests should cover path handling, secret-looking inputs,
  governed network egress, plugin execution, and approval replay.
- Approval replay should remain bound to workspace, exact approved input, and
  agent identity.
- Command-backed plugin tests should cover malicious manifests, path traversal,
  stdin/output limits, timeout limits, and output redaction.
- The terminal workbench should cover session resume, timeline/replay/trace,
  provider/gateway/task status, approval overlays, context/memory/RAG status,
  coding workflow commands, multiline input, queued input, input history, paged
  timelines, inline approval decisions, and live turn cells.
- `doctor --fix` should report migrations, backups, and failures clearly.
- The chat timeline should continue to derive from `SqliteSessionStore`; do not
  reintroduce a separate durable `ChatHistoryStore` backend.

## Runtime

- Harden `AgentRuntime` so future runtimes can plug in without changing provider adapters.
- Keep the stateful `AgentHarness` as the shared path for chat and task
  agent-loop turns, including gateway task drain, scheduled task execution, and
  agent-loop handoff.
- Keep harness branch-summary, compaction, and retry marker helpers append-only
  through `SessionStore`; do not reintroduce history rewriting for these phase
  operations.
- Harden durable `AgentHarness` continuations in `state.db`: queue claiming,
  resume/compact/retry execution, lease expiry/reclaim, attempt tracking,
  failed/cancelled requeue, status reasons, terminal status reporting,
  cancellation evidence, and replay/debug queries should stay consistent across
  process restarts.
- Keep first-pass durable cancellation polling and timeout reporting covered:
  running workers should observe external cancellation, provider waits should
  honor cancellation tokens, descriptor timeouts should report structured
  timeout payloads, and debug output should explain worker lease expiry.
- Do not treat the continuation queue as a full scheduler yet. First-pass
  tool-result continuations are modeled, but configurable polling/backoff,
  worker coordination, scheduler-grade terminal accounting, and richer
  automation-facing timeout reports are still future hardening.
- Keep observer hooks stable as the extension boundary for provider attempts,
  tool lifecycle telemetry, memory policy observation, gateway UI, and replay
  diagnostics.
- Keep governed provider chain representation clear. `provider inspect` and
  `provider matrix` should reflect the active `AgentInstance` model/provider
  override, the resolved profile policy, configured `fallbacks`, and any
  diagnostics the governance wrapper emits when retries or failovers occur.
- Keep model-provider diagnostics durable without leaking prompt or secret
  fragments. `ModelDiagnostic` events should be redacted before entering the
  session timeline, and replay/debug surfaces should expose them as
  retry/failover metadata.
- Keep agent `toolsets` bounded by the harness bridge. Direct toolset skills are
  model-visible; `rag`, `coding`, `voice`, and `plugin` skills must use
  `tool_search` / `tool_describe` for progressive disclosure. Profiles that
  enable deferred toolsets must retain `core` so the bridge stays available.
- Keep `ikaros-session` as the runtime fact source for chat, gateway,
  schedule, approvals, replay, search, and branch navigation.
- Expand session evidence for memory and audit lifecycle boundaries without
  duplicating full prompt or secret-bearing audit payloads.
- Move more cross-store writes toward turn-scoped transactions where rollback
  semantics matter.
- Extend durable failed-turn timelines for memory and audit lifecycle flows that
  still live partly outside `state.db`.
- Derive more runtime reports from persisted event streams rather than carrying
  separate one-off summaries.
- Refine continuation and tool timeout report fields for automation users beyond
  the current worker-lease and descriptor-timeout summaries.
- Extend durable cancellation beyond the current polling/checkpoint model toward
  configurable worker coordination and scheduler-grade terminal accounting.
- Continue separating provider transport concerns from turn-loop ownership.
- Keep provider request quirks inside model adapters and compatibility profiles,
  not in the runtime turn loop.
- Add stricter compatibility tests for provider-specific tool-call differences.
- Migrate provider HTTP clients and future network-capable skills through the
  `NetworkEgress` boundary so allowlists, audit, cancellation, and redaction are
  enforced consistently.
- Keep provider-backed RAG embedding skills on the same `NetworkEgress`
  boundary after approval; do not let OpenAI-compatible or Ollama embedding HTTP
  bypass session egress.
- Keep `@url` context references on the same governed `NetworkEgress` boundary
  and do not let context fetching bypass exact-host allowlists.
- Preserve gateway session continuity with digest-derived session ids and keep
  raw channel/account/peer/thread/message identifiers only as redacted source
  evidence.

## Context And Memory

- Keep local-first memory as the default behavior; keep RAG as opt-in cited
  reference retrieval rather than ordinary chat memory.
- Keep `ikaros-context` as the shared boundary for context bundles, references,
  provider-aware token budgets, quota-based compaction, and context diffs.
- Keep `ModelContextProfile` wired into context budgeting and estimator
  selection. Extend the current deterministic adapters with exact
  provider-native tokenizer libraries using provider registry metadata.
- Extend quota-based context assembly with dynamic priority, semantic
  compression, and stricter long-running session diagnostics beyond the current
  `context-diff` debug query.
- Keep relationship data as `MemoryKind::Relationship`, not as a second memory
  database.
- Keep accepted memory projections, session working memory, retrieved memory,
  and RAG as separate context sections with trust/source metadata.
- Keep ordinary `sync_turn` summaries out of long-term `Task` memory; they
  should remain session working memory unless promoted through an explicit
  candidate path.
- Keep automatic relationship observations as pending candidates until an
  explicit accept path promotes them to core memory.
- Keep supersession metadata (`active`, `supersedes`, `superseded_by`,
  `valid_from`, `valid_until`) as the update path for durable memory conflicts.
- Keep `NoopMemoryProvider` explicit; memory lifecycle hooks should not hide
  default no-op behavior in the trait.
- Keep runtime memory journaling covered for working-memory append,
  skipped-write, candidate accept/reject, projection render, supersession,
  working-memory expiry, promote, demote, forget, and quota decisions as memory
  extraction grows.
- Treat the current memory policy pass as turn-scoped; add cross-store
  transaction and replay consistency before enabling remote or vector writes.
- Add debug/query surfaces that can explain why a projection changed, why a
  candidate was accepted or rejected, and which record superseded an older fact.
- Define governed remote memory adapters behind the provider registry.
- Require remote memory behavior to match local approval, audit, promotion,
  demotion, sync, and secret-handling rules.
- Add dry-run reports for memory migration or synchronization.
- Exercise session-switch, delegation-observation, and pre-compression lifecycle hooks with real
  provider tests before enabling remote writes.

## Coding Agent

- Keep `code workflow` as the controlled coding turn surface: context
  preparation, repo scan, planning, optional guarded patch application, turn
  diff tracking, test evidence, review, iteration plan, final report, and
  persisted `CodingTurn` replay evidence.
- Keep coding modes explicit. `plan` and `review` remain read-oriented; `test`
  requires shell policy for the test matrix; `edit` is the only ordinary
  workflow mode that may apply a candidate patch. `self_modify` must stay on the
  dedicated self-modify approval path, not ordinary `code workflow`.
- Keep the coding instruction boundary explicit. The current context records
  HEAD, branch/detached state, clean/dirty/not-git/unknown state, staged/
  unstaged/untracked flags, user-provided instructions, and workspace
  instruction files from `IKAROS.md` and `.ikaros/instructions.md`.
- Keep hardening patch handling beyond add/update/delete/move: the current
  parser rejects malformed hunk ranges, quoted/space-truncated paths, ambiguous
  anchors, already-applied hunks, generated malformed corpus cases, generated
  line-update roundtrips, and no-mutation malformed diffs; broader fuzzing can
  continue as the parser surface grows.
- Keep the provider-backed loop replayable. `code workflow --model-loop` now
  uses the configured model provider to produce strict JSON candidate patches,
  applies approved patches through `ExecutionEnv`, feeds test evidence into
  follow-up model requests, and records model request/response, budget,
  cancellation, patch, test, review, and loop-stop events in the session
  timeline.
- Keep terminal-first coding commands as thin routes into that workflow:
  `code plan`, `code apply`, `code test`, `code review`, and `code rollback`
  share approval behavior, test evidence, turn diff tracking, and persisted
  replay. Rollback reconstructs the reverse diff from a prior turn's durable
  `diff_updated` event and submits it as a new approved edit turn. The workbench
  `/code ...` command now routes to the same commands instead of introducing a
  second control plane.
- Route coding git status snapshots through the session process boundary. The
  sync context constructor only consumes local fixture state or returns unknown;
  live workflow execution uses `ExecutionEnv` / `ProcessRunner`.
- Keep the coding terminal surface explainable. Coding approval requests now
  include a structured approval context for provider calls, workspace writes,
  shell/test commands, session/turn identity, and replay instructions. `ikaros
  code ...`, workbench `/code ...`, and `ikaros approval approve ...` render
  `approval_scope`, `coding_progress`, and `coding_result` lines in addition to
  the JSON payload.
- Provider-backed coding turns can be cancelled while awaiting the provider:
  Ctrl-C requests cancellation, the provider future is dropped, and the coding
  timeline records `coding_loop_cancelled` before patch/test execution resumes.
- Future work should focus on readline/history/multiline interaction, broader
  slash-command resume/status ergonomics, and deeper property/fuzz coverage
  rather than adding a second coding control plane.
- Keep coding timeline grouping stable. Diff/test/review/progress cells should
  remain visible through workbench timeline/replay filters, or the documented
  grouping contract must be changed together with the smoke tests.
- Keep `debug coding-turn` aligned with the session timeline so coding replay
  can be inspected without reading ad hoc report files.

## Gateway And Automation

- Evolve the local gateway worker into a long-running daemon with device pairing, capabilities,
  multi-channel routing, and session continuity across channel threads.
- Keep `GatewayProtocolPolicy` checks for protocol version, client allowlists,
  channel allowlists, and required capabilities in front of future daemon
  adapters.
- Keep gateway JSONL queue mutations behind portable file locks so local workers
  and external adapters do not corrupt inbox/outbox state.
- Add external message adapters that only write into the governed local gateway inbox.
- Broaden schedule delivery targets after external adapters have clear routing,
  audit behavior, and replay evidence.
- Keep schedules as work requests; execution should continue to pass through
  runtime and harness boundaries.

## Execution Environment

- Preserve the workspace-scoped local environment as the baseline contract:
  reads, writes, removals, directory listing/creation, and process cwd stay under the session
  workspace, including symlink escape checks.
- Keep provider HTTP, future network-capable tools, and local probes routed
  through governed `NetworkEgress`; shell/test commands remain structured and
  should not be the network escape hatch.
- Define isolation levels and mount rules before adding Docker and ssh
  execution backends. Dry-run is the first non-side-effect backend.
- Route file, process, network, plugin, shell, test, and coding helpers through
  the environment abstraction.

## Plugins And Skills

- Keep executable tools separate from prompt skills and skill bundles.
- Expand plugin validation for manifest shape, command metadata, canonical paths, resource limits,
  and marketplace metadata.
- Add richer plugin protocols only after command-backed plugins have stable
  execution and audit behavior.

## Product Surface

- Keep the terminal workbench as the primary pre-MVP user surface. The baseline
  now includes `ikaros`, `ikaros tui`, raw-mode input, multiline editing,
  mouse-wheel scrolling, command palette, navigable live cells, approval
  overlays, quiet routing for read-only/status commands, cancellation, session
  resume, and `/code ...` workflow controls. Remaining work is hardening:
  PTY-backed smoke tests, better resize behavior, clearer recovery prompts,
  auditing any remaining protocol lines in human mode, and stable structured
  models for ACP or future GUI consumers.
- Improve voice and body integration beyond provider/status contracts.
- Add optional remote sync only after local state, audit, and conflict behavior are well defined.
- Define remote or distributed subagent worker boundaries before introducing multi-node execution.
