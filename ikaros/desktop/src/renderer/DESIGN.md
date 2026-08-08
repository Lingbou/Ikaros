# Ikaros Desktop Renderer

## Locked product decisions

- `Thread -> Branch -> Turn -> Event` is the conversation hierarchy; a Thread may optionally belong to a Project.
- The sidebar shows project-owned Threads under Projects and only standalone Threads under Recents, without duplicate projections.
- Project chats and standalone chats have the same capabilities. The composer must not expose separate Agent, Chat, or Research modes.
- Agent profiles, providers, models, tools, and artifacts are orthogonal resources rather than navigation parents.
- The renderer should look and feel like a restrained dark native desktop chat client while using Ikaros general-agent language.
- The composer exposes attachments and tools progressively, keeps model and access state visible, and supports send, stop, retry, and recovery.
- Tool calls, permission requests, plans, file changes, and artifacts render as typed events instead of raw protocol JSON.
- Editing an earlier user message creates a new branch and preserves the original history.
- A deterministic mock client drives five development scenarios without requiring a live kernel.
- Artifacts, file changes, tool activity, and branch creation remain readable inline in the conversation.
- The first renderer is local-first and responsive at both 1280x800 and 1920x1080.

## Integration boundary

The renderer consumes an `AgentClient` event stream. A production client can replace `MockAgentClient` without changing domain records or presentation components.
