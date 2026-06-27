# Configuration

Ikaros stores local state under `~/.ikaros` by default. Use `IKAROS_HOME` or
`--ikaros-home` to isolate a run:

```bash
export IKAROS_HOME=/tmp/ikaros-dev
ikaros --ikaros-home /tmp/ikaros-dev doctor
```

`ikaros init` creates the runtime home and writes one configuration file:
`IKAROS_HOME/config.yaml`. It does not read configuration from repository
example directories.

## Schema Version

Generated configs include a top-level `schema_version: 1`. Runtime loading and
`ikaros config validate` require that field explicitly. Missing or unsupported
schema versions fail before the runtime uses the config.

## Provider Settings

Do not put real API keys in docs, tests, examples, or tracked files. A local
untracked `IKAROS_HOME/config.yaml` may store third-party API keys directly for
ordinary runs and smoke tests.

`ikaros init` writes a minimal model config only:

```yaml
schema_version: 1

model:
  default:
    preset: auto
    model: ""
    api_key: ""
    base_url: ""
```

Fill `model.default.model`, `api_key`, and `base_url` for the common single-model
case. `preset: auto` keeps runtime provider-profile detection enabled; use a
specific preset such as `kimi`, `openai`, `anthropic`, or `ollama` when the
provider is known. Presets expand at config-load time into the provider family,
wire transport, and compatibility profile, so most users never need to write
`provider`, `transport`, or `compat_profile` manually.

`ikaros init --full` writes the complete default YAML, including optional
sections for providers, agent profiles, memory, RAG, voice, gateway, and
execution settings. It is still comment-free YAML.

Model credentials can be written inline under `model.default` or in the shared
pool under `providers.model`. Inline `model.default.api_key` and
`model.default.base_url` take precedence; empty inline values fall back to
`providers.model`. Fallback model entries use their own `api_key` and `base_url`
when present, otherwise they inherit the shared model provider settings.

`providers.embedding`, `providers.tts`, `providers.asr`, and `providers.search`
remain the shared provider settings for those resource types. Plaintext keys
should only live in the local runtime home and must not be committed to the
repository.

`providers.search` supplies defaults for `web_search` when the selected provider
needs a key or endpoint. The built-in `duckduckgo-html` provider can run with an
empty `base_url`; Brave, Bing, SerpAPI, and Tavily-style providers can use
`providers.search.api_key` and `providers.search.base_url`, or per-command
`ikaros web search --provider ... --endpoint ... --api-key ...` overrides.

`ikaros setup` writes the same fields without printing secret values. When one
OpenAI-compatible endpoint handles several resource types, pass
`--reuse-model-provider-for-embedding`, `--reuse-model-provider-for-tts`, or
`--reuse-model-provider-for-asr` together with the resource model flag. The
interactive setup asks the same reuse question after the model provider is
configured.

## Validation

Validate the local runtime config after editing it:

```bash
ikaros config validate
ikaros config show
```

Normal runtime config loading and the explicit validator share the same shape
and semantic checks before returning an `IkarosConfig`. They reject unknown
fields, invalid provider/runtime/transport/backend combinations, missing keys,
URLs, model names, and descriptor-only external memory providers before a remote
call is attempted. Validation output uses field
paths such as `providers.model.api_key`; it reports whether a value is missing
or invalid but never prints secret values.

For automation, use:

```bash
ikaros config validate --json
ikaros config show --json
```

`config show` prints only a redacted runtime summary: provider families, model
ids, storage backends, execution settings, and booleans such as
`model_api_key_configured`. It does not print plaintext credentials or base
URLs. The JSON mode writes only the report to stdout. Invalid configs still
exit non-zero for validation, but the validation report is stable enough for
scripts to read `valid`, `path`, `errors[]`, and `warnings[]`.

## Execution Boundary

Runtime sessions ask `ikaros-host` to build an `ExecutionEnv` from the
`execution` section:

```yaml
execution:
  network:
    enabled: true
    allow_provider_hosts: true
    allowed_hosts: []
    timeout_ms: 30000
  sandbox:
    backend: local
    image: rust:1.85-bookworm
    read_scope: workspace
```

