# Roadmap

This roadmap describes planned work for Ikaros and is scoped as future planning rather than project history.

## Before MVP Release

- Keep formatting, clippy, tests, and generated docs passing through direct local commands.
- Keep agent-loop fallback parsing limited to provider-native tool calls plus the strict documented JSON fallback.
- Keep `config.yaml` validation current as provider, memory, voice, and runtime fields change.
- Keep provider-profile compatibility covered with fixture tests and a live smoke matrix for Moonshot/Kimi, DeepSeek, Gemini OpenAI-compatible, OpenRouter, Qwen/DashScope, SiliconFlow, and local OpenAI-compatible servers.
- Expand provider stream fixture tests for typed text, reasoning, refusal, tool-call, usage, error, done, and true network-incremental behavior.
- Expand policy and approval tests for path handling, secret-looking inputs, network calls, plugin execution, and approval replay.
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

## Context And Memory

- Keep local-first memory as the default behavior; keep RAG as opt-in cited
  reference retrieval rather than ordinary chat memory.
- Keep `ikaros-context` as the shared boundary for context bundles, references,
  provider-aware token budgets, quota-based compaction, and context diffs.
- Keep `ModelContextProfile` wired into context budgeting and estimator
  selection. Extend the current deterministic adapters with exact
  provider-native tokenizer libraries once the provider registry exists.
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
- Expand the first-pass `CodingTurnContext` into a full dirty-state and
  instruction boundary before relying on it for autonomous self-modification or
  long-running coding sessions. The current git baseline already records HEAD,
  branch/detached state, clean/dirty/not-git/unknown state, and
  staged/unstaged/untracked flags.
- Keep hardening patch handling beyond add/update/delete/move: the current
  parser rejects malformed hunk ranges, quoted/space-truncated paths, ambiguous
  anchors, already-applied hunks, and no-mutation malformed diffs; broader
  fuzzing can continue as the parser surface grows.
- Use the mock-model patch/test/review loop as the replay contract for the next
  real-provider coding loop. The current loop can replay multi-iteration
  scripted patches through session events, but provider-generated follow-up
  patches, cancellation, budget handling, and approval replay remain future
  hardening.
- Keep `debug coding-turn` aligned with the session timeline so coding replay
  can be inspected without reading ad hoc report files.

## Gateway And Automation

- Evolve the local gateway worker into a long-running daemon with device pairing, capabilities, multi-channel routing, and session continuity across channel threads.
- Keep gateway JSONL queue mutations behind portable file locks so local workers
  and external adapters do not corrupt inbox/outbox state.
- Add external message adapters that only write into the governed local gateway inbox.
- Broaden schedule delivery targets after external adapters have clear routing, audit behavior, and replay evidence.
- Keep schedules as work requests; execution should continue to pass through runtime and harness boundaries.

## Execution Environment

- Preserve the workspace-scoped local environment as the baseline contract:
  writes, removals, directory creation, and process cwd stay under the session
  workspace, including symlink escape checks.
- Define isolation levels, mount rules, and network-egress behavior before adding non-local execution backends.
- Add Docker, ssh, and dry-run `ExecutionEnv` backends after the isolation contract is testable.
- Route file, process, network, plugin, shell, test, and coding helpers through the environment abstraction.

## Plugins And Skills

- Keep executable tools separate from prompt skills and skill bundles.
- Expand plugin validation for manifest shape, command metadata, canonical paths, resource limits, and marketplace metadata.
- Add richer plugin protocols only after command-backed plugins have stable execution and audit behavior.

## Product Surface

- Improve the terminal-first interactive experience beyond the current basic
  `ikaros chat` REPL: readline/history, multiline input, cleaner streaming
  output, visible tool/approval/context status, cancellation, session resume,
  and coding commands should become the primary pre-MVP user surface before a
  web UI.
- Improve voice and body integration beyond provider/status contracts.
- Add optional remote sync only after local state, audit, and conflict behavior are well defined.
- Define remote or distributed subagent worker boundaries before introducing multi-node execution.
