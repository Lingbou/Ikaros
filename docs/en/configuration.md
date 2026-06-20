# Configuration

Ikaros stores local state under `~/.ikaros` by default. Use `IKAROS_HOME` or `--ikaros-home` to isolate a run:

```bash
export IKAROS_HOME=/tmp/ikaros-dev
ikaros --ikaros-home /tmp/ikaros-dev doctor
```

`ikaros init` creates the runtime home and writes one configuration file:
`IKAROS_HOME/config.yaml`. It does not read configuration from repository
example directories.

## Provider Settings

Do not put real API keys in docs, tests, examples, or tracked files. A local
untracked `IKAROS_HOME/config.yaml` may store third-party API keys directly for
ordinary runs and smoke tests.

`ikaros init` puts the first chat setup fields near the top of the file:
`providers.model.api_key`, `providers.model.base_url`, and
`model.default.model`. Every remote API-backed provider has both an `api_key`
and a `base_url`; model names stay in the feature section that sends the
request.

```yaml
providers:
  model:
    api_key: ""
    base_url: ""
  embedding:
    api_key: ""
    base_url: ""
  tts:
    api_key: ""
    base_url: ""
  asr:
    api_key: ""
    base_url: ""

model:
  default:
    model: ""
    provider: openai-compatible
```

`providers.*` is a schema-only credentials and endpoint section. It is not
merged into `model.default`, `rag`, or `voice`; runtime code passes the matching
provider settings to the model, embedding, TTS, and ASR factories alongside the
feature config that selects provider family, transport, model, timeout, and
budgets.

Provider settings are read only from this section. Plaintext keys should only
live in the local runtime home and must not be committed to the repository.
Generated configs use these plaintext local fields directly.

## Validation

Validate the local runtime config after editing it:

```bash
ikaros config validate
```

Normal runtime config loading already checks YAML shape and rejects unknown
fields before returning an `IkarosConfig`. The explicit validator runs the same
shape checks plus semantic checks for provider/runtime/transport/backend
combinations, missing keys, URLs, model names, and descriptor-only external
memory providers before a remote call is attempted. Validation output uses field
paths such as `providers.model.api_key`; it reports whether a value is missing
or invalid but never prints secret values.

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
    plan:
      mode: plan
      workspace_writes: deny
      shell: ask
      network: ask
      memory_context: true
      rag_context: false
```

Keep `rag_context` false for ordinary chat. Enable it on a profile, or pass
`--rag-top-k`, when the turn needs cited local reference snippets.

Use a profile with:

```bash
ikaros --agent plan chat --message "review only"
ikaros agent run --profile build --dry-run "inspect this repo"
```

Profiles cannot bypass hard denials for destructive commands, direct secret access, protected paths, publishing actions, workspace-external writes, or self-modification.

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
      session_policy:
        history_scope: workspace
        allow_session_switch: true
        max_parallel_subagents: 4
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
- `session_policy.history_scope`: `agent`, `session`, or `workspace`.
- `session_policy.allow_session_switch`: whether runtime may switch sessions for
  this identity.
- `session_policy.max_parallel_subagents`: upper bound for concurrent delegated
  work.
- `auth_scope.local_only`: whether the identity is local-only by default.
- `auth_scope.allow_network`: network default for this identity.
- `route_bindings`: channel/account/peer/thread bindings used by gateway routing.

Resolution rules:

1. A requested name first matches `agent.instances.<name>`.
2. If no instance exists, the same name is resolved as `agent.profiles.<name>`.
3. Without a requested name, `agent.default` is used.

Approval and audit records should use the resolved `agent_id` from the instance,
not just the profile name.

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

chat_history:
  backend: sqlite

rag:
  backend: sqlite
  embedding_provider: hash
```

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
- `IKAROS_HOME/chat/`
- `IKAROS_HOME/rag/`
- `IKAROS_HOME/audit/`
- `IKAROS_HOME/automation/`
- `IKAROS_HOME/gateway/`
- `IKAROS_HOME/skills/`

## Model Provider

The generated config uses the protocol-level `openai-compatible` provider with
empty key, URL, and model fields. A model call fails before the network request
until all required fields are configured.

Supported model provider names are `mock`, `openai-compatible`, `anthropic`, and `ollama`.

OpenAI-compatible example:

```yaml
providers:
  model:
    api_key: "replace-with-provider-key"
    base_url: "https://api.example.com/v1"

model:
  default:
    model: provider-model-id
    provider: openai-compatible
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

Provider names are adapter families, not vendor names. Use `openai-compatible`
for any Chat Completions-compatible service and put the selected endpoint and
model in `providers.model.base_url` and `model.default.model`.

`compat_profile` controls provider/model request quirks inside the
OpenAI-compatible adapter. `auto` first matches `providers.model.base_url`, then
model-name hints, then falls back to `generic`. Supported explicit values are:

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

Provider-specific `compat_profile` values are valid only with
`provider: openai-compatible`. Native `anthropic`, `ollama`, and `mock`
providers accept only `auto` or `generic`.

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
providers:
  model:
    api_key: "replace-with-anthropic-key"
    base_url: "https://api.anthropic.com/v1"

model:
  default:
    model: claude-sonnet-4-5
    provider: anthropic
```

The Anthropic adapter always sends a positive `max_tokens` value. When
`model.default.reasoning` enables thinking, modern Claude models use adaptive
thinking plus `output_config.effort`; legacy Claude models use budget-based
thinking. Claude 4.7 and newer omit sampling fields such as `temperature` and
`top_p`, even if a workflow supplies them.

Ollama local example:

```yaml
providers:
  model:
    api_key: ""
    # Optional. Empty uses http://127.0.0.1:11434.
    base_url: ""

model:
  default:
    model: llama3.2
    provider: ollama
```

The Ollama adapter maps `params.max_tokens` to native `options.num_predict`.
It also forwards explicitly configured `temperature`, `top_p`, `seed`, and
`stop` values through the native `/api/chat` `options` object.

Usage records are written under local audit state and do not include prompt text.

## RAG

The generated config uses the same provider shape for remote embeddings:

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

For fully local indexing without provider credentials, select a local embedding
adapter explicitly:

```yaml
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small
```

Cloud embeddings use the OpenAI-compatible shape and require approval through
the harness before provider calls. Supported embedding provider names are
`hash`, `sparse`, `mock`, and `openai-compatible`.

External memory providers are descriptor metadata only in the current runtime.
`ikaros config validate` rejects enabled external memory providers because
remote append/search adapters are not implemented.

## Voice

The generated config uses remote OpenAI-compatible TTS and ASR slots with empty
credentials and model names:

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

Offline tests can explicitly choose `mock`. The only cloud voice provider name
is `openai-compatible`; the configured remote service must actually expose the
requested TTS or ASR endpoint. TTS text is redacted before provider calls; output
files are treated as workspace writes.

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
