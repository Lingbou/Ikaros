# Ikaros Runtime

The Ikaros Runtime is a long-lived local Python process supervised by the
Electron main process. The architecture and first vertical-slice boundary are
recorded in [DESIGN.md](DESIGN.md).

## Development

Install the locked environment from the repository root:

```powershell
uv sync --project runtime --locked
```

Run the Runtime checks:

```powershell
uv run --project runtime ruff check runtime
uv run --project runtime mypy runtime/src runtime/tests
uv run --project runtime pytest runtime/tests
```

The Runtime server is normally started by Ikaros Desktop. Its standard output
is reserved for one machine-readable readiness record; diagnostics go to
standard error.

Runtime-owned state defaults to `~/.ikaros`. Tests pass an isolated
`IKAROS_HOME`; normal Desktop launches do not override it. The Runtime creates
`state.db` on startup, while `config.yaml` remains absent until the user saves a
real provider/model configuration.

The deterministic `scripted/scripted-v1` provider supports ordinary streamed
conversation and one explicit Tool-loop smoke syntax:

```text
/process.run <command>
```

This invokes the provider-safe `process_run` Tool under V1's fixed
`full_access` policy, persists its bounded result, and asks the scripted
provider for a final answer. It exists for deterministic Runtime/Desktop tests;
real providers request the same Tool through their normal tool-calling
protocol.