`network.enabled` turns on the HTTP egress backend. Egress remains
deny-by-default: `allow_provider_hosts` adds exact hosts parsed from the active
model provider, configured agent-instance model provider overrides,
embedding/TTS/ASR/search providers, built-in web-search provider defaults, and
local Ollama defaults. `allowed_hosts` adds extra exact host names for future
network-capable tools. Use host names only, not full URLs or `host:port`
strings.
Network egress rejects non-HTTP schemes, private/link-local/multicast literal
IP addresses, and automatic redirects. Effective provider base URLs include
inline `model.default.base_url` values, shared `providers.*.base_url` values,
per-agent instance model provider overrides, and local Ollama defaults.
Built-in web-search defaults currently cover DuckDuckGo HTML, Brave, Bing,
SerpAPI, and Tavily-style endpoints. Explicit loopback hosts remain available
for local providers such as Ollama; ordinary domains that resolve to restricted
addresses are rejected by the HTTP egress transport. Resolved socket addresses
are pinned into the per-request HTTP client after validation to avoid a second
independent DNS lookup for the same request. This is a runtime guardrail, not a
complete OS-level network sandbox.

`sandbox.backend` is `local`, `dry-run`, or `docker`. Local sessions use a
workspace-scoped filesystem/process environment and a governed HTTP egress
backend. Dry-run sessions keep read access but skip file writes and process
execution. Docker sessions run process execution through `docker run`, bind
mount the workspace at `/workspace`, set the process working directory inside
that mount, and start the container with `--network none`; `sandbox.image`
selects the container image. File reads/writes still pass through the
workspace-scoped `ExecutionEnv`, and provider HTTP egress still uses the
governed runtime transport outside the process container. Network egress is
controlled separately by `execution.network.enabled` and the host allowlist; set
`network.enabled: false` when a dry run must also avoid network side effects.
`read_scope` is currently fixed to `workspace`; existing paths are canonicalized
so symlink escapes are rejected for reads and writes.

## MCP Servers

External MCP servers are configured under `mcp.servers` and are opt-in:

```yaml
mcp:
  servers:
    - id: local-tools
      enabled: false
      transport: stdio
      command: /path/to/mcp-server
      args: []
      include_tools: []
      exclude_tools: []
      timeout_ms: 5000
      max_output_bytes: 65536
```

Only `stdio` transport is supported in the current client slice. A configured
server is never trusted as a local command: `ikaros mcp probe <id>` launches it
through the harness process boundary and therefore still uses policy, approval,
audit, workspace scope, timeout, and output caps. `include_tools` and
`exclude_tools` are exact tool-name filters applied to the discovered
`tools/list` report. HTTP MCP transport is intentionally not enabled yet; it
must be implemented through `NetworkEgress`.

Useful commands:

```bash
ikaros mcp status
ikaros mcp status --json
ikaros mcp probe local-tools --force
```

## Agent Profiles

Profiles choose persona overlay and ordinary policy behavior:

```yaml
agent:
  default: build
  profiles:
    build:
      mode: build
      workspace_writes: ask
      shell: allow
      network: ask
      memory_context: true
      rag_context: false
      toolsets: [core, workspace, memory, rag, coding, voice, plugin]
    plan:
      mode: plan
      workspace_writes: deny
      shell: ask
      network: ask
      memory_context: true
      rag_context: false
      toolsets: [core, workspace, memory, rag, coding, voice, plugin]
```

Keep `rag_context` false for ordinary chat. Enable it on a profile, or pass
`--rag-top-k`, when the turn needs cited local reference snippets.
Long-term memory search is also opt-in. Ordinary chat reads accepted memory
projections and session working memory; use `--memory-search-limit` or the
`memory_search` tool when a turn needs retrieved memory results.
`toolsets` controls which skill groups are enabled for the profile. Only the
direct surface, `core`, `workspace`, and `memory`, is injected into the model
tool manifest. `rag`, `coding`, `voice`, and `plugin` are enabled in the
default profiles but remain deferred; model turns discover and call them through
`tool_search`,
`tool_describe`, and `tool_call`. The bridge refuses deferred tools from
toolsets that the active profile did not enable. Target execution still routes
through harness policy, approval, and audit. Profiles that enable any deferred
toolset must also keep `core` enabled, because the bridge tools live in `core`.

Use a profile with:

```bash
ikaros --agent plan chat --message "review only"
ikaros agent run --profile build --dry-run "inspect this repo"
```

