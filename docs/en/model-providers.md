# Model Providers

Model code lives in `ikaros-models`, but providers do not own agent turns. The
current boundary has three layers:

- `ModelProvider`: generates/streams model output.
- `ModelTransport`: describes provider wire format, base URL, streaming, and
  tool-call normalization capability.
- `AgentRuntime`: owns the turn loop, tool continuation, stop reason, and
  harness dispatch.

The default runtime is `harness-agent-loop`, and the default
OpenAI-compatible transport is `openai-compatible-chat-completions`.

## Interface Contract

`ModelProvider` exposes two operations:

- `generate(request)`: returns one `ModelResponse`.
- `stream(request)`: returns a `ModelStream` with text chunks, normalized tool
  calls, final metadata, and typed `ModelStreamEvent` entries.
- `context_profile()`: returns provider-aware context-window metadata used by
  runtime context budgeting.

`ModelRequest` carries messages, typed request options, and optional tool
definitions. Request options include output caps, sampling fields, stop
sequences, reasoning controls, and an `extra_body` object for provider-specific
request fields. The configured provider owns the actual model name.
`ModelResponse` carries provider, model, content, usage, and normalized tool
calls. `ModelStreamEvent` is the stream protocol consumed by the runtime event
layer; `chunks` and `tool_calls` remain as aggregate fields for existing
callers.

`ModelContextProfile` records the context window, default output-token
reservation, tokenizer kind, and metadata source. Runtime uses it to cap
`ContextBudget` before context assembly and to choose a context token estimator.
OpenAI-compatible and mock providers have deterministic local estimators;
Anthropic and Ollama currently select explicit fallback estimators.

`ProviderRegistry` supplies local provider descriptors for inspection and
runtime planning. A descriptor contains provider family, resolved profile,
capabilities, context metadata, cost fields, prompt-cache policy, and health
state. This registry is
local metadata: `provider inspect` does not call the remote provider. Runtime
provider calls write `provider-health.jsonl` health records, enforce durable
cooldown after repeated retryable failures, and expose a fallback-chain
`ModelProvider` primitive for ordered backup providers. Fallbacks are configured
on `model.default.fallbacks`; each fallback entry carries its own provider,
transport, model, optional profile, request params, and `api_key` / `base_url`.
The runtime builds one governed provider whose primary entry is
`model.default` and whose later entries are tried only for retryable provider
failures. When a fallback handles a request, the returned response/stream
carries redacted diagnostics for the failed provider(s) and the selected
fallback provider, so replay/debug surfaces can explain failover without
exposing prompts or secrets.
When the governance wrapper retries the same provider after a retryable error,
the successful response/stream also carries redacted `provider_retry_*`
diagnostics. Retry diagnostics include the base retry delay, applied jitter,
final retry delay, and any `Retry-After` header or body hint that shaped the
delay. Retry and fallback diagnostics are metadata only; prompt content and
secret-like error fragments are redacted before they reach reports or logs.
The diagnostic boundary also caps field sizes before events, audit payloads, or
coding reports can persist them: diagnostic kind is limited to 96 characters,
message to 512 characters, and parameter to 128 characters.
The same paths emit structured `tracing` events for provider request start,
retry failure, terminal failure, completion, fallback failure, and fallback
selection. Those trace fields use the same redaction boundary and contain
provider/model metadata, attempt counts, error class, and diagnostic kind, not
prompt text.

Each diagnostic becomes a typed `ModelDiagnostic` `AgentEvent` during the
agent loop and lands in the session timeline alongside the matching model
turn. `ikaros debug trace` surfaces the diagnostic kind under
`diagnostic_kind` for every `model_diagnostic` event, and the workbench
`/trace` view and live cells render the same kind. Replay and debug never
print prompt content or the raw error; only the redacted provider, model,
attempt, and error classification appear.
The agent loop also emits a structured `agent_loop_model_result` trace event
with the same `session:<session_id>:turn:<turn_id>` correlation id used by
debug/replay output.

Providers must not:

