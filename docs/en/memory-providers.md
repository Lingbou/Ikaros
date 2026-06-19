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

The memory policy boundary includes:

- `MemoryScore`: recency, relevance, frequency, source-strength, confidence,
  and sensitivity inputs.
- `MemoryPolicy`: promote, demote, forget, and per-scope quota thresholds.
- `MemoryJournal`: append-only policy/action records.
- `JsonlMemoryJournal`: local `memory_journal.jsonl` implementation.

The journal is an audit and replay aid for memory lifecycle decisions. Runtime
chat writes `sync_turn` append or skipped-write decisions to the journal, then
records promote, demote, forget, and quota-eviction decisions for affected core
memory scopes when any exist. Quota evictions are journaled as `forget` actions
with a quota reason.
Projection renders, candidate accept/reject decisions, working-memory expiry,
and supersession events are also journaled so debug tooling can explain why a
projected memory surface changed, why a scratchpad record disappeared, and
which old memory was replaced.
It does not replace the memory store, and it does not enable external memory
providers. The current policy pass is turn-scoped rather than a full-store
compactor.

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
  after a turn. The local provider writes a redacted session working-memory
  record with `MemoryRef::SessionTurn` when the turn is safe to store; it does
  not promote ordinary turn summaries into long-term `Task` memory. If a
  redacted secret marker is present, it reports a skipped write instead of
  failing the chat turn.
- `pre_compress`: receives session id, agent id, and the target context budget.
- `session_switch`: receives old/new session ids and agent id.
- `delegation_observation`: records parent/child agent ids and a summary of the
  delegated work.

Every lifecycle hook returns a `MemoryLifecycleReport` with phase,
records-read, records-written, and notes. A noop report is only produced by an
explicit provider implementation; it is not a trait fallback.

Runtime chat persists `turn_start` and `sync_turn` reports as
`MemoryLifecycle` session events. `sync_turn` reports include
`MemoryRef::SessionTurn` when they can be tied to a turn. If the local provider
sees a redaction marker in the derived turn summary, it records a skipped write
instead of storing the summary. A successful ordinary `sync_turn` is journaled
as a working-memory append.

After a successful `sync_turn`, runtime applies `MemoryPolicy` only to affected
core memory scopes referenced by the lifecycle report. Ordinary local sync now
writes working memory, so it normally contributes a journaled working-memory
append without promoting, demoting, or forgetting core records. Automatic
relationship observations are written as pending candidates, so they do not
trigger a relationship core memory policy pass until a candidate is accepted.
When a core-memory scope is affected, the same pass checks the kind/scope group
against `max_records_per_scope`. Promote/demote decisions update local tags;
forget decisions delete low-score or quota-evicted records. Every action is
written to `JsonlMemoryJournal` with a score and `MemoryRef::SessionTurn` when
available. Memory store updates/deletes and journal appends are still separate
local operations until cross-store transaction semantics are introduced.

Inspect persisted lifecycle evidence with:

```bash
ikaros debug memory-lifecycle <session-id>
ikaros debug memory-lifecycle <session-id> --turn-id <turn-id>
```

The command reads `state.db` plus `memory_journal.jsonl` and reports lifecycle
phases, records read/written, `MemoryRef::SessionTurn`, skipped-write reasons,
redaction-related notes, action counts, and runtime memory policy actions.

## Runtime Context

Chat context assembly uses memory through harness safe-read skills. Projection
and working-memory reads are separate safe-read tools, and retrieved memory
search remains available for explicit local lookup. The skills execute with the
real local query but record a redacted audit input, so the audit log does not
store full prompts. Relationship memory is `MemoryKind::Relationship` and
normal memory search excludes that kind from the retrieved-memory section
because it is rendered in the relationship section. Ordinary `Task` turn
summaries are also excluded from retrieved memory context. The persisted context
event keeps projection, working memory, and retrieved memory as separate
sections with separate trust/source metadata.
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

- Local memory remains the default path. RAG is local but only injected when a
  profile enables it or the user requests hits with `--rag-top-k`.
- External provider config does not automatically replace local stores.
- External provider descriptors are not a runtime capability until a real
  adapter is implemented.
- Secret-like memory content is rejected or redacted.
- Memory records can carry a structured `MemoryRef` such as a session turn,
  session entry, skill call, or manual note.
- Runtime `sync_turn` working-memory append, skipped-write, promote, demote,
  forget, and quota decisions are recorded in `MemoryJournal`.
- Relationship, task, project, and knowledge memory should not silently diverge across multiple providers.

## Failure Handling

Unsupported local backends fail during store/provider construction. Enabled
external providers are invalid in the current runtime. Secret-like records are
rejected before storage. Provider lifecycle failures should be reported to the
caller instead of silently dropping writes.
