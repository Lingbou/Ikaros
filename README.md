# Ikaros

[简体中文](README_zh-CN.md) | [Documentation](docs/README.md)

Ikaros is an early-stage, local-first agent runtime written in Rust.

It keeps persona, memory, RAG, model providers, tool execution, policy
approvals, and audit logs behind separate boundaries.

The project is currently a pre-MVP local runtime for development and
experimentation. It is not yet a stable product or API surface.

`ikaros init` creates a minimal local `config.yaml` with inline model fields.
Remote model calls fail early until `api_key`, `base_url`, and `model` are
filled in locally. RAG embeddings stay on local `hash`, and voice stays on
`mock`, unless setup or local config explicitly changes them.

## What It Does

- Provides a CLI and terminal workbench for local agent workflows: chat,
  session replay, context/memory/RAG inspection, scheduled tasks, message
  ingestion, approvals, plugins, coding turn reports, code review helpers, and
  guarded edits.
- Exposes first-slice local API, MCP, browser/CDP, web search/extract,
  vision, image generation, and multimodal attachment surfaces through the same
  local runtime boundaries.
- Keeps memory, chat timelines, RAG indexes, automation metadata, gateway
  messages, approvals, and audit logs local by default.
- Routes tool execution through a harness layer with policy decisions,
  approval requests, audit events, dry-run behavior, and guardrails.
- Implements OpenAI-compatible, Anthropic-compatible, and Ollama model adapters,
  plus local RAG embeddings, harness-governed remote RAG embedding egress, and
  OpenAI-compatible TTS/ASR adapters. Mock providers remain available for
  explicit offline tests.
- Exposes agent profiles such as `build`, `plan`, and `general` to adjust
  persona context and policy behavior without bypassing hard safety rules.

## Repository Layout

- `crates/ikaros-core`: shared config, paths, task state, redaction, errors,
  and agent profile types.
- `crates/ikaros-automation`: local scheduled automation metadata and run state.
- `crates/ikaros-body`: replaceable body/status/frame contracts and dashboard rendering.
- `crates/ikaros-cli`: the `ikaros` command-line application.
- `crates/ikaros-coding`: repository scan, guarded patching, structured patch
  failures, turn diff tracking, code review, coding turn reports, self-modify
  records, and test-command analysis.
- `crates/ikaros-context`: context bundles, prompt sections, references,
  provider-aware token budgets, token estimators, quota-based compaction, and
  diff primitives.
- `crates/ikaros-gateway`: local message inbox/outbox metadata and delivery routes.
- `crates/ikaros-harness`: policy engine, approval queue, audit log, skill
  execution session, plugins, and task runner.
- `crates/ikaros-host`: host-side assembly for runtime locations, agent
  instances, execution sessions, skill registries, and governed egress.
- `crates/ikaros-mcp`: harness-managed MCP stdio server and one-shot MCP stdio
  probe primitives.
- `crates/ikaros-memory`: local JSONL and SQLite memory stores, lifecycle hooks,
  and policy journal primitives.
- `crates/ikaros-models`: mock, OpenAI-compatible, Anthropic, and Ollama model
  providers with provider profiles, context profiles, registry metadata,
  governance, retry policy, health state, and usage logging.
- `crates/ikaros-protocol`: versioned protocol types shared by API, TUI,
  gateway, replay, and external surfaces.
- `crates/ikaros-rag`: local RAG ingestion, indexing, retrieval, and local
  embedding primitives. Remote embedding HTTP lives in harness-governed RAG
  skills, not in this core crate.
- `crates/ikaros-runtime`: runtime orchestration for chat, tasks, schedules,
  gateway drain, body status, diagnostics, and agent handoff.
- `crates/ikaros-session`: session ids, turn ids, typed agent events,
  turn-scoped session writes, SQLite `state.db`, append-only session entries,
  and replay reads.
- `crates/ikaros-service`: service-manager template rendering for local worker processes.
- `crates/ikaros-skills`: built-in harness skills for filesystem, shell/git,
  memory, RAG, voice, coding, persona, plugins, and progressive-disclosure tool
  bridges.
- `crates/ikaros-soul`: persona, emotion, tone, and relationship primitives.
- `crates/ikaros-voice`: TTS and ASR provider abstractions with mock and
  OpenAI-compatible implementations.
- `docs/`: language-scoped design notes and subsystem documentation.

## Quick Start

```bash
cargo run -p ikaros-cli -- init
cargo run -p ikaros-cli -- setup --interactive
cargo run -p ikaros-cli -- setup \
  --api-key "$MODEL_API_KEY" \
  --base-url https://api.example.com/v1 \
  --model provider-model-id \
  --reuse-model-provider-for-embedding \
  --embedding-model provider-embedding-model
cargo run -p ikaros-cli -- config validate
cargo run -p ikaros-cli -- config show
cargo run -p ikaros-cli -- provider inspect
cargo run -p ikaros-cli -- provider health
cargo run -p ikaros-cli -- provider matrix
cargo run -p ikaros-cli -- provider profiles
cargo run -p ikaros-cli -- doctor
cargo run -p ikaros-cli --
ikaros
cargo run -p ikaros-cli -- workbench
cargo run -p ikaros-cli -- chat
cargo run -p ikaros-cli -- chat --message "hello"
```

