# Ikaros clients

This directory contains user-facing Ikaros clients. Clients present and control
the Agent runtime; they are not the runtime itself and must not become the
authority for canonical conversations, tool execution, permissions, or Agent
state.

- `desktop/` is the Windows and Linux client. It now integrates the first real
  Runtime vertical slice, including persistent Threads, streaming provider
  turns, process execution, and workspace-backed project conversations.
- `cli/` is not implemented yet. A future CLI will reuse the same Runtime
  contract rather than introduce a separate execution path.

The desktop renderer is kept host-independent so it can be reused if the
desktop shell changes later.