- dispatch tools
- approve policy requests
- write memory or RAG state
- mutate workspaces
- store prompts in usage logs

Those actions belong to runtime, harness, memory/RAG stores, or governance.

`ModelTransportDescriptor` records the provider family, transport name, base URL,
whether streaming is supported, and whether native tool-call normalization is
supported. Runtime selection uses this metadata to avoid coupling the agent loop
to provider-specific JSON.

## Providers

Implemented:

- `mock`: deterministic local provider for explicit offline tests.
- `openai-compatible`: Chat Completions adapter.
- `anthropic`: native Anthropic Messages API adapter with `tool_use` parsing.
- `ollama`: local Ollama `/api/chat` adapter with tool call support for models that expose it.

Streaming status:

- `openai-compatible` parses provider SSE response bodies into rich
  `ModelStreamEvent` records.
- `ollama` parses `/api/chat` streaming JSON lines into the same event shape.
- `anthropic` sends `stream: true` Messages requests and parses native
  Anthropic SSE events for text deltas, tool-use JSON deltas, usage, and cache
  accounting. The current HTTP client still reads the completed response body
  before parsing; true socket-level incremental delivery is a later transport
  hardening step.
- `mock` emits deterministic local chunks for tests.

Accepted provider names:

- OpenAI-compatible: `openai-compatible`
- Anthropic: `anthropic`
- Ollama: `ollama`
- Offline tests: `mock`

## Model Presets

`model.default.preset` is the user-facing shortcut for selecting a provider
service. It expands during config loading into the provider family, transport,
and compatibility profile. Explicit `provider`, `transport`, or
`compat_profile` fields still override the preset when an advanced config needs
to do so.

Preset summary:

- `auto`: OpenAI-compatible Chat Completions, `auto` profile, no fixed base URL.
- `openai`: OpenAI-compatible Chat Completions, `generic` profile,
  `https://api.openai.com/v1`.
- `kimi`: OpenAI-compatible Chat Completions, `moonshot-kimi` profile,
  `https://api.moonshot.cn/v1`.
- `deepseek`: OpenAI-compatible Chat Completions, `deepseek` profile,
  `https://api.deepseek.com`.
- `gemini`: OpenAI-compatible Chat Completions, `gemini-openai` profile,
  `https://generativelanguage.googleapis.com/v1beta/openai`.
- `openrouter`: OpenAI-compatible Chat Completions, `openrouter` profile,
  `https://openrouter.ai/api/v1`.
- `qwen`: OpenAI-compatible Chat Completions, `qwen` profile,
  `https://dashscope.aliyuncs.com/compatible-mode/v1`.
- `local-openai`: OpenAI-compatible Chat Completions,
  `local-openai-compatible` profile, `http://127.0.0.1:8080/v1`.
- `ollama`: native Ollama chat, `ollama-native` profile,
  `http://127.0.0.1:11434`.
- `anthropic`: native Anthropic Messages, `anthropic-native` profile,
  `https://api.anthropic.com`.
- `mock`: mock provider and mock profile.

For a single provider, keep credentials inline under `model.default`:

```yaml
model:
  default:
    preset: kimi
    model: kimi-k2-0711-preview
    api_key: "replace-with-provider-key"
    base_url: "https://api.moonshot.cn/v1"
```

For multi-provider or shared-resource setups, put shared model credentials in
`providers.model`; inline `model.default.api_key` and
`model.default.base_url` still take precedence when present.

Inspect the resolved local descriptor with:

```bash
ikaros provider inspect
ikaros provider health
ikaros provider health --live
ikaros provider matrix
ikaros provider matrix --live
ikaros provider profiles
```

