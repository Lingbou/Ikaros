# Model Providers

Model code lives in `ikaros-models`, but providers do not own agent turns. The current boundary has three layers:

- `ModelProvider`: generates/streams model output.
- `ModelTransport`: describes provider wire format, base URL, streaming, and tool-call normalization capability.
- `AgentRuntime`: owns the turn loop, tool continuation, stop reason, and harness dispatch.

The default runtime is `harness-agent-loop`, and the default OpenAI-compatible transport is `openai-compatible-chat-completions`.

## Interface Contract

`ModelProvider` exposes two operations:

- `generate(request)`: returns one `ModelResponse`.
- `stream(request)`: returns a `ModelStream` with text chunks, normalized tool
  calls, final metadata, and typed `ModelStreamEvent` entries.

`ModelRequest` carries messages, typed request options, and optional tool
definitions. Request options include output caps, sampling fields, stop
sequences, reasoning controls, and an `extra_body` object for provider-specific
request fields. The configured provider owns the actual model name.
`ModelResponse` carries provider, model, content, usage, and normalized tool
calls. `ModelStreamEvent` is the stream protocol consumed by the runtime event
layer; `chunks` and `tool_calls` remain as aggregate fields for existing
callers.

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
- `anthropic` currently exposes generate-backed normalized stream events; it is
  not yet a native Anthropic streaming parser.
- `mock` emits deterministic local chunks for tests.

Accepted provider names:

- OpenAI-compatible: `openai-compatible`
- Anthropic: `anthropic`
- Ollama: `ollama`
- Offline tests: `mock`

OpenAI-compatible example:

```yaml
providers:
  model:
    api_key: "replace-with-provider-key"
    base_url: "https://api.example.com/v1"

model:
  default:
    provider: openai-compatible
    model: provider-model-id
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    compat_profile: auto
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
    daily_token_budget: 100000
```

`api_key` and `base_url` live in the local `IKAROS_HOME/config.yaml` under
`providers.model`. Do not write real keys into tracked files. Provider names are
adapter families; do not encode vendor names in `model.default.provider`.

Anthropic example:

```yaml
providers:
  model:
    api_key: "replace-with-your-anthropic-key"
    base_url: "https://api.anthropic.com/v1"

model:
  default:
    provider: anthropic
    model: claude-sonnet-4-5
    transport: anthropic-messages
```

Ollama local example:

```yaml
providers:
  model:
    api_key: ""
    # Optional. Empty uses http://127.0.0.1:11434.
    base_url: ""

model:
  default:
    provider: ollama
    model: llama3.2
    transport: ollama-chat
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

## OpenAI-Compatible Adapter

The OpenAI-compatible adapter owns Chat Completions requests and responses, HTTP client setup, normal completions, SSE stream parsing, tool-call conversion, request profile handling, and the stream tool-call accumulator. It does not own the agent loop.

The OpenAI-compatible provider name is vendor-neutral. Provider and model
differences live in `model.default.compat_profile`, not in extra provider-name
aliases. `auto` selects a profile from `providers.model.base_url` first, then
model-name hints, then `generic`.

Provider-specific profile names are valid only when `model.default.provider` is
`openai-compatible`; native providers accept only `auto` or `generic`.

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

The request builder emits the final raw HTTP JSON body. Do not copy OpenAI SDK
parameter names blindly: SDK `extra_body` entries are merged into the body, so
Kimi `thinking` is a top-level wire field, while Gemini OpenAI-compatible uses
an actual top-level `extra_body.google.thinking_config` field.

When a provider explicitly reports `temperature` or an omittable `max_tokens`
as an unsupported parameter, the adapter removes that one field and retries the
HTTP request once. Other provider errors are returned without automatic
mutation. A successful retry records a `ModelRequestDiagnostic` with
`kind: unsupported_parameter_retry`; responses, streams, audit payloads, and
coding reports can surface that diagnostic without exposing prompts or secrets.

The current adapter reads the provider response body and parses SSE `data:`
lines into typed events. Text, reasoning, refusal, native tool-call, usage, and
done markers become `ModelStreamEvent` values. Tool-call fragments are
accumulated until the complete normalized call is available; `ToolCallStart`, a
single redacted accumulated `ToolCallDelta`, and `ToolCallEnd` are emitted after
that assembly step. This avoids partial tool names and prevents split
secret-like values from leaking through fragment-level redaction. It is not yet
a true network-incremental streaming parser.

## Governance

The governance wrapper handles:

- request redaction before provider adapters
- per-minute request limits
- daily token budget estimates
- prompt-free usage logging
- streaming response usage recording

Usage records live under local audit state and contain provider, model, timestamp, and token counts. They do not store prompts.

Governance wraps provider adapters. It should see the request before the adapter
does, but it should not understand provider-specific wire formats. Redaction,
rate limiting, daily token budget checks, and usage recording therefore apply to
all provider families.

Daily token-budget checks include configured or per-call output caps. When an
OpenAI-compatible profile supplies an implicit output cap, such as Kimi's
`32000` or Qwen/local `65536`, the governance preflight uses that profile
default so strict profiles are not underestimated.

Failures:

- Missing credentials fail before the remote call.
- Provider HTTP errors are reported with redacted response bodies.
- Rate-limit or token-budget failures stop before the provider call.
- Usage logging failures should not expose prompt text.

## Tools

`ModelRequest` can include tool definitions. OpenAI-compatible and Ollama providers serialize them as function tools and parse native `tool_calls` back into `ModelResponse`. Anthropic serializes them as Messages API tools and parses `tool_use` blocks.

The runtime agent loop consumes native tool calls first and preserves native tool call/tool result history when a provider returns IDs. If a provider returns plain text, the loop can fall back to the internal JSON tool-call protocol.

Tool-call normalization rules:

- Provider-native names and JSON arguments become `ModelToolCall`.
- Provider call ids are preserved when present.
- Invalid or missing argument JSON is converted to an empty object only when the
  adapter can do so deterministically.
- Provider-specific tool result history is built by the runtime/model-turn layer,
  not by skills.

Adapters should prefer native tool calls over prompting the model to emit JSON.
The fallback JSON protocol is an agent-loop compatibility path for providers or
models that return plain text.

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