The default entry, `ikaros` after installation or `cargo run -p ikaros-cli --`
from a checkout, opens the fullscreen TUI. Use it for normal local interaction:
chat with the configured model, inspect the current session, review context and
memory, approve or deny tool requests, and run the coding workflow without
leaving the terminal.

Use `ikaros <PATH>` to start in another workspace. `--workspace <PATH>` remains
available for compatibility and scripts. Use explicit `ikaros workbench`,
`debug`, and inspect/status commands when scripts need machine-readable
screen/status snapshots instead of the human TUI.

Useful workbench commands:

- `/status`: show the active agent, model, provider health, budget, gateway, and queue state.
- `/screen`: render the navigable status/timeline/main/side panels.
- `/timeline`, `/replay`, `/trace`: inspect past turns and failures from `state.db`.
- `/context`, `/memory`, `/rag`, `/tools`: inspect what the agent can see and
  which tools are visible.
- `/sandbox [--probe]`: inspect current execution isolation, process, env, and
  network diagnostics.
- `/attach`: add image, audio, or file content blocks to the next chat turn.
- `/web`, `/browser`, `/vision`, `/image`: use governed web, CDP, vision, and
  image-generation surfaces.
- `/provider inspect`, `/provider health`, `/provider matrix`, `/provider debug`: inspect provider
  setup and diagnostics.
- `/approval`: list or resolve pending approvals.
- `/cancel`: cancel queued or running continuations for the active session.
- `/code plan|apply|test|review|rollback`: run the governed coding workflow.
- `/api status`: inspect the local OpenAI-compatible API surface.
- `/mcp status`: inspect configured external MCP servers.
- `/mcp call-http <url> <tool>`: call a HTTP MCP tool through the active session
  `NetworkEgress` boundary.

The fullscreen TUI is now the default terminal surface: it has raw-mode input,
mouse-wheel scrolling, bracketed paste, persistent redraws on real TTYs, and
deterministic structured exports for explicit screen/debug workflows. In a real
fullscreen TTY, read-only slash commands refresh the workbench instead of
printing raw protocol lines; `/help` and `/commands` open the command palette.
`screen_json`, `screen_mode`, trace, and status snapshots remain available from
explicit screen/debug commands and non-TTY script paths.

Common local workflows:

```bash
cargo run -p ikaros-cli -- memory add "Keep RAG local-first" --kind project --scope ikaros
cargo run -p ikaros-cli -- rag ingest docs --scope project
cargo run -p ikaros-cli -- code workflow "provider coding loop" \
  --mode edit \
  --model-loop \
  --apply-patch \
  --run-tests \
  --max-iterations 2 \
  --test-command "cargo test"
cargo run -p ikaros-cli -- debug trace <session-id>
cargo run -p ikaros-cli -- debug state-db --checkpoint
cargo run -p ikaros-cli -- approval list
cargo run -p ikaros-cli -- mcp status
cargo run -p ikaros-cli -- acp serve --agent build --workspace .
cargo run -p ikaros-cli -- api serve --port 8003
cargo run -p ikaros-cli -- web search "Ikaros runtime"
cargo run -p ikaros-cli -- vision describe screenshots/workbench.png
cargo run -p ikaros-cli -- image generate "small local-first agent logo"
```

Use `IKAROS_HOME=/custom/path` or `--ikaros-home /custom/path` to isolate local
state. The default state directory is `~/.ikaros`.

## Configuration

`ikaros init` creates `IKAROS_HOME/config.yaml`. The default file is intentionally
small:

```yaml
schema_version: 1

model:
  default:
    preset: auto
    model: ""
    api_key: ""
    base_url: ""
```

For the common single-model setup, fill `model.default.model`,
`model.default.api_key`, and `model.default.base_url`. `preset: auto` keeps
provider-profile detection enabled. Use a concrete preset such as `kimi`,
`openai`, `anthropic`, or `ollama` when the provider is known.

Use `ikaros init --full` when you want the expanded default YAML up front. The
full file includes provider pools, agent profiles, memory, RAG, voice, gateway,
and execution sections.

`ikaros setup --interactive` prompts for the same first-run fields that can also
be supplied as flags with `ikaros setup --api-key ... --base-url ... --model ...`.
If the current file is still minimal, setup expands it to the full YAML before
writing provider/resource fields. It stores plaintext provider keys only in the
local config file, leaves embedding on local `hash`, keeps TTS/ASR on `mock`
unless explicit provider triplets are supplied, validates the result, and does
not print the key. When one OpenAI-compatible endpoint provides multiple
resources, use `--reuse-model-provider-for-embedding`,
`--reuse-model-provider-for-tts`, or `--reuse-model-provider-for-asr` with the
corresponding resource model flag to avoid repeating the same key and base URL.

