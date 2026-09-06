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
Python Runtime bundling is outside the daily-use Alpha milestone. Development
and validation use the checkout Runtime with Python 3.12/3.13 installed.

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

- paginated active/archived `thread.list`, `thread.get`, paginated `turn.list`,
  and `thread.create`, including optional workspace snapshots and the canonical
  nullable `archivedAt` field;
- `thread.rename`, `thread.archive`, and `thread.unarchive`, with sequenced
  lifecycle events persisted and projected through SQLite;
- `turn.start`, streamed Item events, bounded multi-Turn model context, and
  settled Run state. Runtime accounts for the model's context window and output
  reserve while Desktop independently pages complete UI history;
- `run.cancel`, event replay, sequence-based reconnect catch-up, and SQLite
  recovery across Runtime restarts;
- ordinary new Turns after failure, preserving Tool results and explicit
  incomplete/unknown-outcome context without replaying old commands;
- `file.preview` and `file.change.get`, with a read-only right-hand panel opened
  from file tool cards or a workspace path. Current content supports refresh,
  revision-bound pagination and line numbers; immutable operation diffs show
  before/after encoding metadata. Quoting a file adds its path to the draft for
  the next conversational edit. Old missing captures are never reconstructed;
- DeepSeek and Custom OpenAI-compatible Provider configuration, real DeepSeek
  model discovery, Model enablement, Provider disconnect/removal, and
  `~/.ikaros/config.yaml` persistence;
- the ScriptedProvider used by deterministic integration tests and real
  OpenAI-compatible streaming Providers used by normal conversations; and
- `usage.read`, backed only by Provider-reported usage from
  `model.response_finished`, which supplies Token totals, longest completed task
  duration, streaks, and daily buckets for the Profile page;
- Runtime-backed Skills V0: safe catalog discovery and diagnostics,
  `skill.list` / `skill.set_enabled`, global enablement persisted in
  `~/.ikaros/config.yaml`, immutable enabled-descriptor snapshots per Run, and
  a lazily loaded Skills settings page;
- Memory management and recall: strict DTO parsing and narrow IPC/preload
  methods for `memory.create`, `memory.correct`, `memory.forget`, paginated
  `memory.list`, and lazy `memory.get`. Create can carry one Runtime-verified
  Session Item source; mutation conflicts cross the bridge as a bounded
  `reasonCode`, not arbitrary JSON-RPC data. The data lives in Runtime-owned
  `~/.ikaros/memory.db`. A lazy Settings page provides active/forgotten,
  kind, Global/exact-Workspace filters, cursor pagination, per-record provenance
  verification, Create/Correct/Forget, and conflict-safe refresh. Runtime recall
  freezes bounded exact revisions in ContextRevision/StepInput audit
  payloads; Electron main validates their scope, token limits,
  omissions, and cross-Step consistency before advancing the Journal cursor;
- current execution audit DTOs: immutable `RunConfig` on Turn creation,
  `ContextRevision` and per-call `StepInput` through `model.input_prepared`, and
  response metadata/usage through `model.response_finished`. Only the current
  protocol and persistence formats are parsed;
- the Runtime-owned `IKAROS.md` identity frozen in each RunConfig;
- model capacity settings (initially 32,768 context tokens / 4,096 output tokens)
  and per-Run budgets (default 100 model calls / 60 minutes);
- `process_start`, `process_read`, `process_wait`, `process_stop`, `read`, `write`,
  and `edit` under `full_access`. Command start returns a process ID; read/wait
  establish current state and observed exit. Wait timeouts leave commands alive.
  Run completion, cancellation, or deadline cleans owned process trees.

The renderer projects canonical Runtime messages, streamed deltas,
managed-command and `read`/`write`/`edit` Tool Calls and Tool Results, and Run state
into the conversation UI. File cards expose bounded metadata such as path, line
range, bytes written, and replacement count without displaying the full write
or replacement arguments. It does not use `MockAgentClient` when the Electron
Runtime bridge is available.

When no runnable Model exists, the composer opens Provider or Model settings
directly and preserves the draft, selected Thread, and staged Workspace. Runtime
connection and catalog loading failures have separate reconnect/reload actions.
Failed and cancelled Runs retain successful Tool results and display a localized
explanation plus a bounded `reasonCode`, including after history reload. An
unsuccessful file or command operation is identified as potentially having changed
files; starting a new Turn neither resumes that Run nor rolls its operations back.

Electron main validates current execution DTO relationships before advancing the
Event cursor. `model.input_prepared` and `model.response_finished` update the Run's
budget/progress display without becoming assistant conversation messages.

