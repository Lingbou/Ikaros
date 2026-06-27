# Body Model

The body layer is a presentation boundary. It renders runtime state but does not execute tools.

## Contracts

`ikaros-body` defines:

- `BodyStatus`: current persona, emotion, task state, context counts, policy decisions, and paths.
- `BodyEvent`: redacted event items with typed JSON `data`, including structured
  audit payloads when the runtime maps audit evidence into a body frame.
- `BodyFrame`: one status snapshot plus recent events.
- render adapters for CLI and web dashboard output.

`ikaros-runtime` assembles body frames from persona, task, chat, and audit state. UI code should
consume those frames instead of reimplementing runtime logic.

## Commands

```bash
ikaros body status
ikaros body dashboard
ikaros body dashboard --refresh-seconds 5 --snapshot-output previews/frame.json
ikaros body serve --port 8001
```

The dashboard writes local HTML under `IKAROS_HOME` by default. The server binds to `127.0.0.1` by
default and serves read-only frame data. Renderers may flatten typed event data for display, but the
`BodyFrame` JSON keeps numbers, booleans, arrays, and objects as JSON values after redaction.

## Rule

Body surfaces may show approvals, audit paths, task status, persona, and emotion. They must not
approve requests, execute skills, write memory, call models, or bypass harness policy.