Profiles cannot bypass hard denials for destructive commands, direct secret access, protected paths,
publishing actions, workspace-external writes, or self-modification.

## Agent Instances

Agent instances are runtime identities. A profile answers "how should this agent
behave"; an instance answers "which agent is running, in which workspace, with
which state and routing policy".

Example:

```yaml
agent:
  instances:
    repo-build:
      profile: build
      workspace: /home/user/src/project
      state_dir: /home/user/.ikaros/agents/repo-build
      toolsets: [core, workspace, memory, coding]
      providers:
        model:
          api_key: "sk-..."
          base_url: "https://api.example.com/v1"
      model:
        provider: openai-compatible
        runtime: harness-agent-loop
        transport: openai-compatible-chat-completions
        model: repo-specialist-model
      session_policy:
        history_scope: workspace
        allow_session_switch: true
        max_parallel_subagents: 4
        max_delegation_depth: 2
      auth_scope:
        local_only: true
        allow_network: ask
      route_bindings:
        - channel: cli
```

Fields:

- `profile`: profile overlay used for persona and ordinary policy behavior.
- `workspace`: optional workspace override. If omitted, the caller workspace is
  used.
- `state_dir`: optional state directory override. If omitted, the instance uses
  `IKAROS_HOME/agents/<agent_id>`.
- `toolsets`: optional model-visible/deferred toolset allowlist override. If
  omitted, the selected profile's toolsets are used. Keep `core` enabled when
  deferred toolsets such as `rag`, `coding`, `voice`, or `plugin` are enabled.
- `providers.model`: optional model endpoint and key override for this identity.
  If omitted, `providers.model` is used.
- `model`: optional full `ModelConfig` override for this identity. If omitted,
  `model.default` is used.
- `session_policy.history_scope`: `agent`, `session`, or `workspace`.
- `session_policy.allow_session_switch`: whether runtime may switch sessions for
  this identity.
- `session_policy.max_parallel_subagents`: upper bound for concurrent delegated
  work.
- `session_policy.max_delegation_depth`: upper bound for nested agent handoff
  depth; requests above this fail before starting the delegated task.
- `auth_scope.local_only`: whether the identity is local-only by default.
- `auth_scope.allow_network`: network default for this identity.
- `route_bindings`: channel/account/peer/thread bindings used by gateway routing.

Resolution rules:

1. A requested name first matches `agent.instances.<name>`.
2. If no instance exists, the same name is resolved as `agent.profiles.<name>`.
3. Without a requested name, `agent.default` is used.

Approval and audit records should use the resolved `agent_id` from the instance,
not just the profile name.

Chat, TUI, coding model loops, task agent-loop execution, `doctor`, and
`provider inspect|health|matrix` resolve model settings through the active
`AgentInstance`. Embedding, TTS, and ASR resources remain global unless their
own runtime path explicitly adds an instance override.

## Local Stores

JSONL is the default. SQLite is available for larger local stores:

```yaml
memory:
  backend: sqlite
  policy:
    promote_threshold: 0.75
    demote_threshold: 0.35
    forget_threshold: 0.15
    max_records_per_scope: 2000

rag:
  backend: sqlite
  embedding_provider: hash
```

The agent `state.db` session store is the authoritative chat timeline. The
runtime does not write a separate chat history mirror for ordinary chat turns.
History, search, replay, context assembly, and workbench views project from
session replay.

Memory policy fields:

- `promote_threshold`: combined score at or above this value records a
  `promote` action and tags the record as policy-promoted.
- `demote_threshold`: combined score at or below this value records a `demote`
  action and tags the record as policy-demoted.
- `forget_threshold`: combined score at or below this value records a `forget`
  action and deletes the low-score record.
- `max_records_per_scope`: per kind/scope quota. When a turn pushes a scope
  over quota, the lowest-score records are deleted and journaled as `forget`
  actions with a quota reason.

The main local paths are:

- `IKAROS_HOME/memory/`
- `IKAROS_HOME/rag/`
- `IKAROS_HOME/audit/`
- `IKAROS_HOME/automation/`
- `IKAROS_HOME/gateway/`
- `IKAROS_HOME/skills/`

## Model Provider

