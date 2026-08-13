import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  activeBranch,
  externalEventText,
  findThread,
  flattenEvents,
  standaloneThreads,
  threadsForProject,
  type AgentEvent,
  type RunStatus,
  type ScenarioId,
} from "./domain";
import { LOCAL_PROFILE, profileInitials } from "./localProfile";
import { createInitialProjects, createInitialThreads } from "./mockAgentClient";
import { selectCurrentThread, useAppStore } from "./store";

const initialState = useAppStore.getInitialState();

const scenarioThreadIds: Record<ScenarioId, string> = {
  streaming: "thread-streaming",
  tools: "thread-tools",
  permission: "thread-permission",
  recovery: "thread-recovery",
  artifact: "thread-artifact",
};

function selectFixtureThread(scenario: ScenarioId) {
  return useAppStore.getState().selectThread(scenarioThreadIds[scenario]);
}

function resetStore() {
  useAppStore.setState(
    {
      ...initialState,
      projects: createInitialProjects(),
      threads: createInitialThreads(),
      expandedProjects: { ...initialState.expandedProjects },
    },
    true,
  );
}

async function settle<T>(promise: Promise<T>) {
  await vi.runAllTimersAsync();
  return promise;
}

beforeEach(() => {
  vi.useFakeTimers();
  resetStore();
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
});

describe("scenario projection into the application store", () => {
  const cases: Array<{
    scenario: ScenarioId;
    terminal: RunStatus;
    eventTypes: AgentEvent["type"][];
  }> = [
    {
      scenario: "streaming",
      terminal: "completed",
      eventTypes: ["message", "message"],
    },
    {
      scenario: "tools",
      terminal: "completed",
      eventTypes: [
        "message",
        "tool_call",
        "tool_result",
        "tool_call",
        "tool_result",
        "message",
      ],
    },
    {
      scenario: "permission",
      terminal: "waiting_permission",
      eventTypes: ["message", "permission_request"],
    },
    {
      scenario: "recovery",
      terminal: "interrupted",
      eventTypes: ["message", "tool_call", "interrupt"],
    },
    {
      scenario: "artifact",
      terminal: "completed",
      eventTypes: ["message", "status", "artifact", "file_change", "message"],
    },
  ];

  it.each(cases)(
    "projects $scenario events in stable order with terminal state $terminal",
    async ({ scenario, terminal, eventTypes }) => {
      const run = selectFixtureThread(scenario);
      expect(useAppStore.getState().runStatus).toBe("running");

      await settle(run);

      const state = useAppStore.getState();
      const thread = selectCurrentThread(state);
      const turn = activeBranch(thread)?.turns.at(-1);
      expect(state.selectedThreadId).toBe(scenarioThreadIds[scenario]);
      expect(state.runStatus).toBe(terminal);
      expect(turn?.status).toBe(terminal);
      expect(flattenEvents(thread).map(({ type }) => type)).toEqual(eventTypes);
      expect(new Set(flattenEvents(thread).map(({ id }) => id)).size).toBe(
        flattenEvents(thread).length,
      );

      if (scenario === "streaming") {
        expect(flattenEvents(thread).at(-1)).toMatchObject({
          id: "thread-streaming-assistant",
          type: "message",
          status: "complete",
        });
      }
      if (scenario === "tools") {
        expect(flattenEvents(thread)).toEqual(
          expect.arrayContaining([
            expect.objectContaining({
              id: "thread-tools-search",
              type: "tool_call",
              status: "success",
            }),
            expect.objectContaining({
              id: "thread-tools-extract",
              type: "tool_call",
              status: "error",
            }),
          ]),
        );
      }
      if (scenario === "artifact") {
        const statuses = flattenEvents(thread).filter(
          (event) => event.id === "thread-artifact-status",
        );
        expect(statuses).toHaveLength(1);
        expect(statuses[0]).toMatchObject({
          type: "status",
          tone: "success",
          label: {
            source: "app",
            kind: "status.briefBuilt",
            values: { count: 12 },
          },
        });
        expect(
          flattenEvents(thread).some(
            (event) => event.type === "status" && event.tone === "neutral",
          ),
        ).toBe(false);
      }
      if (scenario === "recovery") {
        expect(flattenEvents(thread)).toEqual(
          expect.arrayContaining([
            expect.objectContaining({
              id: "thread-recovery-export",
              type: "tool_call",
              status: "interrupted",
            }),
          ]),
        );
      }
    },
  );
});

