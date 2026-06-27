# Ikaros Architecture Refactor Remaining Follow-ups

This document is no longer a phase-by-phase migration plan. The architecture
refactor has already collapsed the workspace into the target crate graph, and
legacy implementation crates are no longer workspace members or source
directories. Future workers should treat this file as a current-state note plus
the remaining hardening and verification checklist.

## Current Workspace

The current workspace has exactly these 11 crates:

- `ikaros-agent`
- `ikaros-cli`
- `ikaros-core`
- `ikaros-execution`
- `ikaros-host`
- `ikaros-protocol`
- `ikaros-providers`
- `ikaros-skills`
- `ikaros-state`
- `ikaros-surfaces`
- `ikaros-terminal`

Former implementation crates have been removed from the workspace and
filesystem. The exhaustive former-crate list is:

- `ikaros-api`
- `ikaros-automation`
- `ikaros-body`
- `ikaros-coding`
- `ikaros-context`
- `ikaros-gateway`
- `ikaros-harness`
- `ikaros-mcp`
- `ikaros-memory`
- `ikaros-models`
- `ikaros-rag`
- `ikaros-runtime`
- `ikaros-sandbox`
- `ikaros-service`
- `ikaros-session`
- `ikaros-soul`
- `ikaros-toolkit`
- `ikaros-tui`
- `ikaros-voice`

Do not ask workers to inspect, migrate, shim, delete, or test these crates.
Their names should appear only in historical notes, architecture guards, or
documentation that explicitly describes removed implementation crates.

## Current Ownership

- `ikaros-core`: shared errors, IDs, paths, config schema, agent/profile base
  types, persona/emotion primitives, and redaction.
- `ikaros-protocol`: stable wire/session shapes for API, terminal, gateway,
  replay, MCP, body, context, and external integration contracts.
- `ikaros-state`: durable local state for session timelines, memory, RAG,
  automation metadata, gateway queues, leases, and local reports.
- `ikaros-execution`: policy, approvals, audit, sandbox/process/filesystem/
  network execution, reusable tool contracts, coding primitives, and
  task/plugin dispatch.
- `ikaros-providers`: model, embedding, voice, vision/image, web, registry, and
  provider-governance adapters.
- `ikaros-host`: composition root for config/path/agent instance/provider/store
  setup, execution environment wiring, diagnostics, persona files, approval
  resolution, and host-owned adapters.
- `ikaros-agent`: application use cases for chat, context assembly, coding
  workflow, task loop, schedules, gateway drain, body status, agent loop,
  handoff, and session runner logic.
- `ikaros-skills`: built-in skill implementations and tool packs, exposed
  through execution-governed tool contracts.
- `ikaros-surfaces`: local API, MCP, gateway adapters/webhooks, body dashboard,
  browser/web/vision/image entry points, and service manager integration.
- `ikaros-terminal`: terminal screen models, input, rendering, slash/status/
  timeline behavior, body rendering, and terminal text helpers.
- `ikaros-cli`: thin `clap`, dispatch, and terminal output adapter.

Current direct workspace dependency direction:

```text
cli -> agent/core/execution/host/protocol/providers/skills/state/surfaces/terminal
host -> core/execution/providers/skills/state
surfaces -> core/execution/host/protocol/providers/skills/state
terminal -> core/protocol
agent -> core/execution/protocol/providers/state
skills -> core/execution/protocol/providers/state
providers -> core/protocol
execution -> core
state -> core/protocol
protocol -> core
core -> none
```

## Boundary Invariants

- `ikaros-cli` should parse flags, dispatch commands, and render terminal
  output. Reusable command behavior belongs in `ikaros-agent`,
  `ikaros-surfaces`, `ikaros-terminal`, `ikaros-host`, or
  `ikaros-execution`.
- `ikaros-agent` must receive host-assembled dependencies. It should not load
  config, resolve workspaces, select stores, build registries, or assemble
  execution environments.
- `ikaros-host` should remain the composition root. It should not own
  agent-loop behavior, terminal rendering, or product protocol schemas.
- `ikaros-terminal` must not own execution, provider setup, config loading,
  local stores, gateway queue mutation, or network egress.
- `ikaros-surfaces` should expose adapters and entry points, not local state
  store implementations.
- `ikaros-state` should own local persistence, not provider HTTP calls or
  runtime orchestration.
- `ikaros-protocol` should contain stable shared shapes, not provider clients,
  store implementations, agent behavior, or CLI presentation logic.
- `ikaros-providers` should own provider wire behavior and governance metadata,
  not tool execution policy or agent-loop orchestration.
- `ikaros-execution` should own governed local action boundaries, not provider
  adapters or user-facing CLI rendering.

## Remaining Follow-ups

- Keep architecture guards aligned with the 11-crate graph. Guards should assert
  current invariants such as deleted legacy crates staying out of Cargo
  metadata, CLI staying thin, terminal avoiding runtime dependencies, surfaces
  not exposing state stores, and agent not loading config.
