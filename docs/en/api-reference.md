# API Reference

The Rust crate APIs are pre-release. The supported user surface is the `ikaros` CLI.

Use generated Rust docs for crate-level details:

```bash
cargo doc --workspace --all-features --no-deps
```

## Common Commands

Initialize and inspect local state:

```bash
ikaros init
ikaros init --full
ikaros setup --interactive
ikaros setup \
  --api-key "$MODEL_API_KEY" \
  --base-url https://api.example.com/v1 \
  --model provider-model-id
ikaros config validate
ikaros config show
ikaros config budget
ikaros config budget --set 200000
ikaros config budget --disable
ikaros doctor
ikaros provider inspect
ikaros provider health
ikaros provider health --live
ikaros provider matrix
ikaros provider matrix --live
ikaros provider profiles
ikaros api serve --port 8003 --bearer-token "$IKAROS_API_TOKEN"
ikaros acp serve --agent build --workspace .
ikaros mcp serve-stdio
ikaros mcp status
ikaros mcp probe <id> [--force]
ikaros mcp probe-stdio <command> -- <args...>
ikaros mcp probe-http https://mcp.example/rpc --include-tool search
ikaros mcp call-http https://mcp.example/rpc search --arguments-json '{"query":"ikaros"}'
ikaros web search "Ikaros runtime" --max-results 5
ikaros web extract https://example.com --max-chars 4000
ikaros vision describe screenshots/workbench.png
ikaros image generate "small local-first agent logo"
ikaros browser launch --headless --url https://example.com
ikaros browser status
ikaros browser list
```

`init` writes the minimal starter config. It contains only `schema_version` and
`model.default` inline fields with `preset: auto`, empty `model`, empty
`api_key`, and empty `base_url`. Use `init --full` when you want the expanded
default YAML with provider pools, agent profiles, memory, RAG, voice, gateway,
and execution sections.

`setup --interactive` prompts for first-run provider fields. The same fields can
be supplied non-interactively with `setup --api-key ... --base-url ... --model
...`. Both paths create the runtime home if needed. If the current config is
still minimal, setup expands it to the full YAML before writing
provider/resource fields. It stores the plaintext model `api_key` in that local
file, defaults embedding to local `hash`, defaults TTS/ASR to `mock` unless
explicit provider triplets are supplied, validates the result, and never prints
the secret value.

`config show` prints the active runtime configuration as a redacted summary. It
shows provider families, model ids, storage backends, execution settings, and
credential/endpoint presence booleans; it does not print API keys or base URLs.
Use `--json` for automation.

`doctor` includes the active config schema version, `config_valid`, and
redacted `config_issue` lines before the subsystem summaries. Its model summary
also reports today's token usage, remaining daily budget, and budget status; the
RAG summary reports whether embeddings use session network egress. It is useful
for startup diagnostics, while `config validate` remains the strict validation
command for scripts.

`provider inspect` reads the local `IKAROS_HOME/config.yaml` provider settings
and prints the resolved provider descriptor: provider family, model, profile,
context window, tokenizer, capabilities, profile policy fields, health state,
cost fields, and any configured model fallback rows. OpenAI-compatible profile
policy fields describe the resolved temperature, reasoning, message,
tool-schema, and extra request-body behavior. It does not call the provider and
does not print API keys.

`provider health` reads the local provider health ledger. `provider health
--live` sends a short request through runtime `NetworkEgress` and records the
result without printing API keys.
`provider matrix` prints the configured model, embedding, TTS, and ASR provider
rows, including descriptor capability, context, input/output/cache-read/cache-write
cost metadata, local readiness,
redacted credential presence, profile policy fields, latest local health status,
and cooldown metadata. `provider matrix --live` probes the configured model, embedding, TTS, and ASR
rows. Model and remote embedding probes go through runtime `NetworkEgress`; local
embedding probes use the local RAG store, while TTS/ASR probes use the configured
voice providers.
`provider profiles` prints the static OpenAI-compatible profile catalog with
auto-detection hints, request-shaping policies, context metadata, and capability
flags.

`api serve` starts the local OpenAI-compatible API surface. It binds only to
loopback addresses and exposes `GET /healthz`, `GET /health`, `GET /ready`,
`GET /v1/models`, `GET /v1/ikaros/protocol`, `POST /v1/chat/completions`,
`POST /v1/responses`, `POST /v1/embeddings`, `POST /v1/images/generations`,
`POST /v1/audio/speech`, and `POST /v1/audio/transcriptions`.

Chat and Responses calls use the active agent's configured model provider
through runtime `NetworkEgress`. Embedding calls use the configured RAG
embedding provider; remote embedding requests route through the same execution
environment. `/v1/embeddings` accepts `encoding_format: "float"` and
`encoding_format: "base64"`; base64 responses encode returned `float32` vector
bytes in little-endian order. `/v1/models` lists chat, embedding, image, TTS,
and ASR rows when they are configured.

Chat request messages may use plain string `content` or OpenAI-style content
parts. The current multimodal protocol slice preserves text, image, audio, and
file parts as model `ContentBlock`s. Provider adapters accept or reject those
blocks according to their capabilities. `image_url` is forwarded to
OpenAI-compatible providers, Anthropic receives image content blocks, and
Ollama receives base64 `images` for `data:image/...;base64,...` inputs.

`/v1/responses` accepts `input` as a string or message-item array. It supports
`instructions`, text/image/audio/file content parts, function tool definitions,
`max_output_tokens`, `temperature`, `top_p`, and `stream: true`. Non-streaming
responses use a Responses-shaped `response` object with Ikaros session evidence;
streaming responses emit Responses SSE-style events. Built-in web/file tools,
background mode, persistent response conversations, retrieval, and exact
Responses event parity remain later API work.

