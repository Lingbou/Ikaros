# Ikaros Desktop Renderer

## Locked product decisions

- Runtime identity is `Thread -> Branch -> Turn -> Run -> Item`; V1 exposes
  only each Thread's default Branch and has no Branch mutation RPC. Renderer
  records are presentation projections and are not the wire protocol.
- A Runtime Thread may carry a workspace snapshot. Projects are Desktop groupings derived from those workspace snapshots, not independent Runtime resources.
- The sidebar shows workspace-owned Threads under Projects and only standalone Threads under Recents, without duplicate projections.
- Project chats and standalone chats use the same `thread.create -> turn.start` path. The workspace and the resulting default Tool working directory are the only semantic difference.
- Agent profiles, providers, models, tools, and artifacts are orthogonal resources rather than navigation parents.
- The renderer should look and feel like a restrained dark native desktop chat client while using Ikaros general-agent language.
- The composer keeps the selected Runtime Model and fixed V1 `full_access` state visible. Unsupported attachments, tool selection, and regeneration are not advertised in production UI.
- Runtime-backed Tool calls render as typed events instead of raw protocol JSON. Future permissions, plans, file changes, and Artifacts must follow the same projection rule when their protocols exist.
- If message editing is connected later, it must create a new Branch and preserve the original history. The current Runtime has no Branch RPC, so editing and Branch creation are disabled in Runtime mode.
- A deterministic mock client remains available for renderer tests and non-Electron development, but it does not drive the production Electron conversation path.
- Mock-only cards for permissions, recovery, Artifacts, file changes, generic status, and Branch creation remain reusable presentation work; their presence does not imply Runtime support.
- The first renderer is local-first and responsive at both 1280x800 and 1920x1080.

## Runtime-backed surface

Electron main already supervises and authenticates the long-lived Python
Runtime. The renderer reaches it only through the typed preload API and trusted
IPC; Electron main translates those calls to authenticated loopback WebSocket
JSON-RPC.

The connected surface is:

- paginated active/archived Thread catalogs, Thread detail/history reads,
  create, rename, archive, and unarchive, with optional workspace;
- Turn start and Run cancel;
- sequenced event notifications, paginated replay, reconnect catch-up, and
  Runtime-restart recovery;
- streamed user/assistant Items, `process.run`, `read`, `write`, and `edit`
  Tool Call/Result Items, and Run state projection;
- Runtime-owned SQLite conversation state;
- DeepSeek and Custom OpenAI-compatible Provider configuration and execution;
- DeepSeek model discovery plus Provider/Model list, enable, disconnect, and
  remove operations;
- `usage.read` backed Profile Token metrics and activity;
- `skill.list` / `skill.set_enabled`, Skills catalog diagnostics, global
  enablement, a real Skills settings page, and immutable enabled-descriptor
  snapshots per Run;
- typed `memory.create/correct/forget/list/get` Runtime client methods below the
  Renderer state layer, including bounded mutation reason codes and Session
  provenance. They are intentionally not consumed by a page or Zustand
  projection in Gate 6; and
- fixed V1 `full_access` execution of the four built-in Tools.

## Desktop-owned local state

The following state is real but intentionally does not belong to the Runtime:

- Electron persists theme, language, font, motion, sidebar preferences, and
  the local profile username in `ui-preferences.json`;
- Electron owns the system directory picker;
- the renderer derives Project groups from Runtime Thread workspaces and keeps
  an uncommitted folder selection locally until the first Thread is created;
- Search filters currently loaded Thread titles only; and
- drafts, selected Model, expanded groups, and settings navigation are
  renderer session state.

## Mock and placeholder surface

`MockAgentClient` remains a deterministic fallback for tests and renderer-only
harnesses. In the production Electron path, the presence of the preload Runtime
bridge selects `RuntimeClient`, starts with Runtime-owned Threads, and routes
messages and Stop to the Python Runtime.

Still mock-only or unavailable:

- the three `/mock-*` slash commands and deterministic scenario cards;
- task-specific Skill selection, automatic full-body loading, dedicated Skill
  execution Items/attribution, and Skill statistics. Skills catalog discovery,
  diagnostics, global enablement, and descriptor injection are Runtime-backed;
- permission decisions and the Ask/Safe access modes;
- message-edit Branch creation and multiple Runtime Branches;
- retry/resume recovery controls;
- Artifact, file-change, generic status, and Branch-created events; and
- attachments, tool selection, and response regeneration.

Bounded history selection, model-input manifests, and the Runtime-owned
`IKAROS.md` Identity Core are connected. Identity is frozen into each Run and
is intentionally not a renderer setting or visible conversation card. The
Memory Store and typed Create/Correct/Forget/Provenance bridge are connected,
and Settings now exposes a real lazy-loaded Memory management page with filters,
pagination, provenance verification, correction, and Forget confirmation.
Model recall remains unavailable; its Runtime design is tracked in
[MODEL_INPUT_AND_MEMORY_DESIGN.md](../../../../runtime/MODEL_INPUT_AND_MEMORY_DESIGN.md).

## Protocol boundary

`domain.ts` remains a UI projection model, not a canonical wire schema. The
versioned DTOs in `src/shared/runtime.ts`, `RuntimeClient`, and the projection
reducer form the production integration boundary. Presentation components may
continue to understand richer mock event types, but those types must not be
documented or treated as Runtime capabilities until an explicit RPC/event
contract exists.
