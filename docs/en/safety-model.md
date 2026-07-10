# Safety Model

Ikaros treats tool use as a controlled operation. `ikaros-execution::toolkit` defines the
shared tool contracts, `ikaros-execution::sandbox` provides concrete local execution
backends, and the harness decides whether a model, persona, command, or task
runner may use them.

## Default Policy

The default policy is conservative:

- Safe reads inside the workspace are allowed.
- Workspace writes usually require approval.
- Workspace-external writes are denied.
- Workspace-external reads require approval.
- Secret-looking paths require approval or a dedicated secret adapter.
- Secret-like memory content is rejected.
- Destructive shell commands are denied.
- Git commit, push, tag, release, and package publish actions are denied.
- Direct secret access is denied.
- Ordinary self-modification is denied.

Agent profiles can change ordinary policy choices such as whether workspace writes, shell actions,
or network actions are allowed, denied, or approval-gated. Profiles cannot weaken the hard denials
above.

## Audit And Approval

Governed tool calls record:

- `tool_call`
- `policy_decision`
- `tool_result`

Approval requests are stored under local audit state and are bound to the workspace where they were
created. Approval replay verifies that binding before executing.

Dry-run mode still evaluates policy and writes audit events, but allowed skills return dry-run
results instead of mutating local state.

Audit events are written to `audit.jsonl` by default. The log rotates when it exceeds 16 MiB or when
the event date changes, and old JSONL files are compressed into `.gz` archives so the active audit
file does not grow without bound.

## Provider Safety

Model and RAG embedding providers are adapter-based. Requests are redacted before provider
calls where the current implementation supports it. Usage logs store provider/model/token metadata
and do not store prompts.

Provider keys and base URLs are read from the local `IKAROS_HOME/config.yaml`.
The active model normally uses `model.default`; shared resource pools still use
`providers.*`. Keys must not be stored in repository files, audit logs, memory,
or RAG indexes.

## Local Tasks

Task requests do not grant permission. When a task is processed, it goes through
the same runtime and harness path as an explicit CLI command.

## Plugins

Command-backed plugins execute only through the built-in plugin runner skill. Plugin manifests
declare risk and command metadata, but policy evaluation still happens at runtime. Plugin
stdin/stdout/stderr are redacted in harness output.