`/v1/images/generations` proxies an OpenAI-compatible image-generation request
to the configured model provider base URL. `POST /v1/audio/speech` and
`POST /v1/audio/transcriptions` proxy OpenAI-compatible voice requests through
the configured voice provider settings. Those routes create session evidence
and audit records like chat, but they do not execute model-supplied tools.

Pass
`--bearer-token` to require `Authorization: Bearer ...` for `/v1/*` routes;
repeat the flag during local key rotation to accept old and new tokens at the
same time. Health and readiness routes stay open for local checks. Clients may
send `X-Ikaros-Client-Id`; the redacted value is recorded in `api_request`
audit events for local traceability. The server also applies a per-process
request budget with `--rate-limit-per-minute` and writes redacted `api_request`
audit events. Each chat or embedding request also creates a
service session turn in `state.db`; the non-standard `ikaros.session_id` and
`ikaros.turn_id` response fields can be used with debug/replay commands. The
same values are also returned as `X-Ikaros-Session-Id`, `X-Ikaros-Turn-Id`, and
`X-Ikaros-Correlation-Id` headers and are written to the audit event. Chat
requests may include OpenAI-style function `tools`, assistant `tool_calls`, and
tool-result messages; Ikaros forwards those through the provider protocol and
projects provider tool calls back into the OpenAI-compatible response, but does
not execute API-supplied tools. Invalid request bodies and internal failures are
returned as redacted JSON error objects instead of dropping the connection.
`stream: true` returns OpenAI SSE-shaped chunks from Ikaros' normalized provider
stream; true live byte-by-byte forwarding, distributed rate limiting, and a
persistent API credential lifecycle are not part of this first slice.

`acp serve` starts the Agent Client Protocol server over stdio JSON-RPC for IDE
clients and other local frontends. It uses the same runtime, session store,
harness policy, approval, audit, workspace scope, and provider boundaries as the
CLI. The first slice supports `initialize`, `initialized`, `session/new`,
`session/prompt`, `session/list`, `session/events`, `session/replay`,
`tools/list`, `approval/list`, and `shutdown`. Its advertised capabilities are
session management, streaming events, tool discovery, approval handling, and
session replay.

`mcp serve-stdio` starts the first harness-managed MCP stdio surface. It speaks
line-delimited JSON-RPC on stdin/stdout and currently supports `initialize`,
`ping`, `tools/list`, and `tools/call`. The exposed tools are the enabled Ikaros
skills for the active agent; tool calls still go through `ExecutionSession`,
policy, approval, audit, workspace scope, and `ExecutionEnv`.

`mcp status` prints configured external MCP servers from `config.yaml` without
starting them. `mcp probe <id>` uses one of those configured records and applies
its `include_tools`/`exclude_tools` filters to the discovered tool report.
Disabled servers are skipped unless `--force` is passed; forced probes still go
through policy and approval.

`mcp probe-stdio` starts a stdio MCP server as a harness-managed process, sends
`initialize` and `tools/list`, and prints the parsed server/tool report. This is
a one-shot client probe rather than a persistent MCP client pool. The command is
treated as an arbitrary local process and therefore goes through the same
policy, approval, audit, workspace scope, timeout, and output-limit path as
other harness process tools.
`mcp probe-http` sends `initialize` and `tools/list` JSON-RPC POST requests to a
HTTP MCP endpoint through the active session `NetworkEgress` boundary, redacts
the response, and applies include/exclude tool filters. It is also a one-shot
probe; persistent HTTP MCP sessions, server-sent event streaming, lifecycle
management, and dynamic tool registration remain later client work.
`mcp call-http` sends `initialize` and one `tools/call` request through the same
`NetworkEgress` boundary. The request and response report are redacted before
printing. This is a controlled one-shot client call; persistent remote tool
registration and long-lived MCP sessions remain later client work.

Chat:

```bash
ikaros
ikaros chat
ikaros chat --message "hello"
ikaros chat --stream --message "hello"
ikaros chat --message "describe this screenshot" --image screenshots/workbench.png
ikaros chat --message "transcribe or inspect this audio" --audio audio/sample.wav
ikaros chat --message "summarize this file" --file docs/en/architecture.md
ikaros chat --context-token-budget 4000 --message "summarize @file:docs/en/architecture.md:1-80"
ikaros chat --memory-search-limit 3 --message "include explicit long-term memory search"
ikaros chat --sessions
ikaros chat --history
ikaros chat --history-search "query"
```

Running `ikaros` with no subcommand, or `ikaros chat` without
`--message` starts the terminal workbench. One-shot CLI commands remain
available for scripting, while the normal interactive path provides history,
multiline input, cursor-aware line editing, undo for the in-memory input buffer,
bracketed paste, queued input, default streaming,
session resume,
timeline/replay/debug views, provider/gateway/task status, approval overlay,
context/memory/RAG status, and coding workflow commands.

Common workbench slash commands are grouped by purpose:

- Help and navigation: `/help`, `/commands [query]`, `/queue`, `/agents`,
  `/agent <profile-or-instance>`, `/status`, and
  `/budget [show|set <tokens>|disable]`.
- Screen control: `/screen [--focus status|timeline|main|side]`,
  `/screen [--focus-next|--focus-prev]`, `/screen [--scroll N]`,
  `/screen [--select N|--select-title TEXT|--select-kind KIND]`,
  `/screen [--select-action SELECTOR]`,
  `/screen [--down|--up|--page-down|--page-up|--top]`,
  `/screen [--palette [query]|--palette-query query|--close-palette]`,
  `/screen [--fullscreen|--inline|--raw|--rich]`, and selected-cell actions
  such as `approve-selected`, `deny-selected`, `cancel-selected`,
  `clear-selected`, `open-selected`, and `confirm-selected`.
- Session control: `/history [limit]`, `/sessions`,
  `/session status|resume|history|timeline|export [path]`, `/resume <session>`,
  `/new`, and `/fork`.
