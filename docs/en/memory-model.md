# Memory Model

Ikaros memory is local by default and is stored as typed records.

## Record Kinds

Supported memory kinds:

- `User`
- `Project`
- `Task`
- `Persona`
- `Relationship`
- `Knowledge`

Records include scope, content, timestamps, tags, source, structured
`source_ref`, confidence, and sensitivity flags. Secret-like content is
rejected before append or update.

`source_ref` can point at a session turn, session entry, skill call, or manual
note. Runtime memory lifecycle writes use it to link derived records back to
the turn that produced them without making the session store the memory
database.

## Lifecycle And Policy

`MemoryProvider` implementations must handle lifecycle hooks explicitly:
`turn_start`, `prefetch`, `sync_turn`, `pre_compress`, `session_switch`, and
`delegation_observation`. Callers that intentionally want no side effects should
use `NoopMemoryProvider`.

`MemoryScore`, `MemoryPolicy`, and `MemoryJournal` define the local boundary for
promotion, demotion, forgetting, skipped writes, and quota decisions.
`JsonlMemoryJournal` writes those decisions to `memory_journal.jsonl`. Runtime
does not yet write every policy action automatically, so the journal is a
lifecycle/audit primitive rather than a replacement for the memory store.

## Backends

Default JSONL path:

```text
IKAROS_HOME/memory/memory.jsonl
```

SQLite path:

```text
IKAROS_HOME/memory/memory.sqlite
```

Select the backend in config:

```yaml
memory:
  backend: sqlite
```

## Relationship Memory

Relationship memory is stored as ordinary `Relationship` records but has a dedicated CLI:

```bash
ikaros relationship remember "Prefer concise updates" --scope user
ikaros relationship show --scope user
ikaros relationship forget --id <id>
```

Chat can learn clear preference, preferred-name, and "remember this" statements after redaction and de-duplication. Use `--no-relationship-learning` to disable that write for a turn.

The relationship CLI is a façade over `MemoryKind::Relationship`; it is not a
second memory database.

## Harness Boundary

Memory writes and deletes go through harness skills. That means policy, audit logging, redaction, and secret rejection apply to both explicit memory commands and runtime-created task or relationship records.
