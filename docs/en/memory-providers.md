# Memory Providers

The memory provider boundary keeps local memory as the default and gives local stores, registries, and provider lifecycle hooks a single interface. In the MVP, executable append/search/update/delete operations still use local JSONL or SQLite stores.

The provider API is a lifecycle boundary, not just a database abstraction. Runtime
turns can notify memory at specific points without knowing whether the active
implementation is JSONL, SQLite, or an external provider declared in config.

## Current State

Implemented providers:

- local JSONL
- local SQLite

`LocalMemoryStore` implements `MemoryProvider`. External providers can be declared in config and inspected by doctor/registry commands, but remote append/search adapters are not enabled.

Registry states:

- `active`: usable provider.
- `disabled`: configured but not selected.
- `blocked`: configuration is invalid for use, for example when more than one
  external provider is enabled.

The built-in local provider is always active. External provider descriptors are
metadata until a remote adapter exists; declaring one does not redirect local
writes.

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

The local provider currently returns noop reports for most lifecycle hooks, while `prefetch` uses local search. Runtime chat turns call memory lifecycle hooks at turn start and turn end.

Lifecycle context:

- `turn_start`: receives session id, agent id, and user input before context
  assembly.
- `prefetch`: receives a `MemoryQuery` plus optional session and agent ids; local
  providers map this to search.
- `sync_turn`: receives session id, agent id, user input, and assistant output
  after a turn.
- `pre_compress`: receives session id, agent id, and the target context budget.
- `session_switch`: receives old/new session ids and agent id.
- `delegation_observation`: records parent/child agent ids and a summary of the
  delegated work.

Every lifecycle hook returns a `MemoryLifecycleReport` with phase,
records-read, records-written, and notes. A noop report is a valid result and
means the provider had no work for that phase.

## Runtime Context

Chat context assembly uses memory through harness safe-read skills. The skill
executes with the real local query but records a redacted audit input, so the
audit log does not store full prompts. Relationship memory and normal memory
search are both context sources; neither one bypasses policy.

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

Only one external provider may be enabled. If multiple are enabled, the registry reports a blocked state instead of splitting writes across systems.

## Boundaries

- Local memory and RAG remain the default path.
- External provider config does not automatically replace local stores.
- Secret-like memory content is rejected or redacted.
- Relationship, task, project, and knowledge memory should not silently diverge across multiple providers.

## Failure Handling

Unsupported local backends fail during store/provider construction. Multiple
enabled external providers put the registry into a blocked state. Secret-like
records are rejected before storage. Provider lifecycle failures should be
reported to the caller instead of silently dropping writes.