- Continue CLI thinning where modules still contain reusable behavior. The
  next likely targets are mid-sized status/interactive modules, the rest of
  `interactive/screen/open.rs` execution routing, and code review/rollback
  semantics that would fit better in `ikaros-agent` or `ikaros-execution`.
- Review `ikaros-surfaces` dependencies as surface adapters move behind
  narrower host/agent APIs. The goal is adapter ownership without leaking store
  or execution internals.
- Keep provider, context, sandbox, continuation, and terminal hardening focused
  on the current target crates. Do not reopen deleted legacy crates as shims.
- Keep README and architecture docs synchronized with root `Cargo.toml` when
  workspace membership changes.
- Rerun full acceptance before declaring the refactor complete for a release or
  merge. Older "known green" command lists are historical evidence only.

## Current Burn-down Estimate

Snapshot date: 2026-07-06.

The workspace shape is already in the desired 11-crate state. Remaining work is
not another crate migration wave; it is ownership hardening, CLI/TUI thinning,
and final verification.

### Near-term Parallel Work

- Split remaining large CLI command/status modules into focused submodules.
  Current high-value candidates are `chat/interactive.rs`,
  `chat/workbench/status/memory.rs`, `memory.rs`,
  `chat/interactive/continuations.rs`, `chat/workbench/cells.rs`,
  `chat/interactive/multimodal.rs`, and `chat/workbench/status/tools.rs`.
- Keep each worker on a disjoint write scope. Workers should not edit
  `architecture_guard.rs`; the integration agent should add guards after each
  split lands.
- Prefer mechanical extraction over behavior changes. User-visible CLI output
  must stay stable unless a task explicitly says otherwise.

### Serial Integration Work

- Add or tighten architecture guards after each ownership move, especially for
  CLI modules that should stay as dispatch/output adapters and terminal modules
  that must not gain host, state, execution, or provider responsibilities.
- Review `ikaros-surfaces` and `ikaros-host` together when API/MCP/service
  entry points move. These boundaries are composition-sensitive and should not
  be split by multiple workers at the same time.
- Review `ikaros-agent` large use-case modules only after CLI/TUI ownership
  stabilizes. Agent edits tend to touch shared chat/session/task behavior and
  should stay integration-controlled.

### Estimated Remaining Size

- Small tasks: 4-6 targeted guard/doc updates or smoke fixes.
- Medium tasks: 6-10 focused module splits in CLI/terminal/surfaces/host.
- Large tasks: 2-3 application-layer cleanups in `ikaros-agent`, especially
  gateway drain, session continuations, and schedule/task orchestration.
- Verification: one broad pass with fmt, architecture guard, workspace check,
  clippy, full tests, CLI help, mock smoke, and live provider smoke only if
  provider or egress paths changed.

The fastest safe path is to run two to three non-overlapping workers at a time
for mechanical splits while the integration agent owns guards, compile fixes,
and broad verification.

## Target-Crate Verification

Use target-crate checks for the current owners. Do not run `cargo test -p`
against deleted crates such as `ikaros-runtime`, `ikaros-memory`,
`ikaros-harness`, `ikaros-api`, `ikaros-tui`, or `ikaros-service`.

For architecture-boundary changes:

```powershell
cargo test -p ikaros-cli --test architecture_guard
cargo check --workspace --all-targets
```

For state, execution, provider, agent, host, surface, terminal, or CLI changes,
prefer the relevant focused checks:

```powershell
cargo test -p ikaros-state
cargo test -p ikaros-execution
cargo test -p ikaros-providers
cargo test -p ikaros-agent
cargo test -p ikaros-host
cargo test -p ikaros-surfaces
cargo test -p ikaros-terminal
cargo test -p ikaros-cli --test cli_smoke runtime_core
cargo run -p ikaros-cli -- --help
```

For broad pre-merge acceptance:

```powershell
cargo fmt --all -- --check
cargo test -p ikaros-cli --test architecture_guard
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p ikaros-cli -- --help
```

Mock and live smoke must use repo-external temporary `IKAROS_HOME` directories.
Do not print or commit API keys. Clean temporary smoke directories after each
run.

## Stop Conditions

Stop and ask before continuing if any of these happen:

- A dependency cycle cannot be resolved by moving shared types into
  `ikaros-core` or stable wire shapes into `ikaros-protocol`.
- A change would reintroduce a removed implementation crate, compatibility
  shim, or top-level dependency outside the current 11-crate workspace.
- A boundary hardening task requires user-visible CLI behavior changes rather
  than ownership-only moves.
- Full workspace tests fail repeatedly for unrelated modules after migration or
  hardening edits.
- Live smoke returns authentication, quota, or provider model availability
  errors after config shape is verified.
- A task requires writing API keys or other secrets to the repo, docs, tests, or
  stdout.