describe("conversation ownership", () => {
  it("partitions every thread into exactly one project or the standalone chat list", () => {
    const state = useAppStore.getState();
    const ids = state.threads.map((thread) => thread.id);
    expect(new Set(ids).size).toBe(ids.length);

    const projectThreadIds = state.projects.flatMap((project) =>
      threadsForProject(state.threads, project.id).map((thread) => thread.id),
    );
    const standaloneThreadIds = standaloneThreads(state.threads).map((thread) => thread.id);

    expect(projectThreadIds).toHaveLength(5);
    expect(standaloneThreadIds).toHaveLength(3);
    expect(projectThreadIds.filter((id) => standaloneThreadIds.includes(id))).toEqual([]);
    expect(new Set([...projectThreadIds, ...standaloneThreadIds])).toEqual(new Set(ids));
    expect(
      state.threads.every(
        (thread) =>
          thread.projectId === null ||
          state.projects.some((project) => project.id === thread.projectId),
      ),
    ).toBe(true);
  });

  it("creates a standalone chat on first send without changing project membership", async () => {
    await settle(useAppStore.getState().selectThread("thread-streaming"));
    const before = useAppStore.getState();
    const projectMembership = new Map(
      before.projects.map((project) => [
        project.id,
        threadsForProject(before.threads, project.id).map((thread) => thread.id),
      ]),
    );
    const threadCount = before.threads.length;

    useAppStore.getState().newChat();
    expect(useAppStore.getState().threads).toHaveLength(threadCount);

    useAppStore.getState().setDraft("Plan a standalone conversation");
    await settle(useAppStore.getState().sendDraft());

    const state = useAppStore.getState();
    const created = selectCurrentThread(state);
    expect(state.threads).toHaveLength(threadCount + 1);
    expect(created).toMatchObject({
      id: "thread-local-1",
      projectId: null,
      title: "Plan a standalone conversation",
    });
    for (const project of state.projects) {
      expect(threadsForProject(state.threads, project.id).map((thread) => thread.id)).toEqual(
        projectMembership.get(project.id),
      );
    }
  });
});