- Timeline and debug views:
  `/timeline|/replay|/debug [turn] [--page N] [--kind KIND] [--failed|--approval]`
  and `/trace [turn] [--kind KIND] [--failed|--approval]`.
- Context and runtime views: `/mentions [query]`, `/context`, `/memory`, `/rag`,
  `/tools`, `/model`,
  `/provider [inspect|health [--live]|matrix [--live]|profiles|debug]`,
  `/mcp status`, `/mcp call-http <url> <tool>`, `/api status`, `/gateway`,
  `/tasks`, `/web`, `/browser`, `/vision`, and `/image`.
- Actions: `/approval|/approvals [approve|deny <id>]`,
  `/cancel [all|<continuation-id>]`, `/diff`, `/multi`, `/clear`,
  `/attach`, `/code <plan|apply|test|review|rollback> ...`, `/review`, and
  `/rollback`.
- Exit: `/quit` or `/exit`.
The `/code` command routes into the same governed `ikaros code` workflow and
writes coding turn evidence to `state.db`.
The `/attach` command adds image, audio, or file content blocks to the next chat
turn. The same attachment path is available to one-shot chat through `--image`,
`--audio`, and `--file`. Local paths are resolved under the workspace, bounded,
and converted to data URLs before they reach the provider.
The `/memory` command shows the active memory surface as three separate layers:
projection files, pending memory candidates, and active working-memory records
for the current chat session, followed by memory lifecycle timeline and journal
cells.
After each streamed turn, the workbench prints compact live cells: model stream
deltas are collapsed into one summary cell, while tool, context, coding,
approval, continuation, audit, and error events remain visible as typed cells.
It also emits `live_cells_json` with
`schema=ikaros-workbench-live-cells-v1`, `version=1`, event category counts,
the suppressed model-stream count, and the redacted compact cell list for async
TUI cell renderers.
It also prints a `rendered_markdown` transcript after streaming completes so
code fences, diff blocks, tables, and redacted error text remain readable
without sacrificing live token output.
The `/status` command prints the active model row with the resolved provider
profile, profile source, context window, default output reservation, tokenizer,
runtime, transport, latest health state, and a `status_model_policy` line for
temperature, reasoning, message, tool-schema, request-body, prompt-cache, and
retry policy. It also includes `status_model_budget`, `status_model_cost`, and
`status_model_fallbacks` lines so the workbench can explain daily token budget,
estimated local cost, cache read/write token accounting, and fallback chain
readiness without a separate debug command.
The `/sandbox [--probe]` command prints the same redacted `sandbox_json` report
as `/debug sandbox`, and the screen sandbox cell opens `/sandbox` by default
while keeping `/debug sandbox` as a deeper diagnostic path.
The `/screen` view also includes gateway, MCP, and API cells. The gateway cell
shows pending/processing/cancelled counts and the redacted `message-worker.lock`
state so duplicate local gateway workers and interrupted messages are visible
without leaving the workbench. The API cell points to `/api status`, which lists
the local OpenAI-compatible chat, Responses, embeddings, image, audio, protocol,
model discovery, and health routes without starting a server or making a live
provider call.
This is the fastest way to confirm that an OpenAI-compatible endpoint such as
Moonshot/Kimi, Qwen, OpenRouter, or a local compatible server resolved to the
expected profile without making a live provider call.
The `/model` command prints the same descriptor from the active workbench
runtime (`model_source: active_runtime`), not by reloading provider state as a
separate top-level command, and prints each configured fallback row with its
resolved profile/readiness summary. Use `/provider inspect` when you explicitly
want the configuration-level provider inspection path.
The slash-command registry classifies commands as inspect, action/probe, or
terminal-output commands for the screen model and future TUI routing. The
default `ikaros` terminal path still prints human-readable command output.
`ikaros chat`, non-TTY runs, and explicit screen/debug commands keep their
deterministic protocol output for scripts. Use `/screen --palette` to inspect
the command palette model; `/help` and `/commands` print command help rather
than opening a persistent palette.
`/provider debug` prints the same redacted structured diagnostics as
`ikaros debug provider`, including profile source, cache policy, health,
fallback rows, and live-smoke readiness hints without making a live provider
call.
`/cancel` marks queued or running durable continuations for the active session
as cancelled. It does not kill arbitrary host processes; cancellation still
depends on the runtime worker or provider wait path observing the persisted
cancel state.
`/screen` renders a deterministic workbench frame for explicit inspection, while
bare `ikaros` uses the normal terminal scrollback with an inline composer.
Non-TTY runs keep the deterministic
snapshot/protocol output used by scripts and smoke tests. After each interactive
turn and slash command, the terminal UI keeps the human transcript and composer
separate from deterministic `/screen` output. While a streamed turn is running,
the cached screen also inserts the submitted user message as `user turn=pending`
before model deltas arrive, so explicit screen inspections can show the active
input immediately. The `--fullscreen` flag remains a diagnostic rendering mode
that wraps a single refresh in an alternate-screen terminal envelope with cursor
hide/show and clear/home control sequences; `--inline` returns to the
script-friendly frame.
Its `--focus`, `--scroll`,
and `--select` flags are a first navigable-screen slice for timeline/main/side
panels; repeated `/screen` commands keep focus, scroll state, and independent
selected rows for the active workbench session. Each refresh prints
`screen_selected` with the focused cell's panel, row, kind, title, and detail so
scripts and replay views can inspect the selected cell without scraping the
frame. It also prints `screen_selected_actions` with direct follow-up commands
for timeline, trace, debug, provider inspection, approval, cancellation, and
queue operations where the selected cell contains enough evidence, followed by
`screen_selected_actions_json` with the same panel, row, kind, and command list
as a machine-readable payload for TUI key binding, ACP consumers, and replay
tooling.
Each refresh also emits `screen_json`, a redacted full-screen snapshot with
status cells, timeline/main/side panel cells, focus/scroll/selection state, the
selected cell, and its safe follow-up commands. The payload declares
`schema=ikaros-workbench-screen-v1`, `version=1`, a compact `key_bindings`
array, and a richer `keymap_model` with grouped bindings for global commands,
panel navigation, composer editing, the command palette, approvals, queue
actions, action-menu selection, timeline tabs, and raw/rich render mode.
The same payload includes a `surface` object with
`schema=ikaros-workbench-surface-v1`. That object is the stable consumer model
for the terminal UI: it contains `bottom_pane_model`, `input_model`,
`input_popup`, `overlay_routing`, `turn_state_model`, `recovery_model`,
`action_menu_model`, timeline grouping, dashboard panels, provider, context,
memory, RAG, coding, approval, and queue panels, readiness, and debug surfaces.
TUI, ACP, and replay consumers should read these structured models instead of
parsing footer text or cell detail strings.
`input_model.context_chips` exposes the currently visible session, memory, and
context state for the composer. Command-palette items also include
`command_class`, `action`, `action_label`, and `visible_state`, so consumers can
show what `/session`, `/context`, or `/memory` will inspect before running it.
The fullscreen footer stays focused on model, workspace, and scroll state. The
active Enter target is exposed through `overlay_routing`, `action_menu_model`,
and the `Selected` panel instead of being parsed from footer text.
The full-screen frame also includes a `Selected` panel with the visible selected
cell's panel, row, kind, title, primary action, and redacted detail. If the
selection is above the current scroll window, the panel reports that no visible
selection is active instead of exposing hidden row content.
`open-selected` executes the first safe read-only follow-up command for the
selected cell. High-risk selected actions such as patch application, rollback,
or live provider probes require `confirm-selected` so Enter does not silently
cross a mutation boundary. Approval, cancellation, and queue mutation actions
remain explicit through `approve-selected`, `deny-selected`, `cancel-selected`,
and `clear-selected`. Line-oriented key aliases are also accepted: `enter` for
`open-selected`, `confirm` for `confirm-selected`, `a` for `approve-selected`,
`d` for `deny-selected`, `c` for `cancel-selected`, and `x` for
`clear-selected`.
Pending approval cells include the approval id, call id, tool, risk, scope,
reason, redacted input preview, and inline approve/deny commands so the side
panel is usable without opening the approval JSON overlay first.
In fullscreen mode the same pending approval is also rendered as a centered TUI
overlay with approve/deny/open key hints; the side panel and JSON payload remain
the structured source for replay and automation.
The tools cell opens `/tools` as a read-only visibility check for direct,
deferred, and disabled tools on the active agent. Live cells also keep stable
tool and context summary rows so long model streams cannot push the current
tool/context state out of view.
`/tools` emits `tools_status_json` with
`schema=ikaros-workbench-tools-status-v1`, `version=1`, active agent, enabled
toolsets, direct/deferred/disabled counts, and sanitized descriptor metadata for
each visible tool.
The main panel includes provider matrix, cost/cache,
health/cooldown/error, and fallback/debug cells so the model backend can be
inspected without leaving the screen; the provider matrix cell carries direct
`/provider matrix`, `--live`, health, debug, and inspect actions. `open-selected`
uses the local read-only provider action by default, while health/fallback rows
open `/provider health` or `/provider debug`; live probes such as
`/provider matrix --live` must be typed explicitly. The main panel projects the
latest coding-turn replay into progress, diff, test, and review cells. Those
cells carry direct follow-up actions for `/code plan`, `/diff`, `/code apply`,
`/code test`, `/code review`, and `/code rollback`. Coding cells use `/diff` as
their safe read-only default; patch, test, review, and rollback actions still
require explicit `/code ...` commands. Interactive `/code apply --diff "..."`
decodes escaped `\n` sequences into newline characters so a unified diff can be
pasted as a single quoted argument. Coding workflow approval requests are
persisted into the session store, and approve/deny decisions emit typed
approval events; `/trace --kind approval` can therefore show the request and
resolution for patch and rollback approvals. `/diff` emits `diff_status_json` with
`schema=ikaros-workbench-diff-status-v1`, `version=1`, git status code,
`has_changes`, redacted stat/error lines, and direct coding workflow actions.
It also projects the latest context diff
into budget, section, reference, and compaction cells; section cells expose the
context contract fields for source, trust level, freshness, scope, token budget,
and injection reason, with secret-like content redacted. The line editor tracks
cursor movement, Home/End, delete, backspace, undo state, and readline-style
Ctrl-P/Ctrl-N/Ctrl-B/Ctrl-F/Ctrl-D shortcuts. Ctrl-U and Ctrl-K delete the text
before or after the cursor, Shift+Enter or Alt+Enter inserts a newline, and
bracketed paste is enabled in raw-mode line input. Control actions emit an
`input_state` line with cursor position, redacted buffer, cursor view, and slash
completion candidates. This is a ratatui/crossterm-backed first slice with
raw-mode input, deterministic screen reducers, single-frame real-TTY diagnostic
drawing, and deterministic non-TTY snapshots, but it is still not a complete
async TUI application.
The side panel includes pending approvals, queued/running continuations, and
the in-memory pending input queue, including `/cancel`, `/queue remove N`, and
`/queue clear` actions where applicable. `approve-selected` and `deny-selected`
resolve the currently selected pending approval, `cancel-selected` cancels the
currently selected queued or running continuation, and `clear-selected` removes
the selected pending input queue item in the side panel, then refreshes the
screen.
`/queue` also emits `pending_inputs_json` after list, add, remove, and clear
operations. The payload declares
`schema=ikaros-workbench-pending-inputs-v1`, `version=1`, `pending_count`,
redacted input `items`, and explicit `/queue remove N` plus `/queue clear`
commands for queue/interrupt panels.
`/cancel` and selected continuation cancellation emit `continuations_json`.
The payload declares `schema=ikaros-workbench-continuations-v1`, `version=1`,
queue status counts, active count, lease owner/expiry, attempt count, terminal
state, redacted continuation payload, and explicit cancel commands for active
queued or running continuations.
Cancelling a queued or running continuation also records a typed
`ContinuationCancelled` event. The payload includes continuation id, kind,
status, redacted reason, attempt count, and lease metadata when present, so
`/trace --kind continuation` and `/timeline --kind continuation` can explain
which continuation was cancelled and why.
Failed interactive turns emit `chat_turn_error_json`. The payload declares
`schema=ikaros-workbench-chat-turn-error-v1`, `version=1`, the failed session,
classified `error_kind` values such as `budget_exceeded` and `provider_error`,
a redacted message, and recovery actions such as `/status`, `/budget`,
`/budget set <tokens>`, `/budget disable`, `/provider debug`, or
`/provider health --live`. The same failure is persisted as a main-panel `latest error`
cell and timeline error cell so `/screen`, `/timeline`, and `/trace --failed`
keep the recovery path visible after the turn aborts.
The approval overlay also emits `approval_overlay_json` with
`schema=ikaros-workbench-approval-overlay-v1`, `version=1`, pending count,
redacted approval context, and explicit approve/deny/replay commands for each
pending item.
After `/approval approve <id>` or `/approval deny <id>`, the workbench prints
`workbench_approval_continue` with the replay status, remaining pending approval
count, and direct `/screen`, `/timeline`, and `/trace` follow-up commands.
The `/session status` command includes the session `state.db`, active leaf,
active branch length, active branch root/leaf, and durable continuation count so
the workbench can explain the current replay branch without requiring JSON
debug output. `/session export [path]` writes a redacted
`ikaros-session-export-v1` artifact for the active session; relative paths are
resolved under the current workspace, and the default path is under
`IKAROS_HOME/exports`.
`/status` emits `workbench_status_json` with
`schema=ikaros-workbench-status-v1`, `version=1`, the active session and
state.db path, agent policy, workspace, model profile and provider health,
budget status, gateway/approval/continuation counts, and direct navigation
actions for screen, timeline, trace, provider debug, approvals, and
cancellation.
`/timeline`, `/replay`, `/debug`, and `/trace` can combine a turn id and
`--kind session|model|tool|context|memory|coding|audit|continuation|approval|error`
to navigate directly to the relevant replay cells or trace spans without
exporting a full trace JSON document. Timeline/replay/debug also accept
`--page N`. Unfiltered timeline/replay/debug pages use the `SessionStore`
paged replay API and print `*_page_source: session_store_page`, so normal
navigation does not load the full event list before slicing. Turn, kind,
failure, and approval filters still scan the session replay to compute the
filtered evidence set. `--failed` jumps to
error/tool-failed/continuation-failed/provider failure diagnostics, and
`--approval` jumps to approval requested/resolved events.
`/commands [query]` prints the human command list and a `commands_json` payload
with each matching command's name, usage, summary, tags, permissions, and
supported surfaces. Full-screen TUI, Gateway, ACP, and other command palettes
should consume this metadata instead of scraping `/help` text.

