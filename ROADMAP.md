# Roadmap

This roadmap describes planned work for Ikaros and is scoped as future planning rather than project history.

## Before MVP Release

- Keep formatting, clippy, tests, and generated docs passing through direct local commands.
- Keep agent-loop fallback parsing limited to provider-native tool calls plus the strict documented JSON fallback.
- Keep `config.yaml` validation current as provider, memory, voice, and runtime fields change.
- Keep provider-profile compatibility covered with fixture tests and a live smoke matrix for Moonshot/Kimi, DeepSeek, Gemini OpenAI-compatible, OpenRouter, Qwen/DashScope, SiliconFlow, and local OpenAI-compatible servers.
- Keep provider registry, live health probes, cooldown state, fallback-chain
  primitives, and provider capability regression tests independent from the
  agent turn loop.
- Expand provider stream fixture tests for typed text, reasoning, refusal, tool-call, usage, error, done, and true network-incremental behavior.
- Expand policy and approval tests for path handling, secret-looking inputs,
  governed network egress, plugin execution, and approval replay.
- Keep approval replay bound to workspace, exact approved input, and agent identity.
- Keep command-backed plugin tests focused on malicious manifests, path traversal, stdin/output limits, timeout limits, and output redaction.
- Add clearer migration and backup reports for `doctor --fix`.
- Keep README concise and move subsystem design details into language-scoped documentation.

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
- Do not treat the continuation queue as a full scheduler yet; configurable
  polling/backoff, tool-result continuations, and richer automation-facing
  timeout reports are still future hardening.
- Keep observer hooks stable as the extension boundary for provider attempts,
  tool lifecycle telemetry, memory policy observation, gateway UI, and replay
  diagnostics.
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
- Require remote memory behavior to match local approval, audit, promotion, demotion, sync, and secret-handling rules.
- Add dry-run reports for memory migration or synchronization.
- Exercise session-switch, delegation-observation, and pre-compression lifecycle hooks with real provider tests before enabling remote writes.

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
  `diff_updated` event and submits it as a new approved edit turn. The chat REPL
  `/code ...` wrapper now routes to the same commands instead of introducing a
  second control plane.
- Route coding git status snapshots through the session process boundary. The
  sync context constructor only consumes local fixture state or returns unknown;
  live workflow execution uses `ExecutionEnv` / `ProcessRunner`.
- Keep the coding terminal surface explainable. Coding approval requests now
  include a structured approval context for provider calls, workspace writes,
  shell/test commands, session/turn identity, and replay instructions. `ikaros
  code ...`, chat REPL `/code ...`, and `ikaros approval approve ...` render
  `approval_scope`, `coding_progress`, and `coding_result` lines in addition to
  the JSON payload.
- Provider-backed coding turns can be cancelled while awaiting the provider:
  Ctrl-C requests cancellation, the provider future is dropped, and the coding
  timeline records `coding_loop_cancelled` before patch/test execution resumes.
- Future work should focus on readline/history/multiline interaction, broader
  slash-command resume/status ergonomics, and deeper property/fuzz coverage
  rather than adding a second coding control plane.
- Keep `debug coding-turn` aligned with the session timeline so coding replay
  can be inspected without reading ad hoc report files.

## Gateway And Automation

- Evolve the local gateway worker into a long-running daemon with device pairing, capabilities, multi-channel routing, and session continuity across channel threads.
- Keep `GatewayProtocolPolicy` checks for protocol version, client allowlists,
  channel allowlists, and required capabilities in front of future daemon
  adapters.
- Keep gateway JSONL queue mutations behind portable file locks so local workers
  and external adapters do not corrupt inbox/outbox state.
- Add external message adapters that only write into the governed local gateway inbox.
- Broaden schedule delivery targets after external adapters have clear routing, audit behavior, and replay evidence.
- Keep schedules as work requests; execution should continue to pass through runtime and harness boundaries.

## Execution Environment

- Preserve the workspace-scoped local environment as the baseline contract:
  reads, writes, removals, directory listing/creation, and process cwd stay under the session
  workspace, including symlink escape checks.
- Keep provider HTTP, future network-capable tools, and local probes routed
  through governed `NetworkEgress`; shell/test commands remain structured and
  should not be the network escape hatch.
- Define isolation levels and mount rules before adding Docker and ssh
  execution backends. Dry-run is the first non-side-effect backend.
- Route file, process, network, plugin, shell, test, and coding helpers through the environment abstraction.

## Plugins And Skills

- Keep executable tools separate from prompt skills and skill bundles.
- Expand plugin validation for manifest shape, command metadata, canonical paths, resource limits, and marketplace metadata.
- Add richer plugin protocols only after command-backed plugins have stable execution and audit behavior.

## Product Surface

- Improve the terminal-first interactive experience beyond the current basic
  `ikaros chat` REPL and `/code ...` wrapper: readline/history, multiline
  input, cleaner streaming output, visible tool/approval/context/coding status,
  cancellation, and session resume should become the primary pre-MVP user
  surface before a web UI.
- Improve voice and body integration beyond provider/status contracts.
- Add optional remote sync only after local state, audit, and conflict behavior are well defined.
- Define remote or distributed subagent worker boundaries before introducing multi-node execution.
