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
