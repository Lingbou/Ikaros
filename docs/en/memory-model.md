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

Records include scope, content, timestamps, tags, source, confidence, and sensitivity flags. Secret-like content is rejected before append or update.

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

## Harness Boundary

Memory writes and deletes go through harness skills. That means policy, audit logging, redaction, and secret rejection apply to both explicit memory commands and runtime-created task or relationship records.