Ordinary chat injects accepted memory projections, recent history, and session
working memory. Long-term memory search is explicit through the `memory_search`
tool or `--memory-search-limit`. RAG is treated as cited reference retrieval and
is off unless a profile enables it or the user passes `--rag-top-k`.

The authoritative chat timeline is the agent `state.db` session store. Ordinary
chat turns write user/assistant entries and typed events there only. History,
search, replay, and workbench views are projected from session replay.

`ikaros mcp serve-stdio` exposes the active agent's enabled skills through a
minimal MCP stdio JSON-RPC server. It does not bypass Ikaros policy: `tools/call`
uses the same `ExecutionSession`, approval, audit, workspace scope, and
`ExecutionEnv` path as normal tool execution.

`ikaros mcp status` lists external MCP servers configured under
`mcp.servers`. `ikaros mcp probe <id>` probes one configured stdio server and
applies its include/exclude tool filters; disabled entries require `--force`.

`ikaros mcp probe-stdio <command> -- <args...>` is the first MCP client slice. It
starts a stdio MCP server through the harness process boundary, sends
`initialize` and `tools/list`, and prints a redacted capability report. It is a
one-shot probe and is treated as an arbitrary local process, so default policy
may require approval before it runs. Persistent client lifecycle management is
intentionally still a later step.

`ikaros api serve` starts a loopback-only OpenAI-compatible API slice for local
clients. It exposes chat completions, Responses, embeddings, image generation,
speech, transcription, model discovery, health, and Ikaros protocol metadata.
Requests still use the active agent, session store, audit log, provider
governance, and network egress boundaries.

`ikaros web`, `ikaros browser`, `ikaros vision`, `ikaros image`, and chat
attachments are local-first integration surfaces. Network work still goes
through `NetworkEgress`; local files and generated outputs stay under the
workspace or `IKAROS_HOME` policy boundaries.

Switch local stores to SQLite by editing `~/.ikaros/config.yaml`:

```yaml
memory:
  backend: sqlite

rag:
  backend: sqlite
  embedding_provider: hash
```

Real API keys must not be written into this repository. Keep them in
`~/.ikaros/config.yaml` or another local `IKAROS_HOME/config.yaml`, then run
`ikaros config validate` after editing the file.

Validation reports missing keys, URLs, model names, invalid backends, unknown
fields, and descriptor-only external memory providers without printing secret
values. `ikaros config show` prints a redacted runtime summary with provider
families, model names, storage backends, execution settings, and
`*_configured` booleans for credentials/endpoints.

Automation can use `ikaros config validate --json` and
`ikaros config show --json`. Invalid configs still exit non-zero for validation,
but stdout is a machine-readable report with `valid`, `errors`, and `warnings`.

## Safety Model

Ikaros treats local tool execution as a policy-governed operation:

- Safe reads are allowed within the harness scope.
- Workspace writes, shell writes, network calls, and secret-looking paths
  require policy evaluation and may return an approval request instead of
  executing.
- Destructive commands, direct secret access, publishing actions, and ordinary
  self-modification are denied by default.
- Approval requests and tool calls are recorded locally with redaction.
- Remote deployment is for test environments only and is handled manually before MVP.

Self-modify commands are narrow: proposals are stored locally, apply requires an
approval id, target drift is checked, and post-check failure can roll back the
change.

## Deployment

The first deployment artifact is a local Docker image:

```bash
docker build -f docker/Dockerfile -t ikaros:local .
docker compose -f docker/compose.yml run --rm ikaros --help
```

Runtime state and plaintext provider credentials stay outside the image under
`/data/ikaros`, normally backed by a Docker volume. See
[Docker deployment](docs/en/deployment.md) for the current contract and
limitations.

## Development

Common checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
cargo audit
cargo doc --workspace --all-features --no-deps
cargo run -p ikaros-cli -- doctor
```

Do not commit, tag, publish, or push from automated tooling unless the
maintainer explicitly asks.

## Documentation

- [Full documentation index](docs/README.md)
- [Architecture](docs/en/architecture.md)
- [Harness model](docs/en/harness-model.md)
- [Agent loop design](docs/en/agent-loop.md)
- [Safety model](docs/en/safety-model.md)
- [Memory model](docs/en/memory-model.md)
- [Context engine](docs/en/context-engine.md)
- [RAG model](docs/en/rag-model.md)
- [Model providers](docs/en/model-providers.md)
- [Voice providers](docs/en/voice-providers.md)
- [Body model](docs/en/body-model.md)
- [Automation model](docs/en/automation-model.md)
- [Message gateway](docs/en/message-gateway.md)
- [Service manager templates](docs/en/service-manager.md)
- [Configuration](docs/en/configuration.md)
- [API reference](docs/en/api-reference.md)
- [Plugin system](docs/en/plugin-system.md)
- [Self-modify design](docs/en/self-modify.md)
- [Deployment](docs/en/deployment.md)
- [Roadmap](ROADMAP.md)