describe("permission, recovery, and cancellation store flows", () => {
  it.each(["allow", "deny"] as const)(
    "resolves permission %s once and reaches a completed turn",
    async (decision) => {
      await settle(selectFixtureThread("permission"));
      expect(useAppStore.getState().runStatus).toBe("waiting_permission");

      await settle(useAppStore.getState().resolvePermission(decision));

      const state = useAppStore.getState();
      const thread = selectCurrentThread(state);
      const events = flattenEvents(thread);
      const permissions = events.filter((event) => event.type === "permission_request");
      expect(state.runStatus).toBe("completed");
      expect(activeBranch(thread)?.turns.at(-1)?.status).toBe("completed");
      expect(permissions).toHaveLength(1);
      expect(permissions[0]).toMatchObject({
        id: "thread-permission-permission",
        status: decision === "allow" ? "allowed" : "denied",
      });
      expect(events.at(-1)).toMatchObject({
        id: `permission-${decision}-assistant`,
        type: "message",
      });
    },
  );

  it.each(["retry", "resume"] as const)(
    "recovers the interrupted run with %s without duplicating its interrupt record",
    async (strategy) => {
      await settle(selectFixtureThread("recovery"));
      expect(useAppStore.getState().runStatus).toBe("interrupted");

      await settle(useAppStore.getState().recoverRun(strategy));

      const state = useAppStore.getState();
      const thread = selectCurrentThread(state);
      const events = flattenEvents(thread);
      expect(state.runStatus).toBe("completed");
      expect(activeBranch(thread)?.turns.at(-1)?.status).toBe("completed");
      expect(events.filter(({ id }) => id === "thread-recovery-interrupt")).toHaveLength(1);
      expect(events).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            id: "thread-recovery-export",
            type: "tool_call",
            status: "success",
          }),
          expect.objectContaining({
            id: "thread-recovery-interrupt",
            type: "interrupt",
            recoverable: false,
            status: "recovered",
          }),
          expect.objectContaining({ id: `recovery-${strategy}-status` }),
          expect.objectContaining({ id: `recovery-${strategy}-assistant` }),
        ]),
      );
      expect(
        events.some(
          (event) =>
            (event.type === "message" && event.status === "streaming") ||
            (event.type === "tool_call" && event.status === "running") ||
            (event.type === "permission_request" && event.status === "pending") ||
            (event.type === "interrupt" && event.status === "recovering"),
        ),
      ).toBe(false);
    },
  );

  it("stops a running scenario and ignores its remaining delayed revisions", async () => {
    const run = selectFixtureThread("streaming");
    await vi.advanceTimersByTimeAsync(180);
    const beforeStop = flattenEvents(selectCurrentThread(useAppStore.getState()));
    expect(beforeStop.at(-1)).toMatchObject({
      id: "thread-streaming-assistant",
      status: "streaming",
    });

    useAppStore.getState().stopRun();
    const stopped = useAppStore.getState();
    expect(stopped.runStatus).toBe("interrupted");
    expect(flattenEvents(selectCurrentThread(stopped)).at(-1)).toMatchObject({
      type: "interrupt",
      recoverable: false,
      status: "interrupted",
    });

    await vi.runAllTimersAsync();
    await run;

    const finalState = useAppStore.getState();
    const events = flattenEvents(selectCurrentThread(finalState));
    expect(finalState.runStatus).toBe("interrupted");
    expect(events.filter(({ id }) => id === "thread-streaming-assistant")).toHaveLength(1);
    expect(events.find(({ id }) => id === "thread-streaming-assistant")).toMatchObject({
      status: "complete",
    });
    expect(activeBranch(selectCurrentThread(finalState))?.turns.at(-1)?.status).toBe(
      "interrupted",
    );
  });

  it("terminalizes a running tool and rejects late callbacks after stop", async () => {
    const run = selectFixtureThread("tools");
    await vi.advanceTimersByTimeAsync(160);

    useAppStore.getState().stopRun();
    const stoppedThread = selectCurrentThread(useAppStore.getState());
    expect(flattenEvents(stoppedThread)).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: "thread-tools-search",
          type: "tool_call",
          status: "interrupted",
        }),
        expect.objectContaining({
          type: "interrupt",
          recoverable: false,
          status: "interrupted",
        }),
      ]),
    );
    const stoppedSnapshot = JSON.stringify(stoppedThread);

    await vi.runAllTimersAsync();
    await run;

    expect(JSON.stringify(findThread(useAppStore.getState().threads, "thread-tools"))).toBe(
      stoppedSnapshot,
    );
  });

  it("makes a manually stopped recovery non-recoverable even before its first callback", async () => {
    await settle(selectFixtureThread("recovery"));
    const recovery = useAppStore.getState().recoverRun("resume");

    useAppStore.getState().stopRun();
    const stopped = selectCurrentThread(useAppStore.getState());
    const interrupts = flattenEvents(stopped).filter(
      (event) => event.type === "interrupt",
    );
    expect(interrupts).toHaveLength(2);
    expect(interrupts.every((event) => !event.recoverable)).toBe(true);
    expect(interrupts.at(-1)).toMatchObject({
      copy: { source: "app", kind: "run_stopped" },
      recoverable: false,
      status: "interrupted",
    });
    const snapshot = JSON.stringify(stopped);

    await vi.runAllTimersAsync();
    await recovery;
    expect(JSON.stringify(selectCurrentThread(useAppStore.getState()))).toBe(snapshot);

    await useAppStore.getState().recoverRun("resume");
    expect(useAppStore.getState().runStatus).toBe("interrupted");
  });

  it("terminalizes a streaming turn on navigation and rejects its late revisions", async () => {
    const run = selectFixtureThread("streaming");
    await vi.advanceTimersByTimeAsync(180);

    useAppStore.getState().newChat();
    const navigated = useAppStore.getState();
    const source = findThread(navigated.threads, "thread-streaming");
    expect(navigated.selectedThreadId).toBeNull();
    expect(navigated.runStatus).toBe("idle");
    expect(activeBranch(source)?.turns.at(-1)?.status).toBe("interrupted");
    expect(flattenEvents(source).at(-1)).toMatchObject({
      id: "thread-streaming-assistant",
      type: "message",
      status: "complete",
    });
    const sourceSnapshot = JSON.stringify(source);

    await vi.runAllTimersAsync();
    await run;

    expect(
      JSON.stringify(findThread(useAppStore.getState().threads, "thread-streaming")),
    ).toBe(sourceSnapshot);
  });

  it("preserves a pending permission across navigation and restores it without replay", async () => {
    await settle(useAppStore.getState().selectThread("thread-permission"));
    const pending = findThread(useAppStore.getState().threads, "thread-permission");
    const pendingSnapshot = JSON.stringify(pending);
    expect(useAppStore.getState().runStatus).toBe("waiting_permission");

    useAppStore.getState().newChat();
    expect(
      JSON.stringify(findThread(useAppStore.getState().threads, "thread-permission")),
    ).toBe(pendingSnapshot);

    await useAppStore.getState().selectThread("thread-permission");
    const restored = useAppStore.getState();
    expect(restored.runStatus).toBe("waiting_permission");
    expect(JSON.stringify(selectCurrentThread(restored))).toBe(pendingSnapshot);
    expect(
      flattenEvents(selectCurrentThread(restored)).filter(
        (event) => event.type === "permission_request",
      ),
    ).toHaveLength(1);
  });
});

