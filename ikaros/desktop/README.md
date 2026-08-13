# Ikaros Desktop

Cross-platform desktop client for the Ikaros general-purpose Agent on Windows
and Linux. Electron main supervises one long-lived local Python Runtime and
connects to it over authenticated, loopback WebSocket JSON-RPC. The production
Desktop conversation path is Runtime-backed; deterministic mocks remain only
for tests, non-Electron renderer harnesses, and UI capabilities that have not
yet received Runtime protocol support.

## Run locally

Prerequisites: Node.js 22.12 or newer, pnpm 11.9, and uv.

```shell
uv sync --project ../../runtime --locked
pnpm install
pnpm dev
```

Useful verification commands:

```shell
pnpm check
pnpm package:dir
pnpm package:linux
pnpm package:win
```

`package:linux` creates unsigned x64 AppImage, DEB, and RPM artifacts in
`dist/`. The Linux packages install the `io.github.lingbou.ikaros.desktop`
launcher, the Ikaros application icon, and the `ikaros` executable.

Fedora build hosts need the FPM compatibility library and RPM build tools:

```shell
sudo dnf install libxcrypt-compat rpm-build
```

Debian and Ubuntu build hosts need the RPM tooling only when producing all
three formats:

```shell
sudo apt install rpm
```

Run the portable AppImage directly, or install the native package for the host
distribution:

```shell
./dist/Ikaros-0.1.0-linux-x86_64.AppImage
sudo apt install ./dist/Ikaros-0.1.0-linux-amd64.deb
sudo dnf install ./dist/Ikaros-0.1.0-linux-x86_64.rpm
```

`package:win` creates an unsigned Windows x64 NSIS development installer. It
is a prototype artifact, not a signed production release.

## Current feature boundary

### Runtime-backed

The production path is:

```text
React renderer
  -> typed preload API
  -> trusted Electron IPC
  -> Electron RuntimeHost
  -> authenticated loopback WebSocket JSON-RPC
  -> Python Runtime
```

The Runtime currently owns:

- `thread.list` and `thread.create`, including optional workspace snapshots;
- `turn.start`, streamed Item events, multi-Turn context, and settled Run state;
- `run.cancel`, event replay, sequence-based reconnect catch-up, and SQLite
  recovery across Runtime restarts;
- DeepSeek and Custom OpenAI-compatible Provider configuration, real DeepSeek
  model discovery, Model enablement, Provider disconnect/removal, and
  `~/.ikaros/config.yaml` persistence;
- the ScriptedProvider used by deterministic integration tests and real
  OpenAI-compatible streaming Providers used by normal conversations; and
- the provider-facing `process_run`, `read`, `write`, and `edit` Tools under the
  V1 `full_access` policy. Desktop labels `process_run` as `process.run`;
  command execution provides timeout, cancellation, bounded output, and
  process-tree cleanup, while the file Tools provide streaming bounded UTF-8
  reads, verified atomic writes, and exact-match edits that reject common
  stale-content races.

The renderer projects canonical Runtime messages, streamed deltas,
`process.run`/`read`/`write`/`edit` Tool Calls and Tool Results, and Run state
into the conversation UI. File cards expose bounded metadata such as path, line
range, bytes written, and replacement count without displaying the full write
or replacement arguments. It does not use `MockAgentClient` when the Electron
Runtime bridge is available.

Projects are not separate Runtime resources and there is no Project API. A
Project is a Desktop grouping derived from a Thread's optional workspace.
Project chats and ordinary chats both use `thread.create -> turn.start`; the
only difference is that a project Thread carries a workspace snapshot. That
workspace is persisted by the Runtime and becomes the default working
directory for `process.run` and the base for relative `read`, `write`, and
`edit` paths. A selected folder without a Thread exists only as temporary
Desktop state until the first message creates that Thread.

### Desktop-owned local state

Some real product state belongs to Desktop rather than the Agent Runtime:

- theme, UI language, fonts, reduced-motion preference, sidebar state, and the
  local profile username are persisted by Electron in `ui-preferences.json`;
- the system directory picker creates workspace snapshots, while Projects are
  derived from Runtime Threads in the renderer;
- Search filters the titles of Threads already loaded in the renderer; it is
  not a Runtime or semantic-search API; and
- drafts, selected Model, expanded Project groups, and settings navigation are
  renderer session state.

### Mock, placeholder, or not connected

The following UI surfaces are not production Runtime capabilities yet:

- `MockAgentClient` and its five deterministic scenarios remain for tests and
  non-Electron renderer development only;
- `/mock-1`, `/mock-2`, and `/mock-3` only insert placeholder text; Skills do
  not yet have a loader, registry, RPC surface, or execution lifecycle;
- Profile metrics, activity history, insights, and most-used Skills are static
  demonstration data;
- permission cards and Ask/Safe/Full choices belong to the mock prototype;
  Runtime V1 always uses `full_access` and emits no permission requests;
- editing a message to fork history, multiple Branches, retry/resume recovery,
  Artifacts, first-class file-change/diff events, and generic status rows are
  mock-only UI projections without corresponding Runtime RPCs or events. This
  does not include the Runtime-backed `read`, `write`, and `edit` Tool cards;
- attachments, tool selection, and response regeneration are disabled
  placeholders;
- Provider health is currently reported as `unknown`; no health-check workflow
  is connected; and
- automatic model discovery currently supports only DeepSeek. Custom
  OpenAI-compatible Provider models are entered manually.

## Architecture boundary

The React renderer never owns the Runtime process or canonical Agent state and
never talks to raw WebSocket or Electron IPC. Electron main owns Runtime
supervision, authentication, reconnect, and the narrow IPC bridge. Runtime
wire DTOs remain separate from renderer projection types so future capabilities
can extend the protocol without turning mock-specific cards into canonical
state.
