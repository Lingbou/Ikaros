# Roadmap

This roadmap describes planned work for Ikaros and is scoped as future planning rather than project history.

## Before MVP Release

- Keep formatting, clippy, tests, and generated docs passing through direct local commands.
- Keep agent-loop fallback parsing limited to provider-native tool calls plus the strict documented JSON fallback.
- Keep `config.yaml` validation current as provider, memory, voice, and runtime fields change.
- Strengthen live smoke coverage for OpenAI-compatible model APIs, especially Moonshot and SiliconFlow.
- Expand provider stream fixture tests for typed text, reasoning, refusal, tool-call, usage, error, and done events.
- Expand policy and approval tests for path handling, secret-looking inputs, network calls, plugin execution, and approval replay.
- Keep approval replay bound to workspace, exact approved input, and agent identity.
- Keep command-backed plugin tests focused on malicious manifests, path traversal, stdin/output limits, timeout limits, and output redaction.
- Add clearer migration and backup reports for `doctor --fix`.
- Keep README concise and move subsystem design details into language-scoped documentation.

## Runtime

- Harden `AgentRuntime` so future runtimes can plug in without changing provider adapters.
- Expand `ikaros-session` from the initial event store into the runtime fact
  source for chat, gateway, schedule, approvals, replay, and reports.
- Derive more runtime reports from persisted event streams rather than carrying
  separate one-off summaries.
- Refine tool continuation behavior and report fields for automation users.
- Continue separating provider transport concerns from turn-loop ownership.
- Add stricter compatibility tests for provider-specific tool-call differences.

## Context And Memory

- Keep local-first memory and RAG as the default behavior.
- Define governed remote memory adapters behind the provider registry.
- Require remote memory behavior to match local approval, audit, promotion, demotion, sync, and secret-handling rules.
- Add dry-run reports for memory migration or synchronization.
- Exercise session-switch, delegation-observation, and pre-compression lifecycle hooks with real provider tests before enabling remote writes.

## Gateway And Automation

- Evolve the local gateway worker into a long-running daemon with device pairing, capabilities, and multi-channel routing.
- Add external message adapters that only write into the governed local gateway inbox.
- Broaden schedule delivery targets after external adapters have clear routing and audit behavior.
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