describe("core store invariants", () => {
  it("replaces repeated event IDs in place instead of duplicating them", () => {
    const threadId = "thread-tools";
    const turnId = "thread-tools-turn";
    const running: AgentEvent = {
      id: "stable-tool-call",
      turnId,
      type: "tool_call",
      toolName: "archive.inspect",
      label: externalEventText("Inspect archive"),
      status: "running",
      arguments: {},
      createdAt: "2026-08-05T06:00:01.000Z",
    };
    useAppStore
      .getState()
      .appendAgentEvent(threadId, "thread-tools-main", running, "running");
    useAppStore.getState().appendAgentEvent(
      threadId,
      "thread-tools-main",
      { ...running, status: "success", durationMs: 42 },
      "completed",
    );

    const thread = findThread(useAppStore.getState().threads, threadId);
    const matches = flattenEvents(thread).filter(({ id }) => id === running.id);
    expect(matches).toHaveLength(1);
    expect(matches[0]).toMatchObject({ status: "success", durationMs: 42 });
    expect(activeBranch(thread)?.turns[0].status).toBe("completed");
    expect(useAppStore.getState().runStatus).toBe("completed");
  });

  it("creates an edited branch while preserving and allowing return to the original", () => {
    const threadId = "thread-streaming";
    const sourceEventId = "thread-streaming-user";
    const original = findThread(useAppStore.getState().threads, threadId);
    const originalContent = flattenEvents(original).find(
      ({ id }) => id === sourceEventId,
    );
    expect(originalContent).toMatchObject({ type: "message" });

    useAppStore.setState({
      selectedThreadId: threadId,
      runStatus: "completed",
    });
    useAppStore.getState().beginEditMessage(sourceEventId, "original draft");
    useAppStore.getState().commitMessageEdit("  Replacement branch prompt  ");

    const state = useAppStore.getState();
    const branched = selectCurrentThread(state);
    expect(branched?.branches).toHaveLength(2);
    expect(branched?.activeBranchId).toBe("thread-streaming-branch-1");
    expect(state.selectedThreadId).toBe(threadId);
    expect(state.editingMessage).toBeNull();
    expect(branched?.branches[0].turns[0].events[0]).toEqual(originalContent);
    expect(flattenEvents(branched)[0]).toMatchObject({
      id: sourceEventId,
      type: "message",
      content: "Replacement branch prompt",
    });
    expect(flattenEvents(branched)[1]).toMatchObject({
      type: "branch_created",
      sourceEventId,
      copy: { source: "app", kind: "edited_message" },
    });
    expect(branched?.branches[1].label).toEqual({
      source: "app",
      kind: "number",
      number: 1,
    });

    useAppStore.getState().switchBranch("thread-streaming-main");
    const restored = selectCurrentThread(useAppStore.getState());
    expect(restored?.activeBranchId).toBe("thread-streaming-main");
    expect(flattenEvents(restored)[0]).toEqual(originalContent);
  });

  it("restores an interrupted branch status after editing a new branch", async () => {
    await settle(selectFixtureThread("recovery"));
    expect(useAppStore.getState().runStatus).toBe("interrupted");

    useAppStore
      .getState()
      .beginEditMessage("thread-recovery-user", "original recovery prompt");
    useAppStore.getState().commitMessageEdit("Explore a different recovery path");
    expect(useAppStore.getState().runStatus).toBe("completed");

    useAppStore.getState().switchBranch("thread-recovery-main");
    expect(useAppStore.getState().runStatus).toBe("interrupted");

    const recovery = useAppStore.getState().recoverRun("resume");
    expect(useAppStore.getState().runStatus).toBe("running");
    await settle(recovery);

    const state = useAppStore.getState();
    expect(state.runStatus).toBe("completed");
    expect(
      flattenEvents(selectCurrentThread(state)).find(
        ({ id }) => id === "thread-recovery-interrupt",
      ),
    ).toMatchObject({ type: "interrupt", status: "recovered" });
  });

  it.each(["queued", "running", "waiting_permission"] as const)(
    "blocks edit-and-branch while the run is %s",
    (runStatus) => {
      const threadId = "thread-streaming";
      const sourceEventId = "thread-streaming-user";
      useAppStore.setState({ selectedThreadId: threadId, runStatus });

      useAppStore.getState().beginEditMessage(sourceEventId, "original");
      expect(useAppStore.getState().editingMessage).toBeNull();

      useAppStore.setState({
        editingMessage: { eventId: sourceEventId, content: "original" },
      });
      useAppStore.getState().commitMessageEdit("replacement");
      const thread = findThread(useAppStore.getState().threads, threadId);
      expect(thread?.branches).toHaveLength(1);
      expect(useAppStore.getState().editingMessage).toEqual({
        eventId: sourceEventId,
        content: "original",
      });
    },
  );

  it.each(["queued", "running", "waiting_permission"] as const)(
    "preserves the draft and refuses submission while the run is %s",
    async (runStatus) => {
      const threadId = "thread-streaming";
      const before = findThread(useAppStore.getState().threads, threadId);
      useAppStore.setState({
        selectedThreadId: threadId,
        runStatus,
        draft: "Keep this draft",
      });

      await useAppStore.getState().sendDraft();

      expect(useAppStore.getState().draft).toBe("Keep this draft");
      expect(findThread(useAppStore.getState().threads, threadId)).toEqual(before);
    },
  );

  it("plays an untouched sidebar fixture once and preserves follow-up and branch history on revisit", async () => {
    const firstPlay = useAppStore.getState().selectThread("thread-streaming");
    const started = selectCurrentThread(useAppStore.getState());
    expect(useAppStore.getState().runStatus).toBe("running");
    expect(activeBranch(started)?.turns.at(-1)?.status).toBe("running");
    await settle(firstPlay);

    useAppStore.getState().setDraft("A follow-up that must survive navigation");
    await settle(useAppStore.getState().sendDraft());
    useAppStore
      .getState()
      .beginEditMessage("thread-streaming-turn-2-user", "follow-up");
    useAppStore.getState().commitMessageEdit("Edited follow-up branch");

    const beforeNavigation = findThread(
      useAppStore.getState().threads,
      "thread-streaming",
    );
    expect(beforeNavigation?.branches).toHaveLength(2);
    expect(beforeNavigation?.branches[0].turns).toHaveLength(2);
    const historySnapshot = JSON.stringify(beforeNavigation);

    useAppStore.getState().newChat();
    await useAppStore.getState().selectThread("thread-streaming");

    const revisited = useAppStore.getState();
    expect(revisited.runStatus).toBe("completed");
    expect(JSON.stringify(selectCurrentThread(revisited))).toBe(historySnapshot);
  });

  it("keeps mid-stream callbacks on their captured branch after the active branch changes", async () => {
    await settle(selectFixtureThread("streaming"));
    useAppStore
      .getState()
      .beginEditMessage("thread-streaming-user", "original prompt");
    useAppStore.getState().commitMessageEdit("Branch-only prompt");
    useAppStore.getState().switchBranch("thread-streaming-main");

    useAppStore.getState().setDraft("Main branch follow-up");
    const run = useAppStore.getState().sendDraft();
    await vi.advanceTimersByTimeAsync(160);
    useAppStore.getState().switchBranch("thread-streaming-branch-1");
    expect(useAppStore.getState().runStatus).toBe("running");
    await settle(run);

    const thread = selectCurrentThread(useAppStore.getState());
    const main = thread?.branches.find((branch) => branch.id === "thread-streaming-main");
    const fork = thread?.branches.find(
      (branch) => branch.id === "thread-streaming-branch-1",
    );
    expect(thread?.activeBranchId).toBe("thread-streaming-branch-1");
    expect(main?.turns).toHaveLength(2);
    expect(main?.turns[1]).toMatchObject({
      id: "thread-streaming-turn-2",
      status: "completed",
    });
    expect(main?.turns[1].events.at(-1)).toMatchObject({
      type: "message",
      role: "assistant",
      status: "complete",
    });
    expect(fork?.turns).toHaveLength(1);
    expect(
      fork?.turns.flatMap((turn) => turn.events).some(
        (event) => event.turnId === "thread-streaming-turn-2",
      ),
    ).toBe(false);
  });

  it("treats settings as a page and returns to the app on conversation navigation", () => {
    useAppStore.setState({
      searchOpen: true,
      settingsOpen: false,
      editingMessage: { eventId: "draft-event", content: "draft" },
    });

    useAppStore.getState().setSettingsOpen(true);

    expect(useAppStore.getState()).toMatchObject({
      searchOpen: false,
      settingsOpen: true,
      editingMessage: null,
    });

    useAppStore.getState().newChat();
    expect(useAppStore.getState().settingsOpen).toBe(false);
  });

  it("keeps the projected profile username in Zustand state", () => {
    expect(useAppStore.getState().profileUsername).toBe(LOCAL_PROFILE.name);
    expect(useAppStore.getInitialState().profileUsername).toBe(LOCAL_PROFILE.name);

    useAppStore.getState().setProfileUsername("March Seven");

    expect(useAppStore.getState().profileUsername).toBe("March Seven");
    expect(useAppStore.getInitialState().profileUsername).toBe(LOCAL_PROFILE.name);

    resetStore();
    expect(useAppStore.getState().profileUsername).toBe(LOCAL_PROFILE.name);
  });

  it("keeps sidebar width as renderer state for drag previews", () => {
    expect(useAppStore.getState().sidebarWidth).toBe(260);

    useAppStore.getState().setSidebarWidth(404);

    expect(useAppStore.getState().sidebarWidth).toBe(404);
    expect(useAppStore.getInitialState().sidebarWidth).toBe(260);
  });

  it("stages a new project workspace without fabricating a Runtime Thread", () => {
    const workspace = {
      id: "workspace-new",
      name: "Named project",
      rootUri: "C:\\Workspace\\named-project",
    };
    const beforeThreadIds = useAppStore.getState().threads.map((thread) => thread.id);

    useAppStore.getState().stageProjectWorkspace(workspace);

    const state = useAppStore.getState();
    expect(state.projects).toContainEqual({
      ...workspace,
      color: "var(--muted-strong)",
    });
    expect(state.threads.map((thread) => thread.id)).toEqual(beforeThreadIds);
    expect(state.newThreadWorkspace).toEqual(workspace);
    expect(state.selectedThreadId).toBeNull();
    expect(state.expandedProjects[workspace.id]).toBe(true);
  });

  it("reuses the canonical project snapshot when a staged workspace id already exists", () => {
    const existing = useAppStore.getState().projects[0];
    if (!existing) throw new Error("Expected a project fixture");
    const beforeProjects = useAppStore.getState().projects;

    useAppStore.getState().stageProjectWorkspace({
      id: existing.id,
      name: "Ignored rename",
      rootUri: "C:\\Different\\root",
    });

    const state = useAppStore.getState();
    expect(state.projects).toBe(beforeProjects);
    expect(state.newThreadWorkspace).toEqual({
      id: existing.id,
      name: existing.name,
      rootUri: existing.rootUri ?? null,
    });
    expect(state.projects.filter((project) => project.id === existing.id)).toHaveLength(1);
  });

  it.each([
    ["hc", "HC"],
    ["  ikaros  ", "IK"],
    ["March Seven", "MS"],
    ["  March   Seventh  ", "MS"],
    ["伊卡洛斯", "伊卡"],
    ["   ", "US"],
  ])("derives profile initials from %j", (username, expected) => {
    expect(profileInitials(username)).toBe(expected);
  });
});
