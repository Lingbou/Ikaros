# Contributing

Thanks for helping build Ikaros.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
cargo audit
```

Use Conventional Commits for future commits, but do not create commits in
automation unless the maintainer explicitly asks.

## Documentation Style

- Write for maintainers and users first, not for agents.
- Keep overview pages short. Put subsystem details in `docs/en` and
  `docs/zh-CN`.
- Wrap Markdown prose so normal review diffs stay readable.
- Use categorized command lists instead of long protocol dumps.
- Keep English and Simplified Chinese docs aligned when behavior changes.

## Pull Requests

- Keep changes scoped to the owning crate or document the cross-crate reason.
- Add tests for policy, memory, RAG, provider, and CLI behavior when those
  surfaces change.
- Do not include real secrets in tests, docs, fixtures, logs, or audit samples.
- Do not modify local reference-material directories; they are not part of the project.
- Update docs when behavior, security boundaries, configuration, or CLI commands change.

## Deferred Test Debt

Before declaring the MVP stable, add focused coverage for:

- `setup --interactive` with a temp `IKAROS_HOME` and scripted stdin.
- `skill quarantine` and `skill unquarantine` CLI output and marketplace state.
- Real PTY workbench rendering through the `CrosstermBackend` path.
- Local HTTP egress integration around DNS pinning and rejected restricted targets.
- Memory update reports for JSONL, SQLite, provider, and skill output paths.
- Docker sandbox backend smoke coverage for workspace mounting, `--network none`,
  file ownership/permissions, timeout/output caps, and plugin cwd behavior.
- `debug sandbox --probe` smoke coverage for local, dry-run, and Docker backends,
  including redacted failures when Docker is unavailable or the image is missing.
- Workbench sandbox screen coverage for the `sandbox` cell, `/debug sandbox`
  open-selected read-only execution, explicit `/debug sandbox --probe`, and
  redacted sandbox diagnostics in `screen_json`.
- Doctor/debug output coverage for Docker sandbox image metadata and RAG remote
  embedding egress classification.
- Voice TTS/ASR egress coverage proving OpenAI-compatible providers use
  `ExecutionEnv::send_network_request`, multipart ASR uses binary request
  bodies, and binary TTS responses are redacted in debug output.
- MCP stdio smoke coverage for malformed JSON-RPC, disabled/deferred tool
  visibility, approval-pending tool results, and audit evidence. Extend the
  one-shot `mcp_stdio_probe` coverage to malformed server output, process
  timeout/output caps, non-zero exit status, approval/rejection behavior, and
  redaction of secret-like tool metadata. Add configured `mcp.servers` CLI smoke
  coverage for `mcp status`, disabled server skip, `mcp probe <id> --force`,
  include/exclude filtering, and unknown server ids.
- MCP HTTP smoke coverage for `mcp probe-http` and `mcp call-http`, including
  governed egress allow/deny behavior, object-only `arguments_json`, redacted
  request/response output, tool-call HTTP failures, and unsupported response
  content.
- Agent handoff smoke coverage for `--parent-session`, persisted child
  `parent_session_id`, redacted parent `subagent_result` evidence, missing
  parent no-op behavior, and cross-profile state-store boundaries.
- Harness tracing capture tests for policy decision, tool allow/deny/approval,
  approval replay, process start/completion/failure, governed network allow/deny,
  HTTP egress completion, and secret redaction in structured tracing fields.
- CLI trace log smoke coverage proving `logs/trace.jsonl` is created, `debug
  logs --source trace` paginates it, invalid JSONL lines are surfaced as
  redacted diagnostic rows, and `debug insights` includes trace counts.
- Workbench debug observability smoke coverage for `/debug logs`,
  `/debug logs --source trace`, `/debug insights`, the `observability` screen
  cell, and `logs_json` / `insights_json` redaction.
- Workbench debug dump/state-db smoke coverage for read-only `/debug dump` and
  `/debug state-db`, `dump_json` / `state_db_json` redaction, and the `state db`
  screen cell. Keep file-writing dump exports and state-db maintenance flags as
  explicit top-level CLI actions.
- Workbench MCP status smoke coverage for `/mcp status`, `/commands mcp`, the
  `mcp` screen cell, `screen open-selected`, and `mcp_status_json` redaction.
  Keep configured MCP probes and direct stdio probes as explicit actions that
  do not run from passive workbench rendering.
- Workbench selected-action protocol coverage for `screen_json.selected.actions`
  and `screen_selected_actions_json.actions`, including approval approve/deny,
  continuation cancel, pending-input clear, read-only open-selected, and secret
  redaction without requiring frontends to parse cell detail strings.
- Workbench status-panel navigation coverage for `/screen --focus status`,
  selected model/session/queue/gateway actions, status scroll/selection JSON,
  and non-TTY smoke output without falling back to timeline selection.
- Workbench memory/RAG panel smoke coverage for the main-screen `memory` and
  `rag` cells, including projection/candidate/working-memory counts, RAG
  backend/embedding metadata, selected `/memory` and `/rag` navigation, and
  secret redaction in snippets and paths.
- Workbench memory projection explain coverage for `/memory`,
  `memory_status_json.projection_explain`, included/excluded projection reasons,
  bucket counts, secret redaction, and proof that passive explain does not write
  projection files.
- Readiness and debug dump smoke coverage for `m6_context_memory_rag`,
  `mcp_protocol`, `mcp_summary`, `memory_summary`, and `rag_summary`, including
  explicit proof that passive reports do not start MCP probes or RAG retrieval.
- Default chat non-TTY smoke coverage for `ikaros` with piped stdin/stdout,
  ensuring it stays on the inline terminal path instead of entering raw terminal
  event mode.
- `config budget` and workbench `/budget` smoke coverage for show/set/disable
  without leaking provider secrets, including hot-reloading the active
  workbench model provider after `/budget set` or `/budget disable`, plus
  `/screen` model-budget cell actions for read-only open-selected and explicit
  raise/disable recovery commands.
- Workbench markdown and diff rendering golden coverage for headings, lists,
  fenced code, diff preview truncation, secret redaction, and non-git
  workspaces.
- Workbench line-mode raw editor PTY smoke coverage for history navigation,
  slash completion, cursor movement, undo, paste, Ctrl-C cancellation, and
  non-TTY fallback.
- Workbench progress output coverage for streaming and non-streaming chat turns,
  tool waits, provider failures, budget failures, elapsed time, recovery actions,
  and secret redaction in `workbench_progress_json`.
- Workbench screen recovery coverage for error cells exposing `/status`,
  `/budget`, `/budget set`, `/budget disable`, and `/trace --failed` actions,
  including open-selected read-only status execution.
- Slash command registry snapshot coverage for command names, argument models,
  side effects, output contracts, surfaces, and permission metadata consumed by
  workbench, gateway, and future ACP clients.
- `debug readiness` snapshot coverage for readiness row statuses, redaction,
  provider attention flags, sandbox backend classification, and config invalid
  reporting.

## DCO / CLA

No CLA is configured. The intended policy is Developer Certificate of Origin
sign-off for external contributions once the project is opened publicly.
