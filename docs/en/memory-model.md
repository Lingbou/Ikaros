# Memory Model

Ikaros memory is local by default. The runtime separates durable memory from
episode history and reference retrieval:

- Core memory is a small set of accepted typed records.
- Memory projections are deterministic Markdown views of core memory.
- Working memory is a session-scoped scratchpad for short-lived turn facts.
- Candidate memory is an inbox for possible promotion into core memory.
- RAG remains a cited reference system, not a memory promotion path.

## Record Kinds

Supported memory kinds:

- `User`
- `Project`
- `Task`
- `Persona`
- `Relationship`
- `Knowledge`

Records include scope, content, timestamps, tags, source, structured
`source_ref`, optional observer/subject perspective metadata, confidence,
active/supersession state, and sensitivity flags. Secret-like content is
rejected before append or update. Ordinary turn summaries should not be stored
as long-term `Task` memory.

`source_ref` can point at a session turn, session entry, skill call, or manual
note. Runtime memory lifecycle writes use it to link derived records back to
the turn that produced them without making the session store the memory
database.

## Perspective Metadata

Core memory can optionally carry a perspective:

```bash
ikaros memory add --kind relationship --scope default \
  --observer alice --subject bob "Bob likes pancakes"
ikaros memory search --observer alice --subject bob "Bob"
```

`observer` is the agent/user whose view is being recorded. `subject` is the
entity the memory is about. Queries and projections can use this pair to keep
directional facts separate. This borrows the useful boundary from Honcho's
observer/observed representation model without making Ikaros depend on a
background external reasoning service.

## Lifecycle And Policy

`MemoryProvider` implementations must handle lifecycle hooks explicitly:
`turn_start`, `prefetch`, `sync_turn`, `pre_compress`, `session_switch`, and
`delegation_observation`. Callers that intentionally want no side effects should
use `NoopMemoryProvider`.

`MemoryScore`, `MemoryPolicy`, and `MemoryJournal` define the local boundary for
promotion, demotion, forgetting, skipped writes, working-memory appends, and
quota decisions. `JsonlMemoryJournal` writes those decisions to
`memory_journal.jsonl`. Runtime chat records `sync_turn` append and
skipped-write decisions automatically. A normal `sync_turn` writes a redacted
working-memory record under `memory/working/`; it does not append a long-term
`Task` record. Automatic relationship learning creates a pending memory
candidate; it does not promote inferred preferences directly into core memory.
Promote and demote actions tag existing records as `policy-promoted` or
`policy-demoted`; forget actions delete records below the forget threshold or
records evicted by `max_records_per_scope`. The journal is a lifecycle/audit
primitive rather than a replacement for the memory store. The policy pass is
turn-scoped; it is not a global memory compactor.

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

Working memory path:

```text
IKAROS_HOME/memory/working/working_memory.jsonl
```

Working memory is session-scoped and TTL-aware. Queries exclude expired records
by default. Use maintenance commands to inspect or prune the scratchpad:

```bash
ikaros memory working list --session <session-id>
ikaros memory working list --session <session-id> --include-expired
ikaros memory working prune
```

`prune` removes expired working records from the scratchpad file and appends a
`working_memory_expired` journal action for each removed record.

Candidate inbox path:

```text
IKAROS_HOME/memory/candidates.jsonl
```

Projection paths:

```text
IKAROS_HOME/memory/projections/USER.md
IKAROS_HOME/memory/projections/PROJECT.<scope>.md
IKAROS_HOME/memory/projections/MEMORY.md
```

## Projection And Candidates

The model-facing long-term memory surface is a projection, not arbitrary search
results. Render it with:

```bash
ikaros memory projection render --scope ikaros
ikaros memory projection show --scope ikaros
```

The projection renderer reads typed core records and omits task summaries,
memory-lifecycle records, inactive superseded records, sensitive records, and
demoted records. Projection files are derived artifacts and can be regenerated
from the memory store.

Automatic or inferred facts should enter the candidate inbox before they become
core memory:

```bash
ikaros memory candidate list
ikaros memory candidate accept <candidate-id> --reason "explicit user instruction"
ikaros memory candidate accept <candidate-id> \
  --supersedes <memory-id> \
  --reason "user corrected this"
ikaros memory candidate reject <candidate-id> --reason "temporary task scope"
```

Creating a candidate appends a `candidate_created` action to
`memory_journal.jsonl` with the candidate id, kind, scope, and source reference
when present. Accepting a candidate appends a core memory record and refreshes
the projection for that scope. Rejected candidates remain in the inbox for
auditability. Projection renders and candidate accept/reject decisions also
append `projection_rendered`, `candidate_accepted`, `candidate_rejected`, or
`superseded` actions to the journal, with the candidate id, replaced memory id,
or projection scope attached.

## Supersession

Long-term memory updates use supersession rather than deleting history. A
replacement record can mark an older record inactive through `supersedes` and
`superseded_by`, with `valid_from` and `valid_until` timestamps. Ordinary
`memory list` and `memory search` calls hide inactive records by default; pass
`--include-inactive` when inspecting supersession history. Use
`ikaros memory supersession <memory-id>` to explain either side of the chain:
the selected record, the records it replaces, the record that replaced it, and
the validity timestamps. Projections render only active records. This lets
Ikaros answer why a current memory replaced an older fact.

## Relationship Memory

Relationship memory is stored as ordinary `Relationship` records but has a dedicated CLI:

```bash
ikaros relationship remember "Prefer concise updates" --scope user
ikaros relationship show --scope user
ikaros relationship forget --id <id>
```

Chat can extract clear preference, preferred-name, and "remember this"
statements after redaction and de-duplication. Short-lived instructions such as
"I want you to do this for this turn" are not accepted as relationship memory.
Accepted automatic observations enter the candidate inbox first. They become
core relationship records only after `ikaros memory candidate accept`. Use
`--no-relationship-learning` to disable candidate creation for a turn.

The relationship CLI is a façade over `MemoryKind::Relationship`; it is not a
second memory database.

## Harness Boundary

Memory writes and deletes go through governed skills where the operation is
model/tool driven. CLI projection and candidate maintenance still use the same
local store validation and secret rejection. Runtime-created ordinary turn
state goes to working memory; accepted candidates, explicit relationship
commands, and explicit memory commands remain core memory.
