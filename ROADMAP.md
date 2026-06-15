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
- Refine tool continuation behavior and report fields for automation users.
- Continue separating provider transport concerns from turn-loop ownership.
- Keep provider request quirks inside model adapters and compatibility profiles,
  not in the runtime turn loop.
- Add stricter compatibility tests for provider-specific tool-call differences.

## Context And Memory

- Keep local-first memory and RAG as the default behavior.
- Keep `ikaros-context` as the shared boundary for context bundles, references,
  provider-aware token budgets, quota-based compaction, and context diffs.
- Keep `ModelContextProfile` wired into context budgeting and estimator
  selection. Extend the current deterministic adapters with exact
  provider-native tokenizer libraries once the provider registry exists.
- Extend quota-based context assembly with dynamic priority, semantic
  compression, replay/debug commands, and stricter long-running session
  diagnostics.
- Keep relationship data as `MemoryKind::Relationship`, not as a second memory
  database.
- Keep `NoopMemoryProvider` explicit; memory lifecycle hooks should not hide
  default no-op behavior in the trait.
- Record real memory policy decisions in `MemoryJournal` before treating
  promotion, demotion, forgetting, or skipped writes as product behavior.
- Define governed remote memory adapters behind the provider registry.
- Require remote memory behavior to match local approval, audit, promotion, demotion, sync, and secret-handling rules.
- Add dry-run reports for memory migration or synchronization.
- Exercise session-switch, delegation-observation, and pre-compression lifecycle hooks with real provider tests before enabling remote writes.

## Gateway And Automation

- Evolve the local gateway worker into a long-running daemon with device pairing, capabilities, multi-channel routing, and session continuity across channel threads.
- Add external message adapters that only write into the governed local gateway inbox.
- Broaden schedule delivery targets after external adapters have clear routing, audit behavior, and replay evidence.
- Keep schedules as work requests; execution should continue to pass through runtime and harness boundaries.

## Execution Environment

- Define isolation levels, mount rules, and network-egress behavior before adding non-local execution backends.
- Add Docker, ssh, and dry-run `ExecutionEnv` backends after the isolation contract is testable.
- Route file, process, network, plugin, shell, test, and coding helpers through the environment abstraction.

## Plugins And Skills

- Keep executable tools separate from prompt skills and skill bundles.
- Expand plugin validation for manifest shape, command metadata, canonical paths, resource limits, and marketplace metadata.
- Add richer plugin protocols only after command-backed plugins have stable execution and audit behavior.

## Product Surface

- Improve voice and body integration beyond provider/status contracts.
- Add optional remote sync only after local state, audit, and conflict behavior are well defined.
- Define remote or distributed subagent worker boundaries before introducing multi-node execution.