The minimal config starts with `preset: auto` and empty key, URL, and model
fields. A remote model call fails before the network request until all required
fields are configured. Supported preset IDs are documented in
`model-providers.md`; supported provider families are `openai-compatible`,
`anthropic`, `ollama`, and `mock`.

Provider calls made by runtime chat, task agent loops, provider-backed coding
commands, and provider-backed RAG embedding skills now go through the session
environment instead of a raw HTTP client. Use `ikaros provider health` to
inspect the local health ledger and `ikaros provider health --live` to run a
real health probe.

Single-provider OpenAI-compatible example using inline credentials:

```yaml
model:
  default:
    preset: kimi
    model: provider-model-id
    api_key: "replace-with-provider-key"
    base_url: "https://api.moonshot.cn/v1"
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
    cost:
      currency: USD
      input_per_million: null
      output_per_million: null
      cache_read_per_million: null
      cache_write_per_million: null
    rate_limit_per_minute: 60
    daily_token_budget: null
    max_retries: 2
```

Preset names are the user-facing provider shortcut. Provider names are adapter
families, not vendor names. Use an OpenAI-compatible preset for any Chat
Completions-compatible service and put the selected endpoint and model in
`model.default.base_url` and `model.default.model`. Multi-provider setups can
move the shared model key and endpoint to `providers.model`; inline model fields
continue to override that shared pool.

`model.default.cost` is local pricing metadata used by `provider inspect`,
`provider matrix`, workbench cost cells, and usage estimates. Ikaros does not
hard-code live vendor pricing. Fill `currency`, `input_per_million`,
`output_per_million`, `cache_read_per_million`, and `cache_write_per_million`
from your provider account when you want cost estimates. Unknown values can stay
`null`; cache read/write pricing falls back to input pricing when those fields
are omitted.

`max_retries` controls the governance retry policy around retryable provider
failures such as rate limits, transient server failures, and network failures.
Authentication, bad request, and context-limit failures are terminal. The
default uses short capped exponential backoff; this is separate from the
OpenAI-compatible adapter's single unsupported-parameter retry.

`compat_profile` controls provider/model request quirks inside the
OpenAI-compatible adapter. Presets fill it automatically. `auto` first matches
the effective model base URL, then model-name hints, then falls back to
`generic`. Supported explicit values are:

- `generic`: standard Chat Completions fields only.
- `moonshot-kimi`: Kimi/Moonshot. Omits `temperature`, defaults missing
  `max_tokens` to `32000`, emits Kimi thinking fields, and sanitizes tool
  schemas to Moonshot's stricter JSON Schema subset.
- `deepseek`: emits DeepSeek thinking fields for `deepseek-reasoner` and
  `deepseek-v4+` models, while leaving `deepseek-chat` V3 unchanged.
- `gemini-openai`: maps reasoning config to Gemini OpenAI-compatible
  `extra_body.google.thinking_config` only for Gemini-family models.
- `openrouter`: keeps OpenRouter routing fields in the final request body and
  avoids invalid reasoning fields for modern Claude routes.
- `qwen`: Qwen/DashScope-compatible request shaping. It normalizes message
  content to text parts, marks the system prompt part as ephemeral cache
  content, enables high-resolution image handling, and uses `65536` as the
  missing `max_tokens` default.
- `local-openai-compatible`: conservative profile for LM Studio, vLLM, SGLang,
  and similar local Chat Completions servers. It uses `65536` as the missing
  `max_tokens` default so local servers that default to very short completions
  do not truncate normal agent turns.

The Moonshot sanitizer is request-only: it repairs the provider payload without
mutating the registered tool schema. See [Model providers](model-providers.md)
for the exact schema repair rules.

OpenAI-compatible profile values are valid only with
`provider: openai-compatible`. Native providers accept `auto`, `generic`, or
their preset-resolved native profile (`anthropic-native`, `ollama-native`, or
`mock`).

For optional numeric `params` fields, `null` means the adapter does not send
that parameter unless the selected profile supplies a provider default.
Supported fields are:

- `max_tokens`: positive output-token cap.
- `temperature`: sampling temperature, validated in the inclusive `0.0..2.0`
  range.
- `top_p`: nucleus sampling value, validated in the inclusive `0.0..1.0`
  range.