Chat messages may include local context references such as `@file:path:line-line`,
`@folder:path`, `@git:rev`, `@diff`, and `@staged`. These references are
resolved under the current workspace and recorded in the session context diff.
`@url:` is fetched through the session `NetworkEgress` boundary when the
configured network allowlist permits the exact host and the URL scheme is
`http` or `https`; denied hosts or unsupported schemes fail the turn without
leaking secret-looking URL text. URL responses with explicit content types must
be plain text, Markdown, JSON, XML, or YAML; HTML/binary responses and bodies
larger than 64 KiB become skipped reference notices rather than prompt content.
This is not web search. Use the explicit `web_search` skill for governed search
result metadata, and `web_extract` when a turn needs single-page citation
metadata and HTML text extraction.
`/context` emits `context_status_json` with
`schema=ikaros-workbench-context-status-v1`, `version=1`, context option values,
registered context engine descriptors, latest context budget metadata, section
and reference counts, sanitized prompt section metadata, and compaction status.
It intentionally omits prompt section bodies. Chat uses the deterministic
context engine by default; `--context-engine llm-summary` opts into the
provider-backed summary compressor, and unknown engine names are rejected.

Default chat context uses accepted memory projections and session working
memory. Long-term memory search is not automatic; pass
`--memory-search-limit N` or use the `memory_search` tool when a turn needs
retrieved memory results.
`/memory` emits `memory_status_json` with
`schema=ikaros-workbench-memory-status-v1`, `version=1`, memory backend, policy
thresholds, external provider count, projection file count, pending candidate
count, active working-memory count, journal entry count, and direct memory debug
actions. It does not embed full memory record content.
`/rag` emits `rag_status_json` with `schema=ikaros-workbench-rag-status-v1`,
`version=1`, RAG backend, embedding provider/model, configured `rag_top_k`,
whether ordinary chat injection is active, the local RAG directory, and direct
ingest/search/context actions.

