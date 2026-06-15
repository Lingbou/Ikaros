# Memory Providers

The memory provider boundary keeps local memory as the default and gives local stores, registries, and provider lifecycle hooks a single interface. In the MVP, executable append/search/update/delete operations still use local JSONL or SQLite stores.

The provider API is a lifecycle boundary, not just a database abstraction. Runtime
turns can notify memory at specific points without knowing whether the active
local implementation is JSONL or SQLite.

## Current State

Implemented providers:

- local JSONL
- local SQLite

`LocalMemoryStore` implements `MemoryProvider`. `NoopMemoryProvider` is an
explicit disabled implementation for callers that deliberately want no memory
side effects. External provider records are descriptor metadata only. They can
document a future integration, but they are not executable providers in the
current runtime.

Registry states:

- `active`: usable provider.
- `disabled`: descriptor metadata is present but not executable.
- `blocked`: configuration is invalid for use.

The built-in local provider is always active. External provider descriptors are
metadata until a remote adapter exists; declaring one does not redirect local
writes. `ikaros config validate` rejects enabled external providers.

Inspect provider state with:

```bash
ikaros memory provider list
ikaros memory provider active
ikaros memory provider show local-jsonl
ikaros doctor
```

## Lifecycle

`MemoryProvider` is more than a store/search interface. It defines turn lifecycle hooks:

- `turn_start`
- `prefetch`
- `sync_turn`
- `pre_compress`
- `session_switch`
- `delegation_observation`

The trait does not provide hidden default noop methods. Each provider must
implement every hook, or the caller must select `NoopMemoryProvider`
explicitly. Runtime chat turns call memory lifecycle hooks at turn start and
turn end.

Lifecycle context:

- `turn_start`: receives session id, agent id, and user input before context
  assembly.
- `prefetch`: receives a `MemoryQuery` plus optional session and agent ids; local
  providers map this to search.
- `sync_turn`: receives session id, agent id, user input, and assistant output
  after a turn. The local provider writes a redacted `Task` turn-summary record
  with `MemoryRef::SessionTurn` when the turn is safe to store; if a redacted
  secret marker is present, it reports a skipped write instead of failing the
  chat turn.
- `pre_compress`: receives session id, agent id, and the target context budget.
- `session_switch`: receives old/new session ids and agent id.
- `delegation_observation`: records parent/child agent ids and a summary of the
  delegated work.

Every lifecycle hook returns a `MemoryLifecycleReport` with phase,
records-read, records-written, and notes. A noop report is only produced by an
explicit provider implementation; it is not a trait fallback.

## Runtime Context

Chat context assembly uses memory through harness safe-read skills. The skill
executes with the real local query but records a redacted audit input, so the
audit log does not store full prompts. Relationship memory is
`MemoryKind::Relationship` and normal memory search excludes that kind from the
generic memory section because it is rendered in the relationship section.
Neither path bypasses policy.

`ContextEngine` owns when memory is assembled and compacted. `MemoryProvider`
owns provider-specific lifecycle side effects. Keep these responsibilities
separate: context assembly should not directly implement remote memory sync, and
memory providers should not build model prompts.

## Config

```yaml
memory:
  backend: jsonl
  external_providers:
    - id: team-memory
      provider: plugin
      enabled: false
      endpoint: http://127.0.0.1:8787
      api_key: "replace-with-your-provider-key"
```

Keep `enabled: false`. Enabled external memory providers are rejected by
`ikaros config validate` because remote append/search adapters are not
implemented.

## Boundaries

- Local memory and RAG remain the default path.
- External provider config does not automatically replace local stores.
- External provider descriptors are not a runtime capability until a real
  adapter is implemented.
- Secret-like memory content is rejected or redacted.
- Memory records can carry a structured `MemoryRef` such as a session turn,
  session entry, skill call, or manual note.
- Relationship, task, project, and knowledge memory should not silently diverge across multiple providers.

## Failure Handling

Unsupported local backends fail during store/provider construction. Enabled
external providers are invalid in the current runtime. Secret-like records are
rejected before storage. Provider lifecycle failures should be reported to the
caller instead of silently dropping writes.
