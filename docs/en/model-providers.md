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

`ModelRequest` carries model name, messages, optional max tokens, optional
temperature, and optional tool definitions. `ModelResponse` carries provider,
model, content, usage, and normalized tool calls. `ModelStreamEvent` is the
stream protocol consumed by the runtime event layer; `chunks` and `tool_calls`
remain as aggregate fields for existing callers.

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
- `moonshot` and `siliconflow`: aliases/config helpers over the OpenAI-compatible adapter.
- `anthropic` / `claude`: native Anthropic Messages API adapter with `tool_use` parsing.
- `ollama`: local Ollama `/api/chat` adapter with tool call support for models that expose it.

Accepted provider names and aliases:

- OpenAI-compatible: `openai-compatible`, `openai_compatible`, `openai`, `moonshot`, `siliconflow`, `silicon-flow`
- Anthropic: `anthropic`, `claude`
- Ollama: `ollama`, `local-llm`, `local_llm`

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
    rate_limit_per_minute: 60
    daily_token_budget: 100000
```

`api_key` and `base_url` live in the local `IKAROS_HOME/config.yaml` under
`providers.model`. Do not write real keys into tracked files. Aliases such as
`moonshot` and `siliconflow` use the same adapter and are convenience names, not
default vendors.

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
    base_url: "http://127.0.0.1:11434"

model:
  default:
    provider: ollama
    model: llama3.2
    transport: ollama-chat
```

## OpenAI-Compatible Adapter

The OpenAI-compatible adapter owns Chat Completions requests and responses, HTTP client setup, normal completions, SSE stream parsing, tool-call conversion, and the stream tool-call accumulator. It does not own the agent loop.

OpenAI-compatible aliases such as `moonshot` and `siliconflow` use the same
adapter with provider-specific normalization. Kimi K2.6 temperature normalization
is handled in the adapter path so runtime callers do not need provider-specific
branches.

Streaming parses SSE chunks incrementally. Text, reasoning, refusal, native
tool-call, usage, and done markers become typed `ModelStreamEvent` values.
Tool-call deltas are accumulated until a complete normalized tool call can be
reported back to the agent loop, while individual argument deltas are still
emitted as stream events.

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