`--context-token-budget 0` asks runtime chat to use the provider-derived
available context window. It does not bypass the model context window.
The persisted context diff records the selected token estimator adapter, such as
OpenAI-compatible, mock, or an explicit Anthropic/Ollama fallback.

Debug persisted session evidence:

```bash
ikaros debug context-diff <session-id>
ikaros debug context-diff <session-id> --turn-id <turn-id>
ikaros debug memory-lifecycle <session-id>
ikaros debug memory-lifecycle <session-id> --turn-id <turn-id>
ikaros debug continuations <session-id>
ikaros debug continuations <session-id> --turn-id <turn-id>
ikaros debug coding-turn <session-id>
ikaros debug coding-turn <session-id> --turn-id <turn-id>
ikaros debug trace <session-id>
ikaros debug trace <session-id> --turn-id <turn-id>
ikaros debug provider
ikaros debug sandbox
ikaros debug insights
ikaros debug logs
ikaros debug logs --source model-usage --page-size 20
ikaros debug logs --source trace --page-size 20
ikaros debug dump --output /tmp/ikaros-debug-dump.json
ikaros debug state-db
ikaros debug state-db --checkpoint
ikaros debug state-db --backup /tmp/ikaros-state-backup.db
ikaros debug state-db --repair /tmp/ikaros-state-repair.db
ikaros debug state-db --restore /tmp/ikaros-state-backup.db
ikaros debug state-db --prune-ended-before 2026-01-01T00:00:00Z --vacuum
```

