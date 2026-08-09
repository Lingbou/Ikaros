# Ikaros clients

This directory contains user-facing Ikaros clients. Clients present and control
the Agent runtime; they are not the runtime itself and must not become the
authority for canonical conversations, tool execution, permissions, or Agent
state.

- `desktop/` is the Windows and Linux interaction prototype.
- A `cli/` client will be added only after a real runtime vertical slice exposes
  a stable shared contract.

The desktop renderer is kept host-independent so it can be reused if the
desktop shell changes later.
