# Ikaros

[简体中文](README_zh-CN.md) | [Documentation](docs/README.md)

Ikaros is an early-stage, local-first agent runtime written in Rust. It is built around a clear separation between persona, memory, RAG, model providers, tool execution, policy approvals, and audit logs.

The project is currently a pre-MVP local runtime for development and experimentation. It is not yet a stable product or API surface. The generated configuration uses local storage and protocol-level provider settings; remote model, embedding, TTS, and ASR calls fail early until `api_key`, `base_url`, and `model` are filled in locally.

## What It Does

- Provides a CLI for local agent workflows: chat, memory, RAG, scheduled tasks, message ingestion, approvals, plugins, coding turn reports, code review helpers, and guarded edits.
- Keeps memory, chat history, RAG indexes, automation metadata, gateway messages, approvals, and audit logs local by default.
- Routes tool execution through a harness layer with policy decisions, approval requests, audit events, dry-run behavior, and guardrails.
- Implements OpenAI-compatible, Anthropic-compatible, and Ollama model adapters, plus OpenAI-compatible embedding, TTS, and ASR adapters. Mock providers remain available only when explicitly selected for offline tests.
- Exposes agent profiles such as `build`, `plan`, and `general` to adjust persona context and policy behavior without bypassing hard safety rules.

## Repository Layout

- `crates/ikaros-core`: shared config, paths, task state, redaction, errors, and agent profile types.
- `crates/ikaros-automation`: local scheduled automation metadata and run state.
- `crates/ikaros-body`: replaceable body/status/frame contracts and dashboard rendering.
- `crates/ikaros-cli`: the `ikaros` command-line application.
- `crates/ikaros-coding`: repository scan, guarded patching, structured patch failures, turn diff tracking, code review, coding turn reports, self-modify records, and test-command analysis.
- `crates/ikaros-context`: context bundle, section, reference, provider-aware token budget, token estimator adapters, quota-based compaction, and diff primitives.
- `crates/ikaros-gateway`: local message inbox/outbox metadata and delivery routes.
- `crates/ikaros-harness`: policy engine, approval queue, audit log, skill execution session, plugins, and task runner.
- `crates/ikaros-memory`: local JSONL and SQLite memory stores, lifecycle hooks, and policy journal primitives.
- `crates/ikaros-models`: mock, OpenAI-compatible, Anthropic, and Ollama model providers with context profiles, provider registry metadata, governance, retry policy, health state, and usage logging.
- `crates/ikaros-rag`: local RAG ingestion, indexing, retrieval, and embedding providers.
- `crates/ikaros-runtime`: runtime orchestration for chat, tasks, schedules, gateway drain, body status, diagnostics, and agent handoff.
- `crates/ikaros-session`: session ids, turn ids, typed agent events, turn-scoped session writes, SQLite `state.db`, append-only session entries, and replay reads.
- `crates/ikaros-service`: service-manager template rendering for local worker processes.
- `crates/ikaros-skills`: built-in harness skills for filesystem, shell/git, memory, RAG, voice, coding, persona, and plugins.
- `crates/ikaros-soul`: persona, emotion, tone, and relationship primitives.
- `crates/ikaros-voice`: TTS and ASR provider abstractions with mock and OpenAI-compatible implementations.
- `docs/`: language-scoped design notes and subsystem documentation.

## Quick Start

```bash
cargo run -p ikaros-cli -- init
cargo run -p ikaros-cli -- config validate
cargo run -p ikaros-cli -- provider inspect
cargo run -p ikaros-cli -- provider health
cargo run -p ikaros-cli -- doctor
cargo run -p ikaros-cli -- chat
cargo run -p ikaros-cli -- chat --message "hello"
```

Useful local commands:

```bash
cargo run -p ikaros-cli -- memory add "Keep RAG local-first" --kind project --scope ikaros
cargo run -p ikaros-cli -- memory search "RAG"
cargo run -p ikaros-cli -- memory add --kind relationship --scope default --observer alice --subject bob "Bob likes pancakes"
cargo run -p ikaros-cli -- memory projection render --scope ikaros
cargo run -p ikaros-cli -- memory candidate list
cargo run -p ikaros-cli -- memory candidate accept <candidate-id> --supersedes <memory-id> --reason "user corrected this"
cargo run -p ikaros-cli -- memory working prune
cargo run -p ikaros-cli -- rag ingest docs --scope project
cargo run -p ikaros-cli -- rag search "harness policy"
cargo run -p ikaros-cli -- task run "summarize this repository" --dry-run
cargo run -p ikaros-cli -- code plan "review a candidate patch" --diff "<unified diff>"
cargo run -p ikaros-cli -- code apply "apply candidate patch" --diff "<unified diff>"
cargo run -p ikaros-cli -- code test "run focused tests" --test-command "cargo test"
cargo run -p ikaros-cli -- code rollback <session-id> --turn-id <turn-id>
cargo run -p ikaros-cli -- code workflow "provider coding loop" --mode edit --model-loop --apply-patch --run-tests --max-iterations 2 --test-command "cargo test"
cargo run -p ikaros-cli -- chat # then: /code plan "review a candidate patch"
cargo run -p ikaros-cli -- debug context-diff <session-id>
cargo run -p ikaros-cli -- debug memory-lifecycle <session-id>
cargo run -p ikaros-cli -- debug coding-turn <session-id>
cargo run -p ikaros-cli -- approval list
cargo run -p ikaros-cli -- skill list
```

Use `IKAROS_HOME=/custom/path` or `--ikaros-home /custom/path` to isolate local state. The default state directory is `~/.ikaros`.

## Configuration

`ikaros init` creates `IKAROS_HOME/config.yaml`. The default config uses local JSONL storage and generic OpenAI-compatible provider entries with empty local credentials. Ordinary chat injects accepted memory projections, recent history, and session working memory; RAG is treated as cited reference retrieval and is off unless a profile enables it or the user passes `--rag-top-k`.

Switch local stores to SQLite by editing `~/.ikaros/config.yaml`:

```yaml
memory:
  backend: sqlite

chat_history:
  backend: sqlite

rag:
  backend: sqlite
  embedding_provider: hash
```

Real API keys must not be written into this repository. Configure remote providers in `~/.ikaros/config.yaml` or another `IKAROS_HOME/config.yaml`. The generated file puts plaintext third-party provider settings at the top:

```yaml
providers:
  model:
    api_key: "replace-with-your-model-key"
    base_url: "https://api.example.com/v1"
  embedding:
    api_key: "replace-with-your-embedding-key"
    base_url: "https://api.example.com/v1"

model:
  default:
    provider: openai-compatible
    model: provider-model-id
```

Provider settings are local-only and are not kept in the repository. Run `ikaros config validate` after editing the file. Missing keys, URLs, model names, invalid backends, unknown fields, and descriptor-only external memory providers are reported without printing secret values.

## Safety Model

Ikaros treats local tool execution as a policy-governed operation:

- Safe reads are allowed within the harness scope.
- Workspace writes, shell writes, network calls, and secret-looking paths require policy evaluation and may return an approval request instead of executing.
- Destructive commands, direct secret access, publishing actions, and ordinary self-modification are denied by default.
- Approval requests and tool calls are recorded locally with redaction.
- Remote deployment is for test environments only and is handled manually before MVP.

Self-modify commands are narrow: proposals are stored locally, apply requires an approval id, target drift is checked, and post-check failure can roll back the change.

## Development

Common checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
cargo run -p ikaros-cli -- doctor
```

Do not commit, tag, publish, or push from automated tooling unless the maintainer explicitly asks.

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
- [Roadmap](ROADMAP.md)