`context-diff` reads `state.db` and reports the estimator, budget, context
window, context section token estimates, prompt section source/priority/token
metadata, added/removed/compressed context, parsed references, compaction
summary, continuation prompt, `ContextCompacted`, and context-limit errors.
`memory-lifecycle` reads the session timeline and
`memory_journal.jsonl` for matching `MemoryLifecycle` events,
`MemoryRef::SessionTurn` links, skipped writes, redaction-related notes, action
counts, and runtime memory policy actions. `continuations` reports durable
continuation queue status, status reason, lease owner, lease expiry, attempt
count, terminal summaries, worker-lease timeout evidence, errors, and redacted
payload data for queued/running/completed/failed/cancelled items. Filtering by
`--turn-id` returns an empty result when the turn exists but has no
continuations; it only errors when the turn is absent from the replay.
`coding-turn` reports persisted `CodingTurn` events, coding event-kind counts,
review findings, and custom session entries produced by `ikaros code workflow`.
`trace` exports a redacted `ikaros-trace-v1` JSON view of a session or turn,
including event category counts, turn spans, ordered event summaries, entries,
and approval counts without prompt text or secret-looking payloads.
`provider` exports a redacted `ikaros-provider-debug-v1` view of the active
model row, fallback chain, profile source, cache policy, cost metadata, health,
and readiness hints. `sandbox` exports the current local sandbox/debug report
and the isolation matrix, including the configured Docker image and mount points
when `execution.sandbox.backend: docker` is active. Docker-backed process
execution is available as a first slice, but it is not a full VM or
multi-tenant sandbox.
`logs` exports a redacted `ikaros-logs-v1` view over local `audit.jsonl`,
`model-usage.jsonl`, and `logs/trace.jsonl` records with source filtering and
pagination. CLI startup writes structured tracing events to `logs/trace.jsonl`;
`RUST_LOG` can narrow or expand the default Ikaros-focused filter.
`insights` exports a redacted `ikaros-debug-insights-v1` operational summary
that combines config validation, `state.db` integrity, provider readiness,
audit/model-usage/trace counts, cache token accounting, recent redacted log
samples, gateway queue state, and alert rows for items that need attention.
`dump` writes a redacted `ikaros-debug-dump-v1` support artifact containing
state database health, recent log entries, sandbox status, paths, and active
agent identity.
`state-db` reports SQLite operational state, WAL checkpoint status, integrity
checks, write policy, and search-index availability. `--backup` writes a raw
backup artifact, `--repair` writes a fresh integrity-checked artifact, and
`--restore` verifies the source database, writes a pre-restore safety backup of
the current database, replaces `state.db`, clears stale WAL sidecars, and then
reports the restored integrity check. `--prune-ended-before` removes only
sessions with `ended_at` earlier than the RFC3339 cutoff before an optional
`--vacuum`.

Memory and relationship notes:

```bash
ikaros memory add "note" --kind project --scope ikaros
ikaros memory add \
  --kind relationship \
  --scope default \
  --observer alice \
  --subject bob \
  "Bob likes pancakes"
ikaros memory search "query"
ikaros memory update <id> --content "new note"
ikaros memory delete --id <id>
ikaros memory projection render --scope ikaros
ikaros memory projection show --scope ikaros
ikaros memory candidate list
ikaros memory candidate accept <candidate-id> --reason "explicit user instruction"
ikaros memory candidate accept <candidate-id> \
  --supersedes <memory-id> \
  --reason "user corrected this"
ikaros memory candidate reject <candidate-id> --reason "temporary task scope"
ikaros memory supersession <memory-id>
ikaros memory working list --session <session-id>
ikaros memory working prune
ikaros relationship remember "preference" --scope user
ikaros relationship show --scope user
```

Runtime chat writes safe turn state into session working memory, not long-term
`Task` memory. Automatic relationship observations are pending candidates until
accepted. Projection commands render the accepted long-term memory surface used
by chat context. `memory update` returns the updated record plus a
`change_report` with `changed_fields`, `before`, and `after` summaries for the
content and tag fields.

RAG:

```bash
ikaros rag ingest docs --scope project
ikaros rag search "harness policy"
ikaros rag stale
ikaros rag reindex docs --scope project
ikaros rag delete-path docs/old.md
ikaros rag delete-scope scratch
```

When RAG uses `openai-compatible` or `ollama` embeddings, `ingest`, `reindex`,
and `search` may return an approval id before the provider call. After approval,
the original request is replayed and provider-backed embedding HTTP is routed
through the session `ExecutionEnv` / `NetworkEgress` boundary. Local
`hash`/`sparse`/`mock` embeddings do not need network approval. `ikaros doctor`
reports whether the configured embedding provider uses network egress and
whether the embedding base URL is configured.

Tasks and agents:

```bash
ikaros task run "summarize the repository" --dry-run
ikaros task run "inspect runtime" --agent-loop
ikaros agent list
ikaros agent show plan
ikaros agent run --profile plan --dry-run "inspect docs"
ikaros agent run --profile plan --agent-loop --parent-session <session-id> "inspect docs"
ikaros agent batch --profile plan --task "inspect docs" --task "inspect runtime"
```

`--parent-session` records delegated agent-loop work as a child session and,
when the parent session is in the same agent store, appends a redacted
`subagent_result` entry back to the parent timeline.

Policy and approvals:

```bash
ikaros policy explain write_note --risk local-write --path note.txt --write
ikaros approval list
ikaros approval approve <approval-id>
ikaros approval deny <approval-id>
```

Gateway and schedules:

```bash
ikaros schedule add "summarize status" --at now
ikaros schedule add "summarize status" --at now --delivery gateway-outbox
ikaros schedule add "summarize status" \
  --at now \
  --retry-max-attempts 3 \
  --retry-backoff-seconds 60 \
  --grace-period-seconds 300 \
  --timezone UTC
ikaros schedule run-due --dry-run
ikaros schedule worker --once
ikaros message send "hello" --kind chat
ikaros message send "hello" --kind chat --source telegram --account acct --peer peer --thread thread
ikaros message status
ikaros message cancel <id> --reason "operator requested cancel"
ikaros message delivery claim --limit 5 --owner telegram-adapter
ikaros message delivery ack <delivery-id> --lease-owner telegram-adapter --summary "sent"
ikaros message delivery fail <delivery-id> \
  --lease-owner telegram-adapter \
  --reason "remote timeout" \
  --backoff-seconds 30 \
  --max-attempts 3
ikaros message pairing create --source telegram --account bot-account --peer user-id
ikaros message pairing list
ikaros message drain --dry-run
ikaros message daemon start --interval-seconds 5 --limit 10
ikaros message daemon status
ikaros message daemon stop --reason "maintenance"
ikaros message daemon restart --interval-seconds 5 --limit 10
ikaros message webhook --port 8002
ikaros message webhook --port 8002 --hmac-secret "$IKAROS_WEBHOOK_SECRET"
ikaros message webhook --port 8002 --allow-source telegram --allow-peer user-id
ikaros message webhook --port 8002 --require-pairing
ikaros message webhook --port 8002 --unsafe-tools
```

`message status` reports
pending/processing/processed/failed/cancelled/dead-letter counts plus delivery
pending/processing/delivered/dead-letter counts and a redacted worker snapshot:
active lease owner/expiry/attempts, stale-processing count, retryable message
and delivery counts with last errors, and dead-letter terminal evidence.
`message cancel` moves pending or processing messages to a terminal cancelled
state so stale worker claims cannot later deliver results. `message daemon`
starts, stops, restarts, and reports the local long-running worker that reuses
the same runtime and harness path as `message worker`.
`message delivery claim|ack|fail` is the lease-bound adapter surface for outbox
delivery retry/backoff. `message webhook --hmac-secret` requires
`X-Ikaros-Signature: sha256=<hex-hmac>` over the raw request body before a
message is enqueued. `message webhook --allow-source|--allow-account|--allow-peer|--allow-thread`
rejects non-matching adapter payloads before enqueueing.
`message pairing create` issues a one-time code for a source/account/peer;
`message webhook --require-pairing` requires that peer to be paired before
enqueueing, or accepts a valid `pairing_code` once to complete the binding.
Webhook messages default to safe-tools mode, which limits drained chat/task
agent loops to the `core` toolset; `--unsafe-tools` opts out for trusted local
adapters.

Voice and body surfaces:

```bash
ikaros voice tts "hello" --output speech.wav
ikaros voice asr input.wav --language en
ikaros body status
ikaros body dashboard
ikaros body dashboard --refresh-seconds 5 --snapshot-output previews/frame.json
ikaros body serve --port 8001
ikaros browser launch --headless --url https://example.com
ikaros browser supervisor-status
ikaros browser stop
ikaros browser status --endpoint http://127.0.0.1:9222
ikaros browser list --endpoint http://127.0.0.1:9222
ikaros browser new https://example.com --endpoint http://127.0.0.1:9222
ikaros browser activate <target-id> --endpoint http://127.0.0.1:9222
ikaros browser close <target-id> --endpoint http://127.0.0.1:9222
ikaros browser navigate <target-id> https://example.com --endpoint http://127.0.0.1:9222
ikaros browser snapshot <target-id> --endpoint http://127.0.0.1:9222
ikaros browser click <target-id> 100 200 --endpoint http://127.0.0.1:9222
ikaros browser type <target-id> "hello" --endpoint http://127.0.0.1:9222
ikaros browser scroll <target-id> --y 600 --endpoint http://127.0.0.1:9222
ikaros browser screenshot <target-id> --format png --endpoint http://127.0.0.1:9222
ikaros browser cdp <target-id> Runtime.evaluate --params-json '{"expression":"location.href"}'
```

Cloud TTS and ASR calls follow the same approval flow. TTS output renders byte
length and optional file path, not raw audio bytes.
`browser launch`, `browser supervisor-status`, and `browser stop` are the first
local browser supervisor slice. Supervisor state lives under
`IKAROS_HOME/browser/supervisor`, and launch uses a profile-specific browser
data directory unless `--user-data-dir` is provided.

The CDP commands call a local or configured Chrome DevTools endpoint through the
active session `NetworkEgress` boundary and print redacted JSON. HTTP discovery
commands cover status, target listing, new target creation, activation, and
close. WebSocket CDP commands cover navigate, snapshot, click, type, scroll,
screenshot, and raw method calls. The governed request is the CDP control
request; page network traffic is still performed by the browser process until a
stricter browser sandbox exists.

Local filesystem and git helpers:

```bash
ikaros fs read README.md
ikaros fs list docs
ikaros fs write notes/example.txt "local note"
ikaros git status
ikaros git diff --stat
```