The command reads `IKAROS_HOME/config.yaml`, resolves the configured provider
family/profile, and prints context window, default output reservation,
tokenizer, capabilities, profile policy fields, health state, and cost fields
for input, output, cache-read, and cache-write tokens when metadata is known.
For OpenAI-compatible providers, the profile policy fields show the resolved
wire behavior for temperature, reasoning, message normalization, tool schema
normalization, extra request-body handling, and prompt-cache handling.
Qwen-style system cache markers are reported as `qwen-system-ephemeral`;
providers without a stable cache policy report `none`. It redacts model values that
look secret-like and never prints the API key. When `model.default.fallbacks`
is configured, `provider inspect` also prints a `fallback_count` and one
`fallback_row` per fallback with the resolved provider/profile/readiness and
capability summary. `provider health` reads the local health ledger. `provider
health --live` sends a short real request through the session `NetworkEgress`
boundary and records success or failure in the same ledger.
`provider matrix` renders the configured model, embedding, TTS, and ASR provider
rows with descriptor metadata, local readiness checks, redacted credential
presence, latest local health status, cooldown metadata, capability flags,
profile policy fields, context fields, input/output/cache-read/cache-write
cost fields, fallback role, fallback count/model list, and a short
`debug_hint`. The chat workbench `/model` view reuses the same descriptor
surface and prints configured fallback rows from the active runtime model.
`provider matrix --live`
probes model, embedding, TTS, and ASR rows: model and remote embedding probes
use runtime `NetworkEgress`, local embedding probes use the local RAG store, and
TTS/ASR probes use the configured voice providers.
Cost fields are registry metadata overlaid with `model.default.cost` from
`config.yaml`; leave values unknown or `null` when the provider invoice does not
separate regular input, output, prompt-cache read, and prompt-cache write tokens.
`provider profiles` prints the static OpenAI-compatible profile catalog. Each
profile `id` is the resolver key for explicit `model.default.compat_profile`
values; the same row also includes auto-detection hints, capability flags,
context metadata, and request-shaping policy fields. The chat workbench exposes
the same catalog with `/provider profiles`.
`ikaros debug provider` and the workbench `/provider debug` command expose a
redacted structured provider diagnostic view, including profile source,
prompt-cache policy, cost metadata, health, fallback rows, and live-smoke
readiness hints without making a live provider call.

OpenAI-compatible example:

```yaml
model:
  default:
    preset: kimi
    model: provider-model-id
    api_key: "replace-with-provider-key"
    base_url: "https://api.moonshot.cn/v1"
    fallbacks:
      - preset: ollama
        runtime: harness-agent-loop
        model: qwen2.5-coder:7b
        base_url: "http://127.0.0.1:11434"
        api_key: ""
    params:
      max_tokens: null
      temperature: null
      top_p: null
      n: null
      presence_penalty: null
      frequency_penalty: null
      seed: null
      stop: []
    reasoning:
      enabled: null
      effort: null
    extra_body: {}
    rate_limit_per_minute: 60
    daily_token_budget: null
```

Do not write real keys into tracked files. Preset names are the normal user
surface; provider names are adapter families and should only be written when an
advanced config intentionally overrides preset expansion.

Anthropic example:

```yaml
model:
  default:
    preset: anthropic
    model: claude-sonnet-4-5
    api_key: "replace-with-your-anthropic-key"
    base_url: "https://api.anthropic.com"
```

Anthropic request shaping sends system prompt content as Anthropic content
blocks. The first non-empty system block carries an ephemeral `cache_control`
marker so Claude-family providers can reuse a stable system prefix when the API
supports prompt caching. Runtime chat builds separate system messages for
cache-stable persona, policy, and tool guidance versus dynamic history,
references, memory, RAG, and compaction notices. The single-call path sends
those layers directly; the harness agent-loop path preserves the same split and
adds its tool-call protocol to the first cache-stable system message. The
cache-marked prefix therefore stays byte-stable across turns. Anthropic usage
metadata is mapped into
`TokenUsage.cache_write_tokens` from `cache_creation_input_tokens` and
`TokenUsage.cache_read_tokens` from `cache_read_input_tokens`; workbench status
and usage logs can explain cache accounting separately from ordinary input and
output tokens.

Ollama local example:

```yaml
model:
  default:
    preset: ollama
    model: llama3.2
    # Optional. Empty uses http://127.0.0.1:11434.
    base_url: ""
```