- `n`: positive completion count when the provider supports it.
- `presence_penalty` and `frequency_penalty`: validated in the inclusive
  `-2.0..2.0` range.
- `seed`: deterministic seed for providers that support it.
- `stop`: list of non-empty stop sequences.

`reasoning.effort` accepts `none`, `minimal`, `low`, `medium`, `high`,
`xhigh`, or `max`. Runtime code may set per-call options for specific
workflows, but strict profiles still remove or rewrite fields that the target
provider rejects. `extra_body` is a JSON object merged into the provider
request body after common parameters and before profile-specific shaping; logs
and audit records must use redacted summaries rather than raw secret-like
values.

Daily token-budget preflight uses the configured or per-call `max_tokens` when
present. If a selected OpenAI-compatible profile provides a missing
`max_tokens` default, such as `moonshot-kimi`, `qwen`, or
`local-openai-compatible`, that profile default is included in the estimate.

If an OpenAI-compatible provider explicitly rejects `temperature` or an
omittable `max_tokens` field as unsupported, the adapter removes that field and
retries the HTTP request once. Authentication, quota, network, and general
validation failures are not retried through this path.

Anthropic example:

```yaml
model:
  default:
    preset: anthropic
    model: claude-sonnet-4-5
    api_key: "replace-with-anthropic-key"
    base_url: "https://api.anthropic.com"
```

The Anthropic adapter always sends a positive `max_tokens` value. When
`model.default.reasoning` enables thinking, modern Claude models use adaptive
thinking plus `output_config.effort`; legacy Claude models use budget-based
thinking. Claude 4.7 and newer omit sampling fields such as `temperature` and
`top_p`, even if a workflow supplies them.

Ollama local example:

```yaml
model:
  default:
    preset: ollama
    model: llama3.2
    # Optional. Empty uses http://127.0.0.1:11434.
    base_url: ""
```

Ollama can also provide local embeddings:

```yaml
providers:
  embedding:
    api_key: ""
    # Optional. Empty uses http://127.0.0.1:11434.
    base_url: ""

rag:
  embedding_provider: ollama
  embedding_model: nomic-embed-text
```

The Ollama adapter maps `params.max_tokens` to native `options.num_predict`.
It also forwards explicitly configured `temperature`, `top_p`, `seed`, and
`stop` values through the native `/api/chat` `options` object.

Usage records are written under local audit state and do not include prompt text.

## RAG

The default config uses local `hash` embeddings, so RAG does not require a
provider key for local indexing:

```yaml
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small
```

For remote embeddings, use the provider settings explicitly:

```yaml
providers:
  embedding:
    api_key: ""
    base_url: ""

rag:
  backend: jsonl
  embedding_provider: openai-compatible
  embedding_model: ""
```

Embedding provider names are `hash`, `sparse`, `mock`, `ollama`, and
`openai-compatible`. `hash`, `sparse`, and `mock` are local deterministic/test
adapters implemented by the RAG core. `ollama` and `openai-compatible` are
remote egress adapters implemented by RAG skills, not by `ikaros-rag`. They
require approval through the harness before provider calls; after approval, RAG
skills route the embedding HTTP through session `NetworkEgress`.

External memory providers are descriptor metadata only in the current runtime.
Runtime config loading and `ikaros config validate` reject enabled external
memory providers because remote append/search adapters are not implemented.

## Voice

The default config uses local mock voice providers, so ordinary model chat does
not require TTS or ASR credentials:

```yaml
voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
```

Remote OpenAI-compatible TTS and ASR must be configured explicitly:

```yaml
providers:
  tts:
    api_key: ""
    base_url: ""
  asr:
    api_key: ""
    base_url: ""

voice:
  tts:
    provider: openai-compatible
    model: ""
    voice: default
  asr:
    provider: openai-compatible
    model: ""
```

The only cloud voice provider name is `openai-compatible`; the configured
remote service must actually expose the requested TTS or ASR endpoint. TTS text
is redacted before provider calls; output files are treated as workspace writes.

## Self-Modify Checks

Self-modify apply can use restricted check profiles:

```yaml
self_modify:
  check_profiles:
    runtime_patch:
      commands:
        - cargo check --workspace --all-features
      reason: "Runtime patches must keep the workspace compiling."
```

These checks do not enable autonomous apply. A proposal still needs explicit approval.
