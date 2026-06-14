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

`ikaros init` puts external-resource settings at the top of the file. Every
remote API-backed provider has both an `api_key` and a `base_url`; model names
stay in the feature section that sends the request.

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
```

At load time, `providers.model` feeds `model.default`,
`providers.embedding` feeds `rag`, and `providers.tts` /
`providers.asr` feed `voice.tts` / `voice.asr`.

Provider settings are read only from this section. Plaintext keys should only
live in the local runtime home and must not be committed to the repository.
Generated configs use these plaintext local fields directly.

## Validation

Validate the local runtime config after editing it:

```bash
ikaros config validate
```

The validator reads `IKAROS_HOME/config.yaml`, checks the YAML shape, rejects
unknown fields, checks provider/runtime/transport/backend combinations, and
reports missing keys, URLs, and model names before a remote call is attempted.
Validation output uses field paths such as `providers.model.api_key`; it reports
whether a value is missing or invalid but never prints secret values.

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
      rag_context: true
    plan:
      mode: plan
      workspace_writes: deny
      shell: ask
      network: ask
      memory_context: true
      rag_context: true
```

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

chat_history:
  backend: sqlite

rag:
  backend: sqlite
  embedding_provider: hash
```

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

Supported model provider names are `mock`, `openai-compatible`/`openai`, `moonshot`, `siliconflow`, `anthropic`/`claude`, and `ollama`/`local-llm`.

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
    rate_limit_per_minute: 60
    daily_token_budget: 100000
```

Provider aliases such as `moonshot` and `siliconflow` use the same
OpenAI-compatible adapter; they are convenience names, not default vendors.

Anthropic example:

```yaml
providers:
  model:
    api_key: "replace-with-anthropic-key"
    base_url: "https://api.anthropic.com/v1"

model:
  default:
    provider: anthropic
    model: claude-sonnet-4-5
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
```

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
`hash`, `sparse`, `mock`, `openai-compatible`/`openai`, `moonshot`, and
`siliconflow`.

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

Offline tests can explicitly choose `mock`. Accepted cloud voice provider names
are aliases for the OpenAI-compatible adapter: `openai-compatible`, `openai`,
`moonshot`, and `siliconflow`. A vendor alias does not guarantee that the remote
service exposes both TTS and ASR endpoints. TTS text is redacted before provider
calls; output files are treated as workspace writes.

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