Anthropic and Ollama are native adapters, not OpenAI-compatible profiles.
Anthropic resolves a positive Messages API `max_tokens` value locally, maps
configured reasoning to Claude adaptive or manual thinking, and strips
sampling fields for Claude 4.7+ families that reject them. Ollama maps
`params.max_tokens` to `options.num_predict` and forwards explicitly set
`temperature`, `top_p`, `seed`, and `stop` through `/api/chat` options.

The same typed `ModelRequestOptions` shape is used by runtime workflows and
provider adapters. Config defaults are merged with per-call options before the
adapter builds a wire payload. Adapter profiles may still omit, rewrite, or add
fields when the target provider requires a different shape.

Runtime chat, task agent loops, provider-backed coding workflows, and
provider-backed RAG embedding skills construct network-capable providers through
the session environment. The adapters still own wire payloads, but actual
network I/O is governed by `ExecutionEnv` host allowlists, redaction, and timeout
policy.

## OpenAI-Compatible Adapter

The OpenAI-compatible adapter owns Chat Completions requests and responses, HTTP client setup,
normal completions, SSE stream parsing, tool-call conversion, request profile handling, and the
stream tool-call accumulator. It does not own the agent loop.

The OpenAI-compatible provider name is vendor-neutral. Provider and model
differences live in `model.default.compat_profile`, not in extra provider-name
aliases. `auto` evaluates the static profile catalog's detection hints
(`base_url` markers, model markers, and model-tail prefixes) in catalog order
and then falls back to `generic`.

OpenAI-compatible profile names are valid only when `model.default.provider` is
`openai-compatible`. Native providers accept `auto`, `generic`, or their
preset-resolved native profile (`anthropic-native`, `ollama-native`, or
`mock`).

Resolved OpenAI-compatible quirks come from a static `ProviderProfile` spec
catalog. The catalog includes both auto-detection hints and request-time
decisions. The request builder and provider registry consume the same resolved
decision for default output tokens, context metadata, temperature policy,
reasoning policy, message policy, tool-schema policy, and extra request body
policy. This keeps provider inspection and wire payload construction from
drifting apart, and it keeps new profile work localized to the catalog instead
of scattering provider branches through the wire builder.

Implemented profiles:

- `generic`: current standard Chat Completions behavior.
- `moonshot-kimi`: omits `temperature`, uses `32000` as the missing
  `max_tokens` default, emits Kimi/Moonshot thinking controls, and sanitizes
  tool schemas to Moonshot's JSON Schema subset.
- `deepseek`: emits thinking controls for `deepseek-reasoner` and DeepSeek V4+
  models, while leaving `deepseek-chat` V3 unchanged.
- `gemini-openai`: maps reasoning options to Gemini OpenAI-compatible thinking
  config for Gemini-family models only.
- `openrouter`: keeps routing/session fields in the final request body and
  avoids invalid reasoning payloads for modern Claude routes.
- `qwen`: normalizes Qwen/DashScope messages to text parts, adds the ephemeral
  cache marker to the system prompt part, enables high-resolution image
  handling, and uses a `65536` missing `max_tokens` default.
- `local-openai-compatible`: conservative profile for local Chat Completions
  servers, with a `65536` missing `max_tokens` default to avoid short local
  completions.

Moonshot schema sanitization is applied only to the outgoing provider request.
It does not mutate the registered tool definition. The sanitizer converts
`oneOf` into `anyOf`, drops null branches and `nullable`, removes unsupported
validation keys such as `title`, `minimum`, `maximum`, and `format`, infers a
missing `type`, and removes scalar enum values that do not match the final
type. Non-object top-level parameters become an empty object schema.

The request builder emits the final raw HTTP JSON body. Do not copy OpenAI SDK
parameter names blindly: SDK `extra_body` entries are merged into the body, so
Kimi `thinking` is a top-level wire field, while Gemini OpenAI-compatible uses
an actual top-level `extra_body.google.thinking_config` field.