Conversation-history cold startup reads only the active Thread catalog and its
sequence waterline; it does not replay the full Journal from sequence zero.
Selecting a Thread then loads and caches that Thread's materialized
Turn/Run/Item history through `thread.get` and `turn.list`. Independent
metadata/page waterlines and a loading-time event buffer merge concurrent live
deltas exactly once, while per-Thread, per-Run activity keeps background
execution and Stop independent from navigation.

The conversation title menu renames or archives the current Thread through the
Runtime. Archived Threads leave the active sidebar catalog and are loaded from
the separate archived catalog only when General settings opens its management
dialog; that dialog can unarchive them. Lifecycle events reconcile concurrent
catalog reads, so a Thread moved between active and archived scopes is not
reintroduced by an older page response. A queued or running Run prevents
archive, and an archived Thread cannot start another Turn.

Projects are not separate Runtime resources and there is no Project API. A
Project is a Desktop grouping derived from a Thread's optional workspace.
Project chats and ordinary chats both use `thread.create -> turn.start`; the
only difference is that a project Thread carries a workspace snapshot. That
workspace is persisted by the Runtime and becomes the default working
directory for `process_start` and the base for relative `read`, `write`, and
`edit` paths. A selected folder without a Thread exists only as temporary
Desktop state until the first message creates that Thread.

### Desktop-owned local state

Some real product state belongs to Desktop rather than the Agent Runtime:

- theme, UI language, fonts, reduced-motion preference, sidebar state, and the
  local profile username are persisted by Electron in `ui-preferences.json`;
- the system directory picker creates workspace snapshots, while Projects are
  derived from Runtime Threads in the renderer;
- Search first loads the active Thread catalog and then filters its titles in
  the renderer; it does not search message bodies or provide semantic search; and
- drafts, selected Model, expanded Project groups, and settings navigation are
  renderer session state.

### Mock, placeholder, or not connected

The following UI surfaces are not production Runtime capabilities yet:

- `MockAgentClient` and its five deterministic scenarios remain for tests and
  non-Electron renderer development only;
- `/mock-1`, `/mock-2`, and `/mock-3` only insert placeholder text. Runtime
  Skills V0 is connected, but task-specific selection, automatic full-body
  loading, dedicated Skill execution Items/attribution, and Skill statistics
  are not implemented;
- permission cards and Ask/Safe/Full choices belong to the mock prototype;
  Runtime V1 always uses `full_access` and emits no permission requests;
- editing a message to fork history, multiple Branches, retry/resume of an old Run,
  Artifacts, and generic status rows are
  mock-only UI projections without corresponding Runtime RPCs or events. This
  does not include the Runtime-backed `read`, `write`, and `edit` Tool cards;
- attachments, tool selection, and response regeneration are disabled
  placeholders;
- Provider health is currently reported as `unknown`; no health-check workflow
  is connected; and
- automatic model discovery currently supports only DeepSeek. Custom
  OpenAI-compatible Provider models are entered manually; and
- The visible Memory management page and deterministic bounded Memory recall are
  Runtime-backed rather than Mock. Recall is limited to Global plus the current
  workspace and freezes exact revisions per Run; no standalone Memory
  maintenance or transfer surface is part of this stage.

## Architecture boundary

The React renderer never owns the Runtime process or canonical Agent state and
never talks to raw WebSocket or Electron IPC. Electron main owns Runtime
supervision, authentication, reconnect, and the narrow IPC bridge. Runtime
wire DTOs remain separate from renderer projection types so future capabilities
can extend the protocol without turning mock-specific cards into canonical
state.

Conversation persistence uses **SQLite schema 10, Journal schema 7, and protocol
4**. Old selectors, execution DTOs, and migration paths have been removed.
Incompatible development state fails with an explicit reset-required error;
Runtime never automatically deletes it. Use a fresh development Runtime home or
rebuild disposable Session state deliberately with Runtime stopped. Provider/model
configuration, API keys, Skills, Desktop preferences, and `memory.db` are separate.

The 13 Journal event types include `process.recorded`: durable command intent,
state, and bounded output, independent of Tool Call completion. Restart marks
active commands unknown and never reattaches their PID. Preview and historical
diff bodies remain outside model input. Commands can create files available for
preview, but their changes are not automatically diff-tracked.

The first long-task stage provides budgets and managed command execution.
Automatic context compression, runtime user steering, completion checking, and
a full process-log panel remain planned. Larger Run budgets alone do not solve
context overflow. Python Runtime bundling, attachments, web tools, and multi-Agent
execution remain outside this stage. See
[the development plan](../../runtime/LONG_TASK_PLAN.md) for subsequent milestones.