Plugins:

```bash
ikaros skill list
ikaros skill audit
ikaros skill validate ./plugins/example
ikaros skill install ./plugins/example
ikaros skill inspect example.tool
ikaros skill run example.tool --input-json '{"message":"hello"}'
ikaros skill run web_search --input-json '{"query":"ikaros runtime"}'
ikaros skill run web_extract --input-json '{"url":"https://example.com"}'
```

`ikaros skill list` prints each built-in skill's toolset and current
model-visibility (`direct`, `deferred`, or `disabled`) for the selected agent
profile. The workbench `/tools` command shows the same direct, deferred, and
disabled surface for the active profile, and emits the same grouped surface as
`tools_status_json` for workbench and ACP consumers. `web_search` is a direct
network skill backed by a governed provider. The default is DuckDuckGo HTML, and
configured runs may use Brave, Bing, SerpAPI, or Tavily-compatible endpoints
through `providers.search` or per-call overrides. It returns result titles,
URLs, snippets, and citation metadata without fetching result pages.
`web_extract` is a direct network skill for one URL: it accepts only
`http`/`https`, routes through the session `NetworkEgress` policy, enforces
retained output caps, returns citation metadata, redacts secret-looking text,
and skips unsupported content types instead of returning binary content.
Deferred RAG, coding, voice, and plugin tools are only discoverable through `tool_search` with a
non-empty query, describable through `tool_describe`, and callable through
`tool_call` when their toolset is enabled for the active agent;
`tool_call` also requires the target to have been disclosed by `tool_search` or
`tool_describe` in the same execution session. The target tool still runs
through harness policy, approval, and audit.

Coding helpers:

```bash
ikaros repo scan
ikaros test infer
ikaros test run --command "cargo test"
ikaros code plan "add focused tests" \
  --diff "<unified diff>" \
  --session-id <session-id> \
  --turn-id <turn-id>
ikaros code apply "apply candidate patch" \
  --diff "<unified diff>" \
  --session-id <session-id> \
  --turn-id <turn-id>
ikaros code test "run focused tests" \
  --test-command "cargo test -p ikaros-coding" \
  --session-id <session-id> \
  --turn-id <turn-id>
ikaros code review --diff "<unified diff>" --session-id <session-id> --turn-id <turn-id>
ikaros code rollback <session-id> --turn-id <turn-id> --rollback-turn-id <rollback-turn-id>
ikaros code workflow "provider loop" \
  --mode edit \
  --model-loop \
  --apply-patch \
  --run-tests \
  --max-iterations 2 \
  --test-command "cargo test"
ikaros code iterate
ikaros code guarded-edit "apply approved patch" --diff "<unified diff>"
```

`code plan`, `code apply`, `code test`, `code review`, and `code rollback` are
the terminal-first coding commands. They are thin routes into the same governed
`code workflow` turn, so they share approval behavior, `ExecutionEnv` writes,
test-matrix evidence, and persisted `CodingTurn` replay. `code rollback` reads
the target turn's last `diff_updated` event from `state.db`, constructs the
reverse unified diff, and submits it as a new approved edit turn.

`code workflow` remains the full low-level surface. It builds a
`CodingTurnContext`, repo map, change plan, optional
patch attempt, turn diff, test-matrix evidence, review, iteration plan, loop
report, and final report. It supports `--mode plan|edit|review|test|self_modify`.
The mode policy is explicit: `plan`/`review` are read-oriented, `test` can run
the test matrix, `edit` can apply a candidate patch only with `--apply-patch`,
and `self_modify` is rejected by ordinary workflow until the dedicated
self-modify approval path is used. The context captures a git baseline including
HEAD, branch/detached state, clean/dirty/not-git/unknown state, and
staged/unstaged/untracked flags when available. When session and turn ids are
present, coding events are persisted into `state.db` and can be inspected with
`ikaros debug coding-turn`. With `--model-loop`, the workflow uses the
configured model provider to request strict JSON candidate patches. The approved
execution path records model request/response metadata, token-budget stops,
cancellation stops, patch attempts, test evidence, review findings, and loop
termination as replayable coding events. `--max-iterations` is bounded to
`1..=8`; `--model-token-budget` stops before a provider request when the
estimated request would exceed the remaining coding-loop budget. Workspace
instructions are loaded from `IKAROS.md` and `.ikaros/instructions.md` when
present. Approval requests for coding turns include structured provider,
shell/test, workspace-write, session, and replay context; the CLI prints that as
`approval_scope`. Coding execution prints `coding_progress` and `coding_result`
summaries, and provider-backed coding turns can be interrupted while waiting for
the provider response with Ctrl-C.

Service-manager templates:

```bash
ikaros service render --kind schedule-worker --manager systemd
ikaros service render \
  --kind message-worker \
  --manager systemd \
  --output services/ikaros-message-worker.service
ikaros service render --kind message-webhook --manager launchd
```

Systemd `message-worker` templates use foreground `message worker` for
`ExecStart` and add an `ExecStop` hook that writes a cooperative
`message-worker.stop` request.

Self-modify:

```bash
ikaros self-modify propose --kind documentation-patch --target README.md --diff "<unified diff>"
ikaros self-modify request-apply <proposal-id>
ikaros self-modify apply-approved <proposal-id> --approval-id <approval-id>
ikaros self-modify rollback <proposal-id>
```

## Global Options

`--ikaros-home <path>` selects local state.

`--agent <profile>` selects the active profile for commands that create a
harness session. It can be placed before or after the subcommand:

```bash
ikaros --agent plan chat --message "read only"
ikaros chat --agent plan --message "read only"
```

## Compatibility

CLI output is primarily human-readable. Automation should depend on structured
report fields that are covered by tests.

After upgrading Ikaros, rerun the relevant validation commands to confirm that
the fields you depend on still match expectations.