When a provider explicitly reports `temperature` or an omittable `max_tokens`
as an unsupported parameter, the adapter removes that one field and retries the
HTTP request once. Other provider errors are returned without automatic
mutation. A successful retry records a `ModelRequestDiagnostic` with
`kind: unsupported_parameter_retry`; responses, streams, audit payloads, and
coding reports can surface that diagnostic without exposing prompts, secrets, or
unbounded provider error bodies.

The current adapter reads the provider response body and parses SSE `data:`
lines into typed events. Text, reasoning, refusal, native tool-call, usage, and
done markers become `ModelStreamEvent` values. Text, reasoning, and refusal
deltas use a small pending-token redactor: content is released only after a
whitespace boundary or final flush, so `sk-`/`token=` style values split across
SSE chunks are redacted as one token instead of leaking the tail fragment.
Tool-call fragments are accumulated until the complete normalized call is
available; `ToolCallStart`, a single redacted accumulated `ToolCallDelta`, and
`ToolCallEnd` are emitted after that assembly step. This avoids partial tool
names and prevents split secret-like values from leaking through
fragment-level redaction. It is not yet a true network-incremental streaming
parser.

## Governance

The governance wrapper handles:

- request redaction before provider adapters
- per-minute request limits
- daily token budget estimates
- prompt-free usage logging
- streaming response usage recording
- classified provider errors and retry/backoff for retryable failures

Usage records live under local audit state and contain provider, model,
timestamp, and token counts. They do not store prompts. The usage ledger keeps a
small in-process cache for budget and workbench status reads, refreshes when the
JSONL file changes, and falls back to the last valid cached records if it sees a
crash-left partial trailing line.

Governance wraps provider adapters. It should see the request before the adapter
does, but it should not understand provider-specific wire formats. Redaction,
rate limiting, daily token budget checks, and usage recording therefore apply to
all provider families.

Daily token-budget checks include configured or per-call output caps. When an
OpenAI-compatible profile supplies an implicit output cap, such as Kimi's
`32000` or Qwen/local `65536`, the governance preflight uses that profile
default so strict profiles are not underestimated.

`model.default.max_retries` controls the governance retry policy around the
configured provider. Retryable classes are rate-limit, transient server, and
network failures. Authentication, bad request, and context-limit failures are
terminal. Backoff is exponential with a capped local default; provider-specific
unsupported-parameter retries inside an adapter remain separate and are still
limited to the exact unsupported field.

Failures:

- Missing credentials fail before the remote call.
- Provider HTTP errors are reported with redacted response bodies.
- Rate-limit or token-budget failures stop before the provider call.
- Usage logging failures should not expose prompt text.

## Tools

`ModelRequest` can include tool definitions. OpenAI-compatible and Ollama providers serialize them
as function tools and parse native `tool_calls` back into `ModelResponse`. Anthropic serializes them
as Messages API tools and parses `tool_use` blocks.

The runtime agent loop consumes native tool calls first and preserves native tool call/tool result
history when a provider returns IDs. If a provider returns plain text, the loop can fall back to the
internal JSON tool-call protocol.

Tool-call normalization rules:

- Provider-native names and JSON arguments become `ModelToolCall`.
- Provider call ids are preserved when present.
- Invalid or missing argument JSON is converted to an empty object only when the
  adapter can do so deterministically.
- Provider-specific tool result history is built by the runtime/model-turn layer,
  not by skills.

Adapters should prefer native tool calls over prompting the model to emit JSON.
The fallback JSON protocol is the strict agent-loop text envelope for providers
or models that return plain text instead of native tool calls.

## Live Tests

Live provider tests are ignored by default and require explicit opt-in:

```bash
export IKAROS_RUN_LIVE_MODEL_TESTS=1
cargo test -p ikaros-models --test live_model -- --ignored
```

Live smoke tests read `api_key`, `base_url`, and `model` from the selected local
`IKAROS_HOME/config.yaml` when `model.default` matches the tested provider.
Live smoke tests verify connectivity, basic responses, and usage logging only;
they should not print model content or secrets.
