import { afterEach, describe, expect, it, vi } from "vitest";

import type { IkarosDesktopApi } from "../shared/platform";
import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  type IkarosRuntimeApi,
  type RuntimeInvocationResult,
  type RuntimeJournalEvent,
  type RuntimeItemHistory,
  type RuntimeModelSummary,
  type RuntimeProviderSummary,
  type RuntimeSkillSummary,
  type RuntimeThreadCreateParams,
  type RuntimeThreadListPage,
  type RuntimeThreadSummary,
  type RuntimeTurnListPage,
} from "../shared/runtime";

const createdAt = "2026-08-11T12:00:00.000Z";

function definitiveRpcError(message: string, code = -32602): Error {
  return Object.assign(new Error(message), {
    name: "RuntimeRpcError",
    kind: "json_rpc" as const,
    code
  });
}

async function bridgeInvocation<TResult>(
  operation: () => Promise<TResult>,
): Promise<RuntimeInvocationResult<TResult>> {
  try {
    return { ok: true, value: await operation() };
  } catch (error) {
    if (
      typeof error === "object" &&
      error !== null &&
      (error as { kind?: unknown }).kind === "json_rpc" &&
      typeof (error as { code?: unknown }).code === "number" &&
      Number.isInteger((error as { code: number }).code) &&
      typeof (error as { message?: unknown }).message === "string"
    ) {
      return {
        ok: false,
        error: {
          kind: "json_rpc",
          code: (error as { code: number }).code,
          message: (error as { message: string }).message,
        },
      };
    }
    throw error;
  }
}

function installRuntimeBridge(api: unknown): void {
  const desktop = api as { runtime: IkarosRuntimeApi };
  const runtime = desktop.runtime;
  let listedThreads: RuntimeThreadSummary[] = [];
  let catalogSnapshotSeq = 0;
  const providerCatalog = {
    providers: [
      {
        id: "test-provider",
        displayName: "Test Provider",
        origin: "custom" as const,
        configured: true,
        credentialConfigured: false,
        health: "unknown" as const,
      },
    ],
  };
  const modelCatalog = {
    models: [
      {
        providerId: "test-provider",
        id: "test-model",
        displayName: "Test Model",
        enabled: true,
      },
    ],
  };
  const bridged = {
    ...(api as object),
    runtime: {
      listThreads: (params) =>
        bridgeInvocation(async () => {
          const listed = await runtime.listThreads(params);
          listedThreads = listed.threads.map((thread) => ({
            ...thread,
            workspace: thread.workspace ?? null,
            archivedAt: thread.archivedAt ?? null,
          }));
          catalogSnapshotSeq =
            typeof listed.snapshotSeq === "number" ? listed.snapshotSeq : 0;
          return {
            threads: listedThreads,
            nextCursor: listed.nextCursor ?? null,
            hasMore: listed.hasMore ?? false,
            snapshotSeq: catalogSnapshotSeq,
          };
        }),
      getThread: (threadId: string) =>
        bridgeInvocation(async () => {
          if (typeof runtime.getThread === "function") {
            return runtime.getThread(threadId);
          }
          const thread = listedThreads.find((candidate) => candidate.id === threadId);
          if (!thread) throw new Error("thread was not found");
          return { thread, snapshotSeq: catalogSnapshotSeq };
        }),
      listTurns: (params: Parameters<IkarosRuntimeApi["listTurns"]>[0]) =>
        bridgeInvocation(() =>
          typeof runtime.listTurns === "function"
            ? runtime.listTurns(params)
            : Promise.resolve({
                turns: [],
                nextCursor: null,
                hasMore: false,
                snapshotSeq: catalogSnapshotSeq,
              }),
        ),
      createThread: (params: Parameters<IkarosRuntimeApi["createThread"]>[0]) =>
        bridgeInvocation(() => runtime.createThread(params)),
      renameThread: (params: Parameters<IkarosRuntimeApi["renameThread"]>[0]) =>
        bridgeInvocation(() => runtime.renameThread(params)),
      archiveThread: (threadId: string) =>
        bridgeInvocation(() => runtime.archiveThread(threadId)),
      unarchiveThread: (threadId: string) =>
        bridgeInvocation(() => runtime.unarchiveThread(threadId)),
      startTurn: (params: Parameters<IkarosRuntimeApi["startTurn"]>[0]) =>
        bridgeInvocation(() => runtime.startTurn(params)),
      cancelRun: (runId: string) => bridgeInvocation(() => runtime.cancelRun(runId)),
      replayEvents: (afterSeq: number, limit?: number) =>
        bridgeInvocation(() => runtime.replayEvents(afterSeq, limit)),
      listProviders: () =>
        bridgeInvocation(() =>
          typeof runtime.listProviders === "function"
            ? runtime.listProviders()
            : Promise.resolve(providerCatalog),
        ),
      configureProvider: (params: Parameters<IkarosRuntimeApi["configureProvider"]>[0]) =>
        bridgeInvocation(() => runtime.configureProvider(params)),
      discoverProviderModels: (
        params: Parameters<IkarosRuntimeApi["discoverProviderModels"]>[0],
      ) => bridgeInvocation(() => runtime.discoverProviderModels(params)),
      disconnectProvider: (providerId: "deepseek") =>
        bridgeInvocation(() => runtime.disconnectProvider(providerId)),
      removeProvider: (providerId: string) =>
        bridgeInvocation(() => runtime.removeProvider(providerId)),
      listModels: () =>
        bridgeInvocation(() =>
          typeof runtime.listModels === "function"
            ? runtime.listModels()
            : Promise.resolve(modelCatalog),
        ),
      setModelEnabled: (params: Parameters<IkarosRuntimeApi["setModelEnabled"]>[0]) =>
        bridgeInvocation(() => runtime.setModelEnabled(params)),
      listSkills: () =>
        bridgeInvocation(() =>
          typeof runtime.listSkills === "function"
            ? runtime.listSkills()
            : Promise.resolve({ skills: [], diagnostics: [] }),
        ),
      setSkillEnabled: (params: Parameters<IkarosRuntimeApi["setSkillEnabled"]>[0]) =>
        bridgeInvocation(() => runtime.setSkillEnabled(params)),
      previewFile: (params: Parameters<IkarosRuntimeApi["previewFile"]>[0]) =>
        bridgeInvocation(() => runtime.previewFile(params)),
      getFileChange: (params: Parameters<IkarosRuntimeApi["getFileChange"]>[0]) =>
        bridgeInvocation(() => runtime.getFileChange(params)),
      readUsage: () =>
        bridgeInvocation(() =>
          typeof runtime.readUsage === "function"
            ? runtime.readUsage()
            : Promise.resolve({
                summary: {
                  lifetimeTokens: null,
                  peakDailyTokens: null,
                  longestRunningTurnSec: null,
                  currentStreakDays: 0,
                  longestStreakDays: 0,
                },
                dailyUsageBuckets: [],
              }),
        ),
      onEvent: runtime.onEvent,
    },
  } as IkarosDesktopApi;
  Object.defineProperty(window, "ikarosDesktop", {
    configurable: true,
    value: bridged,
  });
}

function runtimeEvent(
  seq: number,
  type: RuntimeJournalEvent["type"],
  itemId: string | null,
  payload: Record<string, unknown>,
  identity: Partial<
    Pick<RuntimeJournalEvent, "threadId" | "branchId" | "turnId" | "runId">
  > = {}
): RuntimeJournalEvent {
  return {
    seq,
    schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
    type,
    threadId: identity.threadId ?? "thread-runtime",
    branchId: identity.branchId ?? "branch-runtime",
    turnId: identity.turnId ?? "turn-runtime",
    runId: identity.runId ?? "run-runtime",
    itemId,
    timestamp: createdAt,
    payload
  };
}

function messageItem(
  id: string,
  role: "user" | "assistant",
  content: string,
  status: string
) {
  return {
    id,
    turnId: "turn-runtime",
    runId: "run-runtime",
    ordinal: role === "user" ? 1 : 2,
    kind: "message",
    role,
    status,
    content,
    createdAt,
    updatedAt: createdAt
  };
}

function historyMessageItem(
  id: string,
  role: "user" | "assistant",
  content: string,
  status: "streaming" | "completed" | "failed" | "cancelled",
  ordinal: number,
): RuntimeItemHistory {
  return {
    ...messageItem(id, role, content, status),
    ordinal,
    kind: "message",
    role,
    status,
    data: {},
  };
}

function singleTurnHistoryPage(
  thread: Pick<RuntimeThreadSummary, "id" | "defaultBranchId">,
  items: RuntimeItemHistory[],
  status: "queued" | "running" | "completed" | "failed" | "cancelled",
  snapshotSeq: number,
  identity: { turnId?: string; runId?: string } = {},
): RuntimeTurnListPage {
  const turnId = identity.turnId ?? "turn-runtime";
  const runId = identity.runId ?? "run-runtime";
  return {
    turns: [
      {
        id: turnId,
        threadId: thread.id,
        branchId: thread.defaultBranchId,
        ordinal: 1,
        status,
        createdAt,
        updatedAt: createdAt,
        runs: [
          {
            id: runId,
            turnId,
            providerId: "scripted",
            modelId: "scripted-v1",
            executionPolicy: "full_access",
            status,
            reasonCode: null,
            createdAt,
            settledAt:
              status === "completed" || status === "failed" || status === "cancelled"
                ? createdAt
                : null,
            items: items.map((item) => ({ ...item, turnId, runId })),
          },
        ],
      },
    ],
    nextCursor: null,
    hasMore: false,
    snapshotSeq,
  };
}

afterEach(() => {
  vi.useRealTimers();
  vi.resetModules();
  Reflect.deleteProperty(window, "ikarosDesktop");
});

describe("Runtime-backed renderer store", () => {
  it("renames, archives, and restores a Thread through Runtime mutation events", async () => {
    const thread: RuntimeThreadSummary = {
      id: "thread-lifecycle",
      title: "Original",
      defaultBranchId: "branch-lifecycle",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
      archivedAt: null,
    };
    const renamed = {
      ...thread,
      title: "Renamed",
      updatedAt: "2026-08-11T12:01:00.000Z",
    };
    const archived = {
      ...renamed,
      updatedAt: "2026-08-11T12:02:00.000Z",
      archivedAt: "2026-08-11T12:02:00.000Z",
    };
    const restored = {
      ...archived,
      updatedAt: "2026-08-11T12:03:00.000Z",
      archivedAt: null,
    };
    const mutation = (
      seq: number,
      type: "thread.renamed" | "thread.archived" | "thread.unarchived",
      summary: RuntimeThreadSummary,
    ) => ({
      thread: summary,
      changed: true,
      event: runtimeEvent(seq, type, null, { thread: summary }, {
        threadId: summary.id,
        branchId: summary.defaultBranchId,
        turnId: null,
        runId: null,
      }),
    });
    const renameThread = vi.fn(async () => mutation(2, "thread.renamed", renamed));
    const archiveThread = vi.fn(async () => mutation(3, "thread.archived", archived));
    const unarchiveThread = vi.fn(async () => mutation(4, "thread.unarchived", restored));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 1 })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 1 })),
        listTurns: vi.fn(async () => ({
          turns: [],
          nextCursor: null,
          hasMore: false,
          snapshotSeq: 1,
        })),
        createThread: vi.fn(),
        renameThread,
        archiveThread,
        unarchiveThread,
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: afterSeq,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);
    await useAppStore.getState().renameThread(thread.id, "Renamed");
    expect(useAppStore.getState().threads[0]?.title).toBe("Renamed");
    expect(renameThread).toHaveBeenCalledWith({ threadId: thread.id, title: "Renamed" });

    await useAppStore.getState().archiveThread(thread.id);
    expect(useAppStore.getState().threads).toEqual([]);
    expect(useAppStore.getState().selectedThreadId).toBeNull();
    expect(useAppStore.getState().archivedThreads).toEqual([archived]);

    await useAppStore.getState().unarchiveThread(thread.id);
    expect(useAppStore.getState().threads[0]).toMatchObject({
      id: thread.id,
      title: "Renamed",
    });
    expect(useAppStore.getState().archivedThreads).toEqual([]);
    expect(useAppStore.getState().runtimeSeq).toBe(4);
  });

  it("does not restore a live-unarchived Thread from an older archived catalog response", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const archived: RuntimeThreadSummary = {
      id: "thread-archived-race",
      title: "Archived before refresh",
      defaultBranchId: "branch-archived-race",
      workspace: null,
      createdAt,
      updatedAt: "2026-08-11T12:01:00.000Z",
      archivedAt: "2026-08-11T12:01:00.000Z",
    };
    const restored: RuntimeThreadSummary = {
      ...archived,
      updatedAt: "2026-08-11T12:02:00.000Z",
      archivedAt: null,
    };
    let resolveArchivedCatalog!: (value: {
      threads: RuntimeThreadSummary[];
      nextCursor: string | null;
      hasMore: boolean;
      snapshotSeq: number;
    }) => void;
    const archivedCatalog = new Promise<{
      threads: RuntimeThreadSummary[];
      nextCursor: string | null;
      hasMore: boolean;
      snapshotSeq: number;
    }>((resolve) => {
      resolveArchivedCatalog = resolve;
    });
    const listThreads = vi
      .fn<IkarosRuntimeApi["listThreads"]>()
      .mockImplementation((params) =>
        params?.archived
          ? archivedCatalog
          : Promise.resolve({
              threads: [],
              nextCursor: null,
              hasMore: false,
              snapshotSeq: 10,
            }),
      );
    const api = {
      runtime: {
        listThreads,
        getThread: vi.fn(async () => ({ thread: restored, snapshotSeq: 11 })),
        listTurns: vi.fn(),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: 10,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        }),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    const loading = useAppStore.getState().loadArchivedThreads();
    await vi.waitFor(() => expect(listThreads).toHaveBeenCalledTimes(2));
    for (const listener of listeners) {
      listener(
        runtimeEvent(11, "thread.unarchived", null, { thread: restored }, {
          threadId: restored.id,
          branchId: restored.defaultBranchId,
          turnId: null,
          runId: null,
        }),
      );
    }
    resolveArchivedCatalog({
      threads: [archived],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 10,
    });
    await loading;

    expect(useAppStore.getState()).toMatchObject({
      archivedThreads: [],
      archivedCatalogStatus: "ready",
      runtimeSeq: 11,
    });
    expect(useAppStore.getState().threads).toHaveLength(1);
    expect(useAppStore.getState().threads[0]).toMatchObject({
      id: restored.id,
      title: restored.title,
    });
  });

  it("creates project and ordinary chats through the same Thread flow with distinct workspace snapshots", async () => {
    const pickedWorkspace = {
      id: "workspace-ikaros",
      name: "Ikaros",
      rootUri: "C:\\Workspace\\github\\Ikaros",
    };
    const workspace = { ...pickedWorkspace, name: "Ikaros Runtime" };
    const createdThreads = [
      {
        id: "thread-project",
        title: "project request",
        defaultBranchId: "branch-project",
        workspace,
        createdAt,
        updatedAt: createdAt,
      },
      {
        id: "thread-ordinary",
        title: "ordinary request",
        defaultBranchId: "branch-ordinary",
        workspace: null,
        createdAt,
        updatedAt: createdAt,
      },
    ];
    let seq = 0;
    const createThread = vi.fn(async (params: RuntimeThreadCreateParams) => {
      const thread = createdThreads[seq] as (typeof createdThreads)[number];
      seq += 1;
      const event: RuntimeJournalEvent = {
        seq,
        schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
        type: "thread.created",
        threadId: thread.id,
        branchId: thread.defaultBranchId,
        turnId: null,
        runId: null,
        itemId: null,
        timestamp: createdAt,
        payload: {
          clientRequestId: params.clientRequestId,
          thread,
          branch: {
            id: thread.defaultBranchId,
            threadId: thread.id,
            createdAt,
            isDefault: true,
          },
        },
      };
      return { thread, event };
    });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread,
        startTurn: vi.fn(async (params: { threadId: string; branchId: string }) => ({
          ...params,
          turnId: `turn-${params.threadId}`,
          runId: `run-${params.threadId}`,
        })),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false,
        })),
        onEvent: vi.fn(() => () => undefined),
      },
      workspace: { chooseDirectory: vi.fn(async () => pickedWorkspace) },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().stageProjectWorkspace(workspace);
    useAppStore.getState().setDraft("project request");
    await useAppStore.getState().sendDraft();
    useAppStore.getState().newChat();
    useAppStore.getState().setDraft("ordinary request");
    await useAppStore.getState().sendDraft();

    expect(createThread.mock.calls.map((call) => call[0].workspace)).toEqual([
      workspace,
      null,
    ]);
    expect(useAppStore.getState().projects).toEqual([
      expect.objectContaining({ id: workspace.id, name: workspace.name }),
    ]);
    expect(
      useAppStore.getState().threads.map((thread) => [thread.id, thread.projectId]),
    ).toEqual([
      ["thread-ordinary", null],
      ["thread-project", workspace.id],
    ]);
  });

  it("projects a Run that settles before turn.start returns without Mock fixtures", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "hello runtime",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const createdEvent: RuntimeJournalEvent = {
      seq: 1,
      schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
      type: "thread.created",
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: null,
      runId: null,
      itemId: null,
      timestamp: createdAt,
      payload: {
        thread,
        branch: {
          id: thread.defaultBranchId,
          threadId: thread.id,
          createdAt,
          isDefault: true
        }
      }
    };
    const startTurn = vi.fn(async (_params: { clientRequestId?: string }) => ({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: "turn-runtime",
      runId: "run-runtime"
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread: vi.fn(async () => ({ thread, event: createdEvent })),
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        })
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    expect(useAppStore.getState().runtimeMode).toBe(true);
    expect(useAppStore.getState().threads).toEqual([]);
    expect(useAppStore.getState().projects).toEqual([]);
    await useAppStore.getState().initializeRuntime();
    expect(useAppStore.getState().selectedModel).toEqual({
      providerId: "test-provider",
      modelId: "test-model",
    });
    const events = [
      runtimeEvent(2, "item.completed", "user-runtime", {
        item: messageItem("user-runtime", "user", "hello runtime", "completed")
      }),
      runtimeEvent(3, "run.state_changed", null, { status: "queued" }),
      runtimeEvent(4, "run.state_changed", null, { status: "running" }),
      runtimeEvent(5, "item.started", "assistant-runtime", {
        item: messageItem("assistant-runtime", "assistant", "", "streaming")
      }),
      runtimeEvent(6, "item.delta", "assistant-runtime", { delta: "hello " }),
      runtimeEvent(7, "item.delta", "assistant-runtime", { delta: "back" }),
      runtimeEvent(8, "item.completed", "assistant-runtime", {
        item: messageItem("assistant-runtime", "assistant", "hello back", "completed")
      }),
      runtimeEvent(9, "run.settled", null, { status: "completed" })
    ];
    startTurn.mockImplementationOnce(async () => {
      for (const event of events) {
        for (const listener of listeners) {
          listener(event);
        }
      }
      return {
        threadId: thread.id,
        branchId: thread.defaultBranchId,
        turnId: "turn-runtime",
        runId: "run-runtime"
      };
    });
    useAppStore.getState().setDraft("hello runtime");
    await useAppStore.getState().sendDraft();

    const state = useAppStore.getState();
    const turn = state.threads[0]?.branches[0]?.turns[0];
    expect(startTurn).toHaveBeenCalledWith(
      expect.objectContaining({
        threadId: thread.id,
        branchId: thread.defaultBranchId,
        content: "hello runtime",
        providerId: "test-provider",
        modelId: "test-model",
        clientRequestId: expect.any(String)
      })
    );
    expect(state.runStatus).toBe("completed");
    expect(state.activeRun).toBeNull();
    expect(turn).toMatchObject({
      runId: "run-runtime",
      status: "completed",
      events: [
        { role: "user", content: "hello runtime" },
        { role: "assistant", content: "hello back", status: "complete" }
      ]
    });
  });

  it("keeps a Runtime draft when no runnable model is selected", async () => {
    const createThread = vi.fn();
    const startTurn = vi.fn();
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread,
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false,
        })),
        listProviders: vi.fn(async () => ({
          providers: [
            {
              id: "deepseek",
              displayName: "DeepSeek",
              origin: "builtin" as const,
              configured: false,
              credentialConfigured: false,
              health: "unknown" as const,
            },
          ],
        })),
        listModels: vi.fn(async () => ({
          models: [
            {
              providerId: "deepseek",
              id: "deepseek-chat",
              displayName: "DeepSeek Chat",
              enabled: true,
            },
          ],
        })),
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    expect(useAppStore.getState().selectedModel).toBeNull();
    useAppStore.getState().setDraft("keep me");
    await useAppStore.getState().sendDraft();

    expect(createThread).not.toHaveBeenCalled();
    expect(startTurn).not.toHaveBeenCalled();
    expect(useAppStore.getState().draft).toBe("keep me");
    expect(useAppStore.getState().runtimeError).toBe("No configured model is selected.");
  });

  it("snapshots the selected model when sendDraft is invoked", async () => {
    const thread = {
      id: "thread-runtime",
      title: "Model snapshot",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt,
    };
    const startTurn = vi.fn(async () => ({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: "turn-runtime",
      runId: "run-runtime",
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread] })),
        createThread: vi.fn(),
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false,
        })),
        listProviders: vi.fn(async () => ({
          providers: [
            {
              id: "test-provider",
              displayName: "Test Provider",
              origin: "custom" as const,
              configured: true,
              credentialConfigured: false,
              health: "unknown" as const,
            },
          ],
        })),
        listModels: vi.fn(async () => ({
          models: [
            {
              providerId: "test-provider",
              id: "model-a",
              displayName: "Model A",
              enabled: true,
            },
            {
              providerId: "test-provider",
              id: "model-b",
              displayName: "Model B",
              enabled: true,
            },
          ],
        })),
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);
    useAppStore.getState().selectModel({ providerId: "test-provider", modelId: "model-a" });
    useAppStore.getState().setDraft("use the selected model");

    const sending = useAppStore.getState().sendDraft();
    useAppStore.getState().selectModel({ providerId: "test-provider", modelId: "model-b" });
    await sending;

    expect(startTurn).toHaveBeenCalledWith(
      expect.objectContaining({
        threadId: thread.id,
        providerId: "test-provider",
        modelId: "model-a",
        content: "use the selected model",
      }),
    );
    expect(useAppStore.getState().selectedModel).toEqual({
      providerId: "test-provider",
      modelId: "model-b",
    });
  });

  it("reconciles model selection after disable, disconnect, and remove mutations", async () => {
    let providers: RuntimeProviderSummary[] = [
      {
        id: "deepseek",
        displayName: "DeepSeek",
        origin: "builtin",
        configured: true,
        credentialConfigured: true,
        health: "unknown",
      },
      {
        id: "custom-provider",
        displayName: "Custom Provider",
        origin: "custom",
        configured: true,
        credentialConfigured: false,
        health: "unknown",
      },
    ];
    let models: RuntimeModelSummary[] = [
      {
        providerId: "deepseek",
        id: "deepseek-chat",
        displayName: "DeepSeek Chat",
        enabled: true,
      },
      {
        providerId: "custom-provider",
        id: "custom-model",
        displayName: "Custom Model",
        enabled: true,
      },
    ];
    const setModelEnabled = vi.fn(async ({
      providerId,
      modelId,
      enabled,
    }: {
      providerId: string;
      modelId: string;
      enabled: boolean;
    }) => {
      models = models.map((model) =>
        model.providerId === providerId && model.id === modelId
          ? { ...model, enabled }
          : model,
      );
      return models.find((model) => model.providerId === providerId && model.id === modelId);
    });
    const disconnectProvider = vi.fn(async () => {
      providers = providers.map((provider) =>
        provider.id === "deepseek"
          ? { ...provider, configured: false, credentialConfigured: false }
          : provider,
      );
      models = models.filter((model) => model.providerId !== "deepseek");
      return providers.find((provider) => provider.id === "deepseek");
    });
    const removeProvider = vi.fn(async (providerId: string) => {
      providers = providers.filter((provider) => provider.id !== providerId);
      models = models.filter((model) => model.providerId !== providerId);
      return { removed: true, providerId };
    });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false,
        })),
        listProviders: vi.fn(async () => ({ providers })),
        listModels: vi.fn(async () => ({ models })),
        setModelEnabled,
        disconnectProvider,
        removeProvider,
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().selectModel({ providerId: "deepseek", modelId: "deepseek-chat" });
    await useAppStore.getState().setModelEnabled({
      providerId: "deepseek",
      modelId: "deepseek-chat",
      enabled: false,
    });
    expect(useAppStore.getState().selectedModel).toEqual({
      providerId: "custom-provider",
      modelId: "custom-model",
    });

    await useAppStore.getState().setModelEnabled({
      providerId: "deepseek",
      modelId: "deepseek-chat",
      enabled: true,
    });
    useAppStore.getState().selectModel({ providerId: "deepseek", modelId: "deepseek-chat" });
    await useAppStore.getState().disconnectProvider("deepseek");
    expect(useAppStore.getState().models.some((model) => model.providerId === "deepseek")).toBe(
      false,
    );
    expect(useAppStore.getState().selectedModel).toEqual({
      providerId: "custom-provider",
      modelId: "custom-model",
    });

    await useAppStore.getState().removeProvider("custom-provider");
    expect(useAppStore.getState().selectedModel).toBeNull();
    expect(disconnectProvider).toHaveBeenCalledWith("deepseek");
    expect(removeProvider).toHaveBeenCalledWith("custom-provider");
  });

  it("returns discovered DeepSeek candidates without mutating or refreshing the catalog", async () => {
    const providers: RuntimeProviderSummary[] = [
      {
        id: "deepseek",
        displayName: "DeepSeek",
        origin: "builtin",
        configured: true,
        credentialConfigured: true,
        health: "unknown",
      },
    ];
    const models: RuntimeModelSummary[] = [
      {
        providerId: "deepseek",
        id: "deepseek-chat",
        displayName: "DeepSeek Chat",
        enabled: true,
      },
    ];
    const candidates = [
      { id: "deepseek-v4-flash", displayName: "DeepSeek V4 Flash" },
      { id: "deepseek-v4-pro", displayName: "DeepSeek V4 Pro" },
    ];
    const listProviders = vi.fn(async () => ({ providers }));
    const listModels = vi.fn(async () => ({ models }));
    const discoverProviderModels = vi.fn(async () => ({ models: candidates }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false,
        })),
        listProviders,
        listModels,
        discoverProviderModels,
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    const before = useAppStore.getState();
    const result = await before.discoverDeepSeekModels("write-only-discovery-secret");
    const after = useAppStore.getState();

    expect(result).toEqual(candidates);
    expect(discoverProviderModels).toHaveBeenCalledOnce();
    expect(discoverProviderModels).toHaveBeenCalledWith({
      kind: "deepseek",
      apiKey: "write-only-discovery-secret",
    });
    expect(listProviders).toHaveBeenCalledOnce();
    expect(listModels).toHaveBeenCalledOnce();
    expect(after.providers).toBe(before.providers);
    expect(after.models).toBe(before.models);
    expect(after.selectedModel).toBe(before.selectedModel);
  });

  it("ignores an older provider catalog response that finishes after a newer refresh", async () => {
    let resolveOldProviders: ((value: { providers: RuntimeProviderSummary[] }) => void) | undefined;
    let resolveOldModels: ((value: { models: RuntimeModelSummary[] }) => void) | undefined;
    const oldProviders = new Promise<{ providers: RuntimeProviderSummary[] }>((resolve) => {
      resolveOldProviders = resolve;
    });
    const oldModels = new Promise<{ models: RuntimeModelSummary[] }>((resolve) => {
      resolveOldModels = resolve;
    });
    const staleProvider: RuntimeProviderSummary = {
      id: "stale-provider",
      displayName: "Stale Provider",
      origin: "custom",
      configured: true,
      credentialConfigured: false,
      health: "unknown",
    };
    const freshProvider: RuntimeProviderSummary = {
      id: "fresh-provider",
      displayName: "Fresh Provider",
      origin: "custom",
      configured: true,
      credentialConfigured: false,
      health: "unknown",
    };
    const staleModel: RuntimeModelSummary = {
      providerId: "stale-provider",
      id: "stale-model",
      displayName: "Stale Model",
      enabled: true,
    };
    const freshModel: RuntimeModelSummary = {
      providerId: "fresh-provider",
      id: "fresh-model",
      displayName: "Fresh Model",
      enabled: true,
    };
    const listProviders = vi
      .fn()
      .mockImplementationOnce(() => oldProviders)
      .mockResolvedValueOnce({ providers: [freshProvider] });
    const listModels = vi
      .fn()
      .mockImplementationOnce(() => oldModels)
      .mockResolvedValueOnce({ models: [freshModel] });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(),
        listProviders,
        listModels,
        setModelEnabled: vi.fn(async () => freshModel),
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    const olderLoad = useAppStore.getState().loadProviderCatalog();
    await vi.waitFor(() => expect(listProviders).toHaveBeenCalledTimes(1));
    await useAppStore.getState().setModelEnabled({
      providerId: "fresh-provider",
      modelId: "fresh-model",
      enabled: true,
    });
    resolveOldProviders?.({ providers: [staleProvider] });
    resolveOldModels?.({ models: [staleModel] });
    await olderLoad;

    expect(useAppStore.getState().providers).toEqual([freshProvider]);
    expect(useAppStore.getState().models).toEqual([freshModel]);
    expect(useAppStore.getState().selectedModel).toEqual({
      providerId: "fresh-provider",
      modelId: "fresh-model",
    });
  });

  it("loads Skills lazily and applies a global enablement result in place", async () => {
    const enabled: RuntimeSkillSummary = {
      name: "demo",
      description: "Demo Skill",
      location: "C:/Users/demo/.ikaros/skills/demo/SKILL.md",
      enabled: true,
    };
    const disabled = { ...enabled, enabled: false };
    const diagnostics = [
      { entry: "broken", code: "missing_file", message: "SKILL.md is missing." },
    ];
    const listSkills = vi.fn(async () => ({ skills: [enabled], diagnostics }));
    const setSkillEnabled = vi.fn(async () => ({ skill: disabled }));
    installRuntimeBridge({
      runtime: {
        listSkills,
        setSkillEnabled,
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    expect(useAppStore.getState()).toMatchObject({
      skillCatalogStatus: "idle",
      skills: [],
      skillDiagnostics: [],
    });
    expect(listSkills).not.toHaveBeenCalled();

    await useAppStore.getState().loadSkillCatalog();
    expect(useAppStore.getState()).toMatchObject({
      skillCatalogStatus: "ready",
      skillCatalogError: null,
      skills: [enabled],
      skillDiagnostics: diagnostics,
    });

    await useAppStore.getState().setSkillEnabled({ name: "demo", enabled: false });
    expect(setSkillEnabled).toHaveBeenCalledWith({ name: "demo", enabled: false });
    expect(useAppStore.getState()).toMatchObject({
      skillCatalogStatus: "ready",
      skills: [disabled],
      skillDiagnostics: diagnostics,
    });
  });

  it("ignores stale Skill loads and preserves the catalog when a refresh fails", async () => {
    let resolveOld: ((value: { skills: RuntimeSkillSummary[]; diagnostics: [] }) => void) | undefined;
    const old = new Promise<{ skills: RuntimeSkillSummary[]; diagnostics: [] }>((resolve) => {
      resolveOld = resolve;
    });
    const enabled: RuntimeSkillSummary = {
      name: "demo",
      description: "Demo Skill",
      location: "C:/Users/demo/.ikaros/skills/demo/SKILL.md",
      enabled: true,
    };
    const disabled = { ...enabled, enabled: false };
    const listSkills = vi
      .fn()
      .mockImplementationOnce(() => old)
      .mockRejectedValueOnce(new Error("catalog unavailable"));
    const setSkillEnabled = vi.fn(async () => ({ skill: disabled }));
    installRuntimeBridge({
      runtime: {
        listSkills,
        setSkillEnabled,
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    const staleLoad = useAppStore.getState().loadSkillCatalog();
    await vi.waitFor(() => expect(listSkills).toHaveBeenCalledOnce());
    await useAppStore.getState().setSkillEnabled({ name: "demo", enabled: false });
    resolveOld?.({ skills: [enabled], diagnostics: [] });
    await staleLoad;
    expect(useAppStore.getState().skills).toEqual([disabled]);

    await expect(useAppStore.getState().loadSkillCatalog()).rejects.toThrow(
      "catalog unavailable",
    );
    expect(useAppStore.getState()).toMatchObject({
      skillCatalogStatus: "error",
      skillCatalogError: "catalog unavailable",
      skills: [disabled],
    });
  });

  it("starts an empty database from its catalog snapshot without replaying sequence zero", async () => {
    const replayEvents = vi.fn();
    const getThread = vi.fn();
    const listTurns = vi.fn();
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [], snapshotSeq: 0 })),
        getThread,
        listTurns,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents,
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();

    expect(useAppStore.getState()).toMatchObject({
      runtimeReady: true,
      runtimeSeq: 0,
      threads: [],
    });
    expect(replayEvents).not.toHaveBeenCalled();
    expect(getThread).not.toHaveBeenCalled();
    expect(listTurns).not.toHaveBeenCalled();
  });

  it("loads persisted process tool Items only after selecting their Thread", async () => {
    const thread = {
      id: "thread-runtime",
      title: "Tool replay",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const toolCall = {
      id: "tool-call-runtime",
      turnId: "turn-runtime",
      runId: "run-runtime",
      ordinal: 2,
      kind: "tool_call",
      role: "assistant",
      status: "running",
      content: "",
      data: {
        stepId: "step-runtime",
        callId: "provider-call-runtime",
        toolName: "process_run",
        arguments: { command: "Write-Output runtime-tool" }
      },
      createdAt,
      updatedAt: createdAt
    };
    const history = singleTurnHistoryPage(
      thread,
      [
        historyMessageItem(
          "user-runtime",
          "user",
          "/process.run Write-Output runtime-tool",
          "completed",
          1,
        ),
        {
          ...toolCall,
          kind: "tool_call",
          role: "assistant",
          status: "completed",
          data: { ...toolCall.data, outcome: "completed", durationMs: 14 },
        },
        {
          id: "tool-result-runtime",
          turnId: "turn-runtime",
          runId: "run-runtime",
          ordinal: 3,
          kind: "tool_result",
          role: "tool",
          status: "completed",
          content: "{}",
          data: {
            stepId: "step-runtime",
            callId: "provider-call-runtime",
            toolCallItemId: toolCall.id,
            toolName: "process_run",
            result: {
              ok: true,
              output: "runtime-tool\nsecond-line",
              stdout: "runtime-tool\n",
              stderr: "",
              exitCode: 0,
              timedOut: false,
              truncated: false,
            },
          },
          createdAt,
          updatedAt: createdAt,
        },
        historyMessageItem(
          "assistant-runtime",
          "assistant",
          "Command exited with code 0.",
          "completed",
          4,
        ),
      ],
      "completed",
      9,
    );
    const getThread = vi.fn(async () => ({ thread, snapshotSeq: 9 }));
    const listTurns = vi.fn(async () => history);
    const replayEvents = vi.fn(async (afterSeq: number) => ({
      events: [],
      latestSeq: 9,
      nextAfterSeq: afterSeq,
      hasMore: false,
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 9 })),
        getThread,
        listTurns,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents,
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    expect(getThread).not.toHaveBeenCalled();
    expect(listTurns).not.toHaveBeenCalled();
    expect(useAppStore.getState().threads[0]?.branches[0]?.turns).toEqual([]);
    expect(replayEvents).toHaveBeenCalledWith(9, 500);
    expect(replayEvents.mock.calls.some(([afterSeq]) => afterSeq === 0)).toBe(false);

    await useAppStore.getState().selectThread(thread.id);
    await useAppStore.getState().selectThread(thread.id);

    const state = useAppStore.getState();
    const projected = state.threads[0]?.branches[0]?.turns[0];
    expect(state.runtimeSeq).toBe(9);
    expect(state.runStatus).toBe("completed");
    expect(projected?.events).toMatchObject([
      { type: "message", role: "user" },
      {
        id: toolCall.id,
        type: "tool_call",
        toolName: "process.run",
        status: "success",
        durationMs: 14
      },
      {
        id: "tool-result-runtime",
        type: "tool_result",
        toolCallId: toolCall.id,
        status: "success",
        output: "runtime-tool\nsecond-line"
      },
      { type: "message", role: "assistant", status: "complete" }
    ]);
    expect(projected?.events.some((event) => event.type === "permission_request")).toBe(false);
    expect(getThread).toHaveBeenCalledOnce();
    expect(listTurns).toHaveBeenCalledOnce();
  });

  it("merges a live delta received during lazy hydration exactly once", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "Hydration race",
      defaultBranchId: "branch-runtime",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
      archivedAt: null,
    };
    const history = singleTurnHistoryPage(
      thread,
      [historyMessageItem("assistant-runtime", "assistant", "A", "streaming", 1)],
      "running",
      10,
    );
    const delta = runtimeEvent(11, "item.delta", "assistant-runtime", { delta: "B" });
    const listTurns = vi.fn(async () => {
      for (const listener of listeners) listener(delta);
      return history;
    });
    const replayEvents = vi
      .fn()
      .mockResolvedValueOnce({
        events: [],
        latestSeq: 10,
        nextAfterSeq: 10,
        hasMore: false,
      })
      .mockResolvedValue({
        events: [delta],
        latestSeq: 11,
        nextAfterSeq: 11,
        hasMore: false,
      });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 10 })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 10 })),
        listTurns,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents,
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        }),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    const assistant = useAppStore
      .getState()
      .threads[0]?.branches[0]?.turns[0]?.events[0];
    expect(assistant).toMatchObject({
      id: "assistant-runtime",
      content: "AB",
      status: "streaming",
    });
    expect(useAppStore.getState().runtimeSeq).toBe(11);
    expect(useAppStore.getState().runtimeThreadDetails[thread.id]).toMatchObject({
      status: "ready",
      snapshotSeq: 11,
      error: null,
    });
    expect(listTurns).toHaveBeenCalledOnce();
  });

  it("keeps the newer selection when an older Thread hydration finishes late", async () => {
    const firstThread = {
      id: "thread-first",
      title: "First",
      defaultBranchId: "branch-first",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
    };
    const secondThread = {
      id: "thread-second",
      title: "Second",
      defaultBranchId: "branch-second",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
    };
    let resolveFirst!: (page: RuntimeTurnListPage) => void;
    const firstPage = new Promise<RuntimeTurnListPage>((resolve) => {
      resolveFirst = resolve;
    });
    const getThread = vi.fn(async (threadId: string) => ({
      thread: threadId === firstThread.id ? firstThread : secondThread,
      snapshotSeq: 0,
    }));
    const listTurns = vi.fn(async (params: { threadId: string }) =>
      params.threadId === firstThread.id
        ? firstPage
        : {
            turns: [],
            nextCursor: null,
            hasMore: false,
            snapshotSeq: 0,
          },
    );
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({
          threads: [firstThread, secondThread],
          snapshotSeq: 0,
        })),
        getThread,
        listTurns,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(),
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    const selectingFirst = useAppStore.getState().selectThread(firstThread.id);
    await vi.waitFor(() => expect(listTurns).toHaveBeenCalledWith(
      expect.objectContaining({ threadId: firstThread.id }),
    ));
    await useAppStore.getState().selectThread(secondThread.id);
    resolveFirst(
      singleTurnHistoryPage(
        firstThread,
        [historyMessageItem("first-message", "assistant", "late", "completed", 1)],
        "completed",
        0,
        { turnId: "turn-first", runId: "run-first" },
      ),
    );
    await selectingFirst;

    expect(useAppStore.getState().selectedThreadId).toBe(secondThread.id);
    expect(useAppStore.getState().runtimeThreadDetails[firstThread.id]?.status).toBe("ready");
    expect(useAppStore.getState().runtimeThreadDetails[secondThread.id]?.status).toBe("ready");
  });

  it("keeps a canonical running Turn when turn.start is accepted but its ACK is lost", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "ACK loss",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const cancelRun = vi.fn(async (runId: string) => ({
      accepted: true,
      runId,
      status: "running" as const
    }));
    const startTurn = vi.fn(async (params: { clientRequestId?: string }) => {
      const events = [
        runtimeEvent(1, "item.completed", "user-runtime", {
          clientRequestId: params.clientRequestId,
          item: messageItem("user-runtime", "user", "accepted request", "completed")
        }),
        runtimeEvent(2, "run.state_changed", null, { status: "queued" }),
        runtimeEvent(3, "run.state_changed", null, { status: "running" })
      ];
      for (const event of events) {
        for (const listener of listeners) listener(event);
      }
      throw new Error("turn.start ACK lost");
    });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread] })),
        createThread: vi.fn(),
        startTurn,
        cancelRun,
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        })
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    useAppStore.getState().setDraft("accepted request");
    await useAppStore.getState().sendDraft();

    expect(startTurn).toHaveBeenCalledOnce();
    expect(useAppStore.getState().draft).toBe("");
    expect(useAppStore.getState().runtimeError).toBeNull();
    expect(useAppStore.getState().runStatus).toBe("running");
    expect(useAppStore.getState().pendingRuntimeSubmissions[thread.id]).toBeUndefined();

    useAppStore.getState().stopRun();
    await vi.waitFor(() => expect(cancelRun).toHaveBeenCalledWith("run-runtime"));
  });

  it("keeps a canonical completed Turn when turn.start settles before its ACK is lost", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "Settled ACK loss",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const startTurn = vi.fn(async (params: { clientRequestId?: string }) => {
      const events = [
        runtimeEvent(1, "item.completed", "user-runtime", {
          clientRequestId: params.clientRequestId,
          item: messageItem("user-runtime", "user", "settled request", "completed")
        }),
        runtimeEvent(2, "run.state_changed", null, { status: "queued" }),
        runtimeEvent(3, "run.state_changed", null, { status: "running" }),
        runtimeEvent(4, "item.started", "assistant-runtime", {
          item: messageItem("assistant-runtime", "assistant", "", "streaming")
        }),
        runtimeEvent(5, "item.completed", "assistant-runtime", {
          item: messageItem("assistant-runtime", "assistant", "finished", "completed")
        }),
        runtimeEvent(6, "run.settled", null, { status: "completed" })
      ];
      for (const event of events) {
        for (const listener of listeners) listener(event);
      }
      throw new Error("settled ACK lost");
    });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread] })),
        createThread: vi.fn(),
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        })
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    useAppStore.getState().setDraft("settled request");
    await useAppStore.getState().sendDraft();

    const turn = useAppStore.getState().threads[0]?.branches[0]?.turns[0];
    expect(startTurn).toHaveBeenCalledOnce();
    expect(useAppStore.getState().draft).toBe("");
    expect(useAppStore.getState().runtimeError).toBeNull();
    expect(useAppStore.getState().runStatus).toBe("completed");
    expect(turn).toMatchObject({
      runId: "run-runtime",
      status: "completed",
      events: [
        { role: "user", content: "settled request" },
        { role: "assistant", content: "finished", status: "complete" }
      ]
    });
  });

  it("starts exactly one Turn when thread.create ACK loss is seen live and in replay", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-created-after-ack-loss",
      title: "first message",
      defaultBranchId: "branch-created-after-ack-loss",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
      archivedAt: null,
    };
    let createdEvent: RuntimeJournalEvent | undefined;
    const createThread = vi.fn(async (params: RuntimeThreadCreateParams) => {
      const clientRequestId = params.clientRequestId;
      createdEvent = {
        seq: 1,
        schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
        type: "thread.created",
        threadId: thread.id,
        branchId: thread.defaultBranchId,
        turnId: null,
        runId: null,
        itemId: null,
        timestamp: createdAt,
        payload: {
          clientRequestId,
          thread,
          branch: {
            id: thread.defaultBranchId,
            threadId: thread.id,
            createdAt,
            isDefault: true
          }
        }
      };
      throw new Error("thread.create ACK lost");
    });
    const startTurn = vi.fn(async (_params: { clientRequestId?: string }) => ({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: "turn-created-after-ack-loss",
      runId: "run-created-after-ack-loss"
    }));
    const replayEvents = vi.fn(async () => {
      if (!createdEvent) {
        return { events: [], latestSeq: 0, nextAfterSeq: 0, hasMore: false };
      }
      for (const listener of listeners) listener(createdEvent);
      return { events: [createdEvent], latestSeq: 1, nextAfterSeq: 1, hasMore: false };
    });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread,
        startTurn,
        cancelRun: vi.fn(),
        replayEvents,
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        })
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().setDraft("first message");
    await useAppStore.getState().sendDraft();

    expect(createThread).toHaveBeenCalledOnce();
    expect(startTurn).toHaveBeenCalledOnce();
    expect(createThread.mock.calls[0]?.[0].clientRequestId).toEqual(expect.any(String));
    expect(startTurn.mock.calls[0]?.[0].clientRequestId).toEqual(expect.any(String));
    expect(startTurn.mock.calls[0]?.[0].clientRequestId).not.toBe(
      createThread.mock.calls[0]?.[0].clientRequestId
    );
    expect(useAppStore.getState().threads).toHaveLength(1);
    expect(useAppStore.getState().selectedThreadId).toBe(thread.id);
    expect(useAppStore.getState().pendingRuntimeNewThread).toBeNull();
    expect(useAppStore.getState().draft).toBe("");
    expect(useAppStore.getState().runtimeError).toBeNull();
  });

  it("keeps retrying an idempotent thread.create after the initial recovery window", async () => {
    vi.useFakeTimers();
    const thread = {
      id: "thread-after-long-outage",
      title: "long outage",
      defaultBranchId: "branch-after-long-outage",
      createdAt,
      updatedAt: createdAt
    };
    let createAttempts = 0;
    const createThread = vi.fn(async (params: RuntimeThreadCreateParams) => {
      const clientRequestId = params.clientRequestId;
      createAttempts += 1;
      if (createAttempts <= 5) throw new Error("Runtime connection unavailable");
      const event: RuntimeJournalEvent = {
        seq: 1,
        schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
        type: "thread.created",
        threadId: thread.id,
        branchId: thread.defaultBranchId,
        turnId: null,
        runId: null,
        itemId: null,
        timestamp: createdAt,
        payload: {
          clientRequestId,
          thread,
          branch: {
            id: thread.defaultBranchId,
            threadId: thread.id,
            createdAt,
            isDefault: true
          }
        }
      };
      return { thread, event };
    });
    const startTurn = vi.fn(async (_params: { clientRequestId?: string }) => ({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: "turn-after-long-outage",
      runId: "run-after-long-outage"
    }));
    let replayCalls = 0;
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread,
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => {
          replayCalls += 1;
          if (replayCalls === 1) {
            return { events: [], latestSeq: 0, nextAfterSeq: 0, hasMore: false };
          }
          throw new Error("Runtime replay unavailable");
        }),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().setDraft("long outage");
    const sending = useAppStore.getState().sendDraft();
    await vi.advanceTimersByTimeAsync(5_000);
    await sending;

    expect(createThread).toHaveBeenCalledTimes(6);
    expect(
      new Set(createThread.mock.calls.map((call) => call[0].clientRequestId)).size,
    ).toBe(1);
    expect(startTurn).toHaveBeenCalledOnce();
    expect(useAppStore.getState().selectedThreadId).toBe(thread.id);
    expect(useAppStore.getState().draft).toBe("");
  });

  it("keeps retrying an idempotent turn.start after the initial recovery window", async () => {
    vi.useFakeTimers();
    const thread = {
      id: "thread-runtime",
      title: "long turn outage",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    let startAttempts = 0;
    const startTurn = vi.fn(async (params: { clientRequestId?: string }) => {
      startAttempts += 1;
      if (startAttempts <= 5) throw new Error("Runtime connection unavailable");
      return {
        threadId: thread.id,
        branchId: thread.defaultBranchId,
        turnId: "turn-after-long-outage",
        runId: "run-after-long-outage",
        requestId: params.clientRequestId
      };
    });
    let replayCalls = 0;
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread] })),
        createThread: vi.fn(),
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => {
          replayCalls += 1;
          if (replayCalls === 1) {
            return { events: [], latestSeq: 0, nextAfterSeq: 0, hasMore: false };
          }
          throw new Error("Runtime replay unavailable");
        }),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    useAppStore.getState().setDraft("long turn outage");
    const sending = useAppStore.getState().sendDraft();
    await vi.advanceTimersByTimeAsync(5_000);
    await sending;

    expect(startTurn).toHaveBeenCalledTimes(6);
    expect(
      new Set(startTurn.mock.calls.map((call) => call[0].clientRequestId)).size
    ).toBe(1);
    expect(useAppStore.getState().runStatus).toBe("queued");
    expect(useAppStore.getState().draft).toBe("");
    expect(useAppStore.getState().runtimeError).toBeNull();
  });

  it("keeps thread.create ambiguous after an empty recovery replay", async () => {
    vi.useFakeTimers();
    const thread = {
      id: "thread-empty-create-replay",
      title: "empty create replay",
      defaultBranchId: "branch-empty-create-replay",
      createdAt,
      updatedAt: createdAt
    };
    let createAttempts = 0;
    const createThread = vi.fn(async (params: RuntimeThreadCreateParams) => {
      const clientRequestId = params.clientRequestId;
      createAttempts += 1;
      if (createAttempts === 1) {
        throw new Error("socket closed after thread.create was sent");
      }
      const event: RuntimeJournalEvent = {
        seq: 1,
        schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
        type: "thread.created",
        threadId: thread.id,
        branchId: thread.defaultBranchId,
        turnId: null,
        runId: null,
        itemId: null,
        timestamp: createdAt,
        payload: {
          clientRequestId,
          thread,
          branch: {
            id: thread.defaultBranchId,
            threadId: thread.id,
            createdAt,
            isDefault: true
          }
        }
      };
      return { thread, event };
    });
    const startTurn = vi.fn(async () => ({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: "turn-empty-create-replay",
      runId: "run-empty-create-replay"
    }));
    const replayEvents = vi.fn(async () => ({
      events: [],
      latestSeq: 0,
      nextAfterSeq: 0,
      hasMore: false
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread,
        startTurn,
        cancelRun: vi.fn(),
        replayEvents,
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().setDraft("empty create replay");
    const sending = useAppStore.getState().sendDraft();
    await vi.advanceTimersByTimeAsync(100);
    await sending;

    expect(createThread).toHaveBeenCalledTimes(2);
    expect(
      new Set(createThread.mock.calls.map((call) => call[0].clientRequestId)).size,
    ).toBe(1);
    expect(replayEvents).toHaveBeenCalledOnce();
    expect(startTurn).toHaveBeenCalledOnce();
    expect(useAppStore.getState().selectedThreadId).toBe(thread.id);
    expect(useAppStore.getState().draft).toBe("");
    expect(useAppStore.getState().runtimeError).toBeNull();
  });

  it("keeps turn.start ambiguous after an empty recovery replay", async () => {
    vi.useFakeTimers();
    const thread = {
      id: "thread-empty-turn-replay",
      title: "empty turn replay",
      defaultBranchId: "branch-empty-turn-replay",
      createdAt,
      updatedAt: createdAt
    };
    let startAttempts = 0;
    const startTurn = vi.fn(async (params: { clientRequestId?: string }) => {
      startAttempts += 1;
      if (startAttempts === 1) {
        throw new Error("socket closed after turn.start was sent");
      }
      return {
        threadId: thread.id,
        branchId: thread.defaultBranchId,
        turnId: "turn-empty-turn-replay",
        runId: "run-empty-turn-replay",
        requestId: params.clientRequestId
      };
    });
    const replayEvents = vi.fn(async () => ({
      events: [],
      latestSeq: 0,
      nextAfterSeq: 0,
      hasMore: false
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread] })),
        createThread: vi.fn(),
        startTurn,
        cancelRun: vi.fn(),
        replayEvents,
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    useAppStore.getState().setDraft("empty turn replay");
    const sending = useAppStore.getState().sendDraft();
    await vi.advanceTimersByTimeAsync(100);
    await sending;

    expect(startTurn).toHaveBeenCalledTimes(2);
    expect(new Set(startTurn.mock.calls.map((call) => call[0].clientRequestId)).size).toBe(1);
    expect(replayEvents).toHaveBeenCalledOnce();
    expect(useAppStore.getState().runStatus).toBe("queued");
    expect(useAppStore.getState().draft).toBe("");
    expect(useAppStore.getState().runtimeError).toBeNull();
  });

  it("retries initialization after a transient Runtime failure", async () => {
    const listThreads = vi
      .fn()
      .mockRejectedValueOnce(new Error("temporary disconnect"))
      .mockResolvedValueOnce({ threads: [] });
    const api = {
      runtime: {
        listThreads,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    expect(useAppStore.getState().runtimeReady).toBe(false);
    expect(useAppStore.getState().runtimeError).toBe("temporary disconnect");

    await useAppStore.getState().initializeRuntime();
    expect(listThreads).toHaveBeenCalledTimes(2);
    expect(useAppStore.getState().runtimeReady).toBe(true);
    expect(useAppStore.getState().runtimeError).toBeNull();
  });

  it("does not switch back when a turn.start ACK arrives after navigation", async () => {
    const firstThread = {
      id: "thread-runtime",
      title: "Slow ACK",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const secondThread = {
      id: "thread-other",
      title: "Current chat",
      defaultBranchId: "branch-other",
      createdAt,
      updatedAt: createdAt
    };
    let resolveStart: ((result: {
      threadId: string;
      branchId: string;
      turnId: string;
      runId: string;
    }) => void) | undefined;
    const startTurn = vi.fn(
      () =>
        new Promise<{
          threadId: string;
          branchId: string;
          turnId: string;
          runId: string;
        }>((resolve) => {
          resolveStart = resolve;
        })
    );
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [firstThread, secondThread] })),
        createThread: vi.fn(),
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(firstThread.id);

    useAppStore.getState().setDraft("slow request");
    const sending = useAppStore.getState().sendDraft();
    await vi.waitFor(() => expect(startTurn).toHaveBeenCalledOnce());
    await useAppStore.getState().selectThread(secondThread.id);
    resolveStart?.({
      threadId: firstThread.id,
      branchId: firstThread.defaultBranchId,
      turnId: "turn-runtime",
      runId: "run-runtime"
    });
    await sending;

    expect(useAppStore.getState().selectedThreadId).toBe(secondThread.id);
    expect(useAppStore.getState().runStatus).toBe("idle");
    expect(useAppStore.getState().activeRun).toBeNull();
  });

  it("keeps a pending submission queued across A to B to A navigation and rejects a duplicate", async () => {
    const firstThread = {
      id: "thread-runtime",
      title: "Pending chat",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const secondThread = {
      id: "thread-other",
      title: "Other chat",
      defaultBranchId: "branch-other",
      createdAt,
      updatedAt: createdAt
    };
    let resolveStart: ((result: {
      threadId: string;
      branchId: string;
      turnId: string;
      runId: string;
    }) => void) | undefined;
    const startTurn = vi.fn(
      () =>
        new Promise<{
          threadId: string;
          branchId: string;
          turnId: string;
          runId: string;
        }>((resolve) => {
          resolveStart = resolve;
        })
    );
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [firstThread, secondThread] })),
        createThread: vi.fn(),
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(firstThread.id);

    useAppStore.getState().setDraft("first submission");
    const sending = useAppStore.getState().sendDraft();
    await vi.waitFor(() => expect(startTurn).toHaveBeenCalledOnce());
    await useAppStore.getState().selectThread(secondThread.id);
    await useAppStore.getState().selectThread(firstThread.id);

    expect(useAppStore.getState().runStatus).toBe("queued");
    useAppStore.getState().setDraft("duplicate submission");
    await useAppStore.getState().sendDraft();
    expect(startTurn).toHaveBeenCalledOnce();
    expect(useAppStore.getState().draft).toBe("duplicate submission");

    resolveStart?.({
      threadId: firstThread.id,
      branchId: firstThread.defaultBranchId,
      turnId: "turn-runtime",
      runId: "run-runtime"
    });
    await sending;

    expect(useAppStore.getState().selectedThreadId).toBe(firstThread.id);
    expect(useAppStore.getState().runStatus).toBe("queued");
    expect(useAppStore.getState().activeRun).toBeNull();
  });

  it("does not bind a pending submission to an unrelated Run event from the same Thread", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "Correlated pending chat",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    let resolveStart: ((result: {
      threadId: string;
      branchId: string;
      turnId: string;
      runId: string;
    }) => void) | undefined;
    const startTurn = vi.fn(
      () =>
        new Promise<{
          threadId: string;
          branchId: string;
          turnId: string;
          runId: string;
        }>((resolve) => {
          resolveStart = resolve;
        })
    );
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread] })),
        createThread: vi.fn(),
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        })
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    useAppStore.getState().setDraft("new submission");
    const sending = useAppStore.getState().sendDraft();
    await vi.waitFor(() => expect(startTurn).toHaveBeenCalledOnce());
    const unrelated = runtimeEvent(
      1,
      "run.state_changed",
      null,
      { status: "running" },
      { turnId: "turn-old", runId: "run-old" }
    );
    for (const listener of listeners) {
      listener(unrelated);
    }

    expect(useAppStore.getState().pendingRuntimeSubmissions[thread.id]?.runId).toBeUndefined();
    expect(useAppStore.getState().pendingRuntimeSubmissions[thread.id]?.prompt).toBe(
      "new submission"
    );
    resolveStart?.({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: "turn-new",
      runId: "run-new"
    });
    await sending;

    expect(useAppStore.getState().pendingRuntimeSubmissions[thread.id]).toMatchObject({
      runId: "run-new",
      turnId: "turn-new"
    });
  });

  it("keeps a newly created Thread in the background when thread.create ACK arrives late", async () => {
    const createdThread = {
      id: "thread-runtime",
      title: "Background chat",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const currentThread = {
      id: "thread-other",
      title: "Current chat",
      defaultBranchId: "branch-other",
      createdAt,
      updatedAt: createdAt
    };
    const createdEvent: RuntimeJournalEvent = {
      seq: 1,
      schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
      type: "thread.created",
      threadId: createdThread.id,
      branchId: createdThread.defaultBranchId,
      turnId: null,
      runId: null,
      itemId: null,
      timestamp: createdAt,
      payload: {
        thread: createdThread,
        branch: {
          id: createdThread.defaultBranchId,
          threadId: createdThread.id,
          createdAt,
          isDefault: true
        }
      }
    };
    let resolveCreate: ((result: {
      thread: typeof createdThread;
      event: RuntimeJournalEvent;
    }) => void) | undefined;
    const createThread = vi.fn(
      () =>
        new Promise<{
          thread: typeof createdThread;
          event: RuntimeJournalEvent;
        }>((resolve) => {
          resolveCreate = resolve;
        })
    );
    const startTurn = vi.fn(async () => ({
      threadId: createdThread.id,
      branchId: createdThread.defaultBranchId,
      turnId: "turn-runtime",
      runId: "run-runtime"
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [currentThread] })),
        createThread,
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().setDraft("create in background");
    const sending = useAppStore.getState().sendDraft();
    await vi.waitFor(() => expect(createThread).toHaveBeenCalledOnce());
    await useAppStore.getState().selectThread(currentThread.id);
    useAppStore.getState().setDraft("keep this draft");
    resolveCreate?.({ thread: createdThread, event: createdEvent });
    await sending;

    expect(startTurn).toHaveBeenCalledWith(
      expect.objectContaining({
        threadId: createdThread.id,
        branchId: createdThread.defaultBranchId,
        content: "create in background",
        providerId: "test-provider",
        modelId: "test-model",
        clientRequestId: expect.any(String)
      })
    );
    expect(useAppStore.getState().threads.some((thread) => thread.id === createdThread.id)).toBe(
      true
    );
    expect(useAppStore.getState().selectedThreadId).toBe(currentThread.id);
    expect(useAppStore.getState().draft).toBe("keep this draft");
    expect(useAppStore.getState().runStatus).toBe("idle");
    expect(useAppStore.getState().activeRun).toBeNull();
  });

  it("keeps the first message queued after thread.created while turn.start is pending", async () => {
    const thread = {
      id: "thread-runtime",
      title: "First message",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const createdEvent: RuntimeJournalEvent = {
      seq: 1,
      schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
      type: "thread.created",
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: null,
      runId: null,
      itemId: null,
      timestamp: createdAt,
      payload: {
        thread,
        branch: {
          id: thread.defaultBranchId,
          threadId: thread.id,
          createdAt,
          isDefault: true
        }
      }
    };
    let resolveStart: ((result: {
      threadId: string;
      branchId: string;
      turnId: string;
      runId: string;
    }) => void) | undefined;
    const startTurn = vi.fn(
      () =>
        new Promise<{
          threadId: string;
          branchId: string;
          turnId: string;
          runId: string;
        }>((resolve) => {
          resolveStart = resolve;
        })
    );
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread: vi.fn(async () => ({ thread, event: createdEvent })),
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().setDraft("first message");
    const sending = useAppStore.getState().sendDraft();
    await vi.waitFor(() => expect(startTurn).toHaveBeenCalledOnce());

    expect(useAppStore.getState().selectedThreadId).toBe(thread.id);
    expect(useAppStore.getState().runStatus).toBe("queued");
    useAppStore.getState().setDraft("duplicate first message");
    await useAppStore.getState().sendDraft();
    expect(startTurn).toHaveBeenCalledOnce();
    expect(useAppStore.getState().draft).toBe("duplicate first message");

    resolveStart?.({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: "turn-runtime",
      runId: "run-runtime"
    });
    await sending;
  });

  it("does not reclaim a fresh New Chat when an older thread.create ACK arrives", async () => {
    const thread = {
      id: "thread-runtime",
      title: "Older new chat",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const createdEvent: RuntimeJournalEvent = {
      seq: 1,
      schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
      type: "thread.created",
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: null,
      runId: null,
      itemId: null,
      timestamp: createdAt,
      payload: {
        thread,
        branch: {
          id: thread.defaultBranchId,
          threadId: thread.id,
          createdAt,
          isDefault: true
        }
      }
    };
    let resolveCreate: ((result: {
      thread: typeof thread;
      event: RuntimeJournalEvent;
    }) => void) | undefined;
    const createThread = vi.fn(
      () =>
        new Promise<{
          thread: typeof thread;
          event: RuntimeJournalEvent;
        }>((resolve) => {
          resolveCreate = resolve;
        })
    );
    const startTurn = vi.fn(async () => ({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: "turn-runtime",
      runId: "run-runtime"
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread,
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().setDraft("older request");
    const sending = useAppStore.getState().sendDraft();
    await vi.waitFor(() => expect(createThread).toHaveBeenCalledOnce());
    useAppStore.getState().newChat();
    useAppStore.getState().setDraft("fresh new chat draft");
    resolveCreate?.({ thread, event: createdEvent });
    await sending;

    expect(startTurn).toHaveBeenCalledOnce();
    expect(useAppStore.getState().selectedThreadId).toBeNull();
    expect(useAppStore.getState().draft).toBe("fresh new chat draft");
    expect(useAppStore.getState().runStatus).toBe("idle");
    expect(useAppStore.getState().activeRun).toBeNull();
  });

  it("cleans a background thread.create failure without overwriting a fresh New Chat", async () => {
    let rejectCreate: ((error: Error) => void) | undefined;
    const createThread = vi.fn(
      () =>
        new Promise<never>((_resolve, reject) => {
          rejectCreate = reject;
        })
    );
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread,
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().setDraft("older request");
    const sending = useAppStore.getState().sendDraft();
    await vi.waitFor(() => expect(createThread).toHaveBeenCalledOnce());
    useAppStore.getState().newChat();
    useAppStore.getState().setDraft("fresh draft");
    rejectCreate?.(definitiveRpcError("background create failed"));
    await sending;

    expect(useAppStore.getState().selectedThreadId).toBeNull();
    expect(useAppStore.getState().draft).toBe("fresh draft");
    expect(useAppStore.getState().runStatus).toBe("idle");
    expect(useAppStore.getState().activeRun).toBeNull();
    expect(useAppStore.getState().pendingRuntimeNewThread).toBeNull();
    expect(useAppStore.getState().runtimeError).toBeNull();
    expect(useAppStore.getState().runtimeIssue).toBeNull();
  });

  it("does not let a late turn.start rejection overwrite a newer active Thread", async () => {
    const firstThread = {
      id: "thread-runtime",
      title: "Rejected chat",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const secondThread = {
      id: "thread-other",
      title: "Active chat",
      defaultBranchId: "branch-other",
      createdAt,
      updatedAt: createdAt
    };
    let rejectFirst: ((error: Error) => void) | undefined;
    const startTurn = vi.fn((params: { threadId: string; branchId: string }) => {
      if (params.threadId === firstThread.id) {
        return new Promise<never>((_resolve, reject) => {
          rejectFirst = reject;
        });
      }
      return Promise.resolve({
        threadId: secondThread.id,
        branchId: secondThread.defaultBranchId,
        turnId: "turn-other",
        runId: "run-other"
      });
    });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [firstThread, secondThread] })),
        createThread: vi.fn(),
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(firstThread.id);

    useAppStore.getState().setDraft("request A");
    const firstSending = useAppStore.getState().sendDraft();
    await vi.waitFor(() => expect(startTurn).toHaveBeenCalledTimes(1));
    await useAppStore.getState().selectThread(secondThread.id);
    useAppStore.getState().setDraft("request B");
    await useAppStore.getState().sendDraft();
    useAppStore.getState().setDraft("unsent B draft");
    const activeRun = useAppStore.getState().activeRun;

    rejectFirst?.(definitiveRpcError("A start failed"));
    await firstSending;

    expect(useAppStore.getState().selectedThreadId).toBe(secondThread.id);
    expect(useAppStore.getState().draft).toBe("unsent B draft");
    expect(useAppStore.getState().runStatus).toBe("queued");
    expect(useAppStore.getState().activeRun).toEqual(activeRun);
    expect(useAppStore.getState().activeRun).toMatchObject({
      threadId: secondThread.id,
      runId: "run-other"
    });
    expect(useAppStore.getState().runtimeError).toBe("A start failed");
    expect(useAppStore.getState().pendingRuntimeSubmissions[firstThread.id]).toBeUndefined();
  });

  it("clears a failed thread.create submission so the first message can retry", async () => {
    const thread = {
      id: "thread-runtime",
      title: "Retried creation",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const createdEvent: RuntimeJournalEvent = {
      seq: 1,
      schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
      type: "thread.created",
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: null,
      runId: null,
      itemId: null,
      timestamp: createdAt,
      payload: {
        thread,
        branch: {
          id: thread.defaultBranchId,
          threadId: thread.id,
          createdAt,
          isDefault: true
        }
      }
    };
    const createThread = vi
      .fn()
      .mockRejectedValueOnce(definitiveRpcError("create failed"))
      .mockResolvedValueOnce({ thread, event: createdEvent });
    const startTurn = vi.fn(async () => ({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: "turn-runtime",
      runId: "run-runtime"
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread,
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().setDraft("retry creation");
    await useAppStore.getState().sendDraft();
    expect(useAppStore.getState().runtimeError).toBe("create failed");
    expect(useAppStore.getState().runStatus).toBe("failed");
    expect(useAppStore.getState().draft).toBe("retry creation");
    expect(useAppStore.getState().pendingRuntimeNewThread).toBeNull();
    expect(useAppStore.getState().runtimeIssue).toEqual({
      kind: "send",
      message: "create failed",
      threadId: null,
      prompt: "retry creation"
    });

    await useAppStore.getState().retryRuntimeIssue();

    expect(createThread).toHaveBeenCalledTimes(2);
    expect(startTurn).toHaveBeenCalledOnce();
    expect(useAppStore.getState().selectedThreadId).toBe(thread.id);
    expect(useAppStore.getState().runStatus).toBe("queued");
  });

  it("clears a first-message turn.start failure so the created Thread can retry", async () => {
    const thread = {
      id: "thread-runtime",
      title: "Retried turn",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const createdEvent: RuntimeJournalEvent = {
      seq: 1,
      schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
      type: "thread.created",
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      turnId: null,
      runId: null,
      itemId: null,
      timestamp: createdAt,
      payload: {
        thread,
        branch: {
          id: thread.defaultBranchId,
          threadId: thread.id,
          createdAt,
          isDefault: true
        }
      }
    };
    const startTurn = vi
      .fn()
      .mockRejectedValueOnce(definitiveRpcError("start failed"))
      .mockResolvedValueOnce({
        threadId: thread.id,
        branchId: thread.defaultBranchId,
        turnId: "turn-runtime",
        runId: "run-runtime"
      });
    const createThread = vi.fn(async () => ({ thread, event: createdEvent }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [] })),
        createThread,
        startTurn,
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    useAppStore.getState().setDraft("retry turn");
    await useAppStore.getState().sendDraft();
    expect(useAppStore.getState().runtimeError).toBe("start failed");
    expect(useAppStore.getState().runStatus).toBe("failed");
    expect(useAppStore.getState().draft).toBe("retry turn");
    expect(useAppStore.getState().pendingRuntimeSubmissions[thread.id]).toBeUndefined();

    await useAppStore.getState().sendDraft();

    expect(createThread).toHaveBeenCalledOnce();
    expect(startTurn).toHaveBeenCalledTimes(2);
    expect(useAppStore.getState().runStatus).toBe("queued");
    expect(useAppStore.getState().activeRun).toMatchObject({ runId: "run-runtime" });
  });

  it("retries a failed paginated gap replay and merges interleaved events exactly once", async () => {
    vi.useFakeTimers();
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "Gap recovery",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const started = runtimeEvent(1, "item.started", "assistant-runtime", {
      item: messageItem("assistant-runtime", "assistant", "", "streaming")
    });
    const firstDelta = runtimeEvent(2, "item.delta", "assistant-runtime", { delta: "A" });
    const interleavedDelta = runtimeEvent(3, "item.delta", "assistant-runtime", {
      delta: "B"
    });
    const running = runtimeEvent(4, "run.state_changed", null, { status: "running" });
    const replayEvents = vi
      .fn()
      .mockRejectedValueOnce(new Error("transient replay disconnect"))
      .mockResolvedValueOnce({
        events: [started],
        latestSeq: 4,
        nextAfterSeq: 1,
        hasMore: true
      })
      .mockImplementationOnce(async () => {
        for (const listener of listeners) {
          listener(interleavedDelta);
        }
        return {
          events: [firstDelta, interleavedDelta],
          latestSeq: 4,
          nextAfterSeq: 3,
          hasMore: false
        };
      });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread] })),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents,
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        })
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    for (const listener of listeners) {
      listener(running);
      listener(running);
    }

    await Promise.resolve();
    await Promise.resolve();
    expect(replayEvents).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(1_000);

    expect(useAppStore.getState().runtimeSeq).toBe(4);
    const assistant = useAppStore
      .getState()
      .threads[0]?.branches[0]?.turns[0]?.events[0];
    expect(assistant).toMatchObject({
      id: "assistant-runtime",
      content: "AB",
      status: "streaming"
    });
    expect(useAppStore.getState().runtimeError).toBeNull();
    expect(replayEvents).toHaveBeenCalledTimes(3);
  });

  it("recovers a completed Run when Stop races with events after a history snapshot", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "Retained journal prefix",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const history = singleTurnHistoryPage(
      thread,
      [
        historyMessageItem("user-runtime", "user", "hello", "completed", 1),
        historyMessageItem("assistant-runtime", "assistant", "Hi", "streaming", 2),
      ],
      "running",
      230,
    );
    const completedEvents = [
      runtimeEvent(231, "item.completed", "assistant-runtime", {
        item: messageItem("assistant-runtime", "assistant", "Hi", "completed")
      }),
      runtimeEvent(232, "run.settled", null, { status: "completed" })
    ];
    const cancelRun = vi.fn(async () => ({
      accepted: false,
      runId: "run-runtime",
      status: "completed" as const
    }));
    const replayEvents = vi
      .fn()
      .mockResolvedValueOnce({
        events: [],
        latestSeq: 230,
        nextAfterSeq: 230,
        hasMore: false,
      })
      .mockResolvedValueOnce({
        events: [],
        latestSeq: 230,
        nextAfterSeq: 230,
        hasMore: false,
      })
      .mockResolvedValueOnce({
        events: completedEvents,
        latestSeq: 232,
        nextAfterSeq: 232,
        hasMore: false
      });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 230 })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 230 })),
        listTurns: vi.fn(async () => history),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun,
        replayEvents,
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        })
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    expect(useAppStore.getState().runtimeSeq).toBe(230);
    expect(useAppStore.getState().runStatus).toBe("running");
    expect(
      useAppStore.getState().threads[0]?.branches[0]?.turns[0]?.events
    ).toMatchObject([
      { role: "user", content: "hello", status: "complete" },
      { role: "assistant", content: "Hi", status: "streaming" }
    ]);

    useAppStore.getState().stopRun();
    await vi.waitFor(() => expect(useAppStore.getState().runStatus).toBe("completed"));

    expect(cancelRun).toHaveBeenCalledOnce();
    expect(cancelRun).toHaveBeenCalledWith("run-runtime");
    expect(replayEvents).toHaveBeenNthCalledWith(3, 230, 500);
    expect(useAppStore.getState().runtimeSeq).toBe(232);
    expect(
      useAppStore.getState().threads[0]?.branches[0]?.turns[0]
    ).toMatchObject({
      runId: "run-runtime",
      status: "completed",
      events: [
        { role: "user", content: "hello", status: "complete" },
        { role: "assistant", content: "Hi", status: "complete" }
      ]
    });
  });

  it("stops the selected Runtime Run once and waits for canonical cancellation", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "Canonical stop",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const history = singleTurnHistoryPage(
      thread,
      [historyMessageItem("user-runtime", "user", "stop me", "completed", 1)],
      "running",
      2,
    );
    const cancelRun = vi.fn(async () => ({
      accepted: true,
      runId: "run-runtime",
      status: "running" as const
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 2 })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 2 })),
        listTurns: vi.fn(async () => history),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun,
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 2,
          nextAfterSeq: 2,
          hasMore: false
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        })
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    useAppStore.getState().setSettingsOpen(true);
    useAppStore.getState().setSettingsOpen(false);
    useAppStore.getState().newChat();
    await useAppStore.getState().selectThread(thread.id);
    expect(cancelRun).not.toHaveBeenCalled();

    useAppStore.getState().stopRun();
    useAppStore.getState().stopRun();
    expect(cancelRun).toHaveBeenCalledOnce();
    expect(cancelRun).toHaveBeenCalledWith("run-runtime");
    expect(useAppStore.getState().runStatus).toBe("running");

    const settled = runtimeEvent(3, "run.settled", null, { status: "cancelled" });
    for (const listener of listeners) {
      listener(settled);
    }
    expect(useAppStore.getState().runStatus).toBe("interrupted");
    expect(
      useAppStore.getState().threads[0]?.branches[0]?.turns[0]
    ).toMatchObject({ runId: "run-runtime", status: "interrupted" });
  });

  it("paginates canonical replay when cancellation loses to completion", async () => {
    const thread = {
      id: "thread-runtime",
      title: "Completion wins",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const history = singleTurnHistoryPage(
      thread,
      [historyMessageItem("user-runtime", "user", "finish first", "completed", 1)],
      "running",
      2,
    );
    const completedItem = runtimeEvent(3, "item.completed", "assistant-runtime", {
      item: messageItem("assistant-runtime", "assistant", "finished", "completed")
    });
    const settled = runtimeEvent(4, "run.settled", null, { status: "completed" });
    const replayEvents = vi
      .fn()
      .mockResolvedValueOnce({
        events: [],
        latestSeq: 2,
        nextAfterSeq: 2,
        hasMore: false
      })
      .mockResolvedValueOnce({
        events: [],
        latestSeq: 2,
        nextAfterSeq: 2,
        hasMore: false,
      })
      .mockResolvedValueOnce({
        events: [completedItem],
        latestSeq: 4,
        nextAfterSeq: 3,
        hasMore: true
      })
      .mockResolvedValueOnce({
        events: [settled],
        latestSeq: 4,
        nextAfterSeq: 4,
        hasMore: false
      });
    const cancelRun = vi.fn(async () => ({
      accepted: false,
      runId: "run-runtime",
      status: "completed" as const
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 2 })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 2 })),
        listTurns: vi.fn(async () => history),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun,
        replayEvents,
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    useAppStore.getState().stopRun();
    await vi.waitFor(() => expect(useAppStore.getState().runStatus).toBe("completed"));

    expect(cancelRun).toHaveBeenCalledOnce();
    expect(replayEvents).toHaveBeenCalledTimes(4);
    expect(replayEvents).toHaveBeenNthCalledWith(3, 2, 500);
    expect(replayEvents).toHaveBeenNthCalledWith(4, 3, 500);
    expect(
      useAppStore.getState().threads[0]?.branches[0]?.turns[0]
    ).toMatchObject({
      runId: "run-runtime",
      status: "completed",
      events: [{ role: "user" }, { role: "assistant", status: "complete" }]
    });
  });

  it("targets Stop at the selected thread when two Runs are active", async () => {
    const firstThread = {
      id: "thread-runtime",
      title: "Running chat",
      defaultBranchId: "branch-runtime",
      createdAt,
      updatedAt: createdAt
    };
    const secondThread = {
      id: "thread-other",
      title: "Queued chat",
      defaultBranchId: "branch-other",
      createdAt,
      updatedAt: createdAt
    };
    const secondIdentity = {
      threadId: secondThread.id,
      branchId: secondThread.defaultBranchId,
      turnId: "turn-other",
      runId: "run-other"
    };
    const secondHistory = singleTurnHistoryPage(
      secondThread,
      [historyMessageItem("user-other", "user", "second", "completed", 1)],
      "queued",
      4,
      { turnId: secondIdentity.turnId, runId: secondIdentity.runId },
    );
    const cancelRun = vi.fn(async (runId: string) => ({
      accepted: true,
      runId,
      status: "queued" as const
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({
          threads: [firstThread, secondThread],
          snapshotSeq: 4,
        })),
        getThread: vi.fn(async () => ({ thread: secondThread, snapshotSeq: 4 })),
        listTurns: vi.fn(async () => secondHistory),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun,
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 4,
          nextAfterSeq: 4,
          hasMore: false
        })),
        onEvent: vi.fn(() => () => undefined)
      },
      preferences: {},
      windowControls: {}
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(secondThread.id);

    useAppStore.getState().stopRun();

    expect(cancelRun).toHaveBeenCalledOnce();
    expect(cancelRun).toHaveBeenCalledWith("run-other");
    expect(cancelRun).not.toHaveBeenCalledWith("run-runtime");
  });

  it("keeps Thread recency from a newer Turn page when metadata uses an older snapshot", async () => {
    const historyUpdatedAt = "2026-08-11T12:01:00.000Z";
    const thread = {
      id: "thread-runtime",
      title: "Split snapshots",
      defaultBranchId: "branch-runtime",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
    };
    const history = singleTurnHistoryPage(thread, [], "completed", 11);
    history.turns[0] = {
      ...history.turns[0]!,
      updatedAt: historyUpdatedAt,
    };
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 10 })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 10 })),
        listTurns: vi.fn(async () => history),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: 11,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    expect(useAppStore.getState().threads[0]?.updatedAt).toBe(historyUpdatedAt);
    expect(useAppStore.getState().runtimeThreadDetails[thread.id]?.snapshotSeq).toBe(11);
  });

  it("uses each Turn page watermark when deltas arrive during later pagination", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "Page watermarks",
      defaultBranchId: "branch-runtime",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
    };
    const newerPage = singleTurnHistoryPage(
      thread,
      [historyMessageItem("assistant-newer", "assistant", "A", "streaming", 1)],
      "running",
      10,
      { turnId: "turn-newer", runId: "run-newer" },
    );
    newerPage.turns[0] = { ...newerPage.turns[0]!, ordinal: 2 };
    newerPage.nextCursor = "older-page";
    newerPage.hasMore = true;
    const olderPage = singleTurnHistoryPage(
      thread,
      [historyMessageItem("assistant-older", "assistant", "AB", "streaming", 1)],
      "running",
      12,
      { turnId: "turn-older", runId: "run-older" },
    );
    const newerDelta = runtimeEvent(11, "item.delta", "assistant-newer", { delta: "B" }, {
      turnId: "turn-newer",
      runId: "run-newer",
    });
    const olderDelta = runtimeEvent(12, "item.delta", "assistant-older", { delta: "B" }, {
      turnId: "turn-older",
      runId: "run-older",
    });
    let deltasEmitted = false;
    const listTurns = vi
      .fn<IkarosRuntimeApi["listTurns"]>()
      .mockResolvedValueOnce(newerPage)
      .mockImplementationOnce(async () => {
        deltasEmitted = true;
        for (const listener of listeners) {
          listener(newerDelta);
          listener(olderDelta);
        }
        return olderPage;
      });
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 10 })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 10 })),
        listTurns,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: deltasEmitted && afterSeq < 12 ? [newerDelta, olderDelta] : [],
          latestSeq: deltasEmitted ? 12 : 10,
          nextAfterSeq: deltasEmitted ? 12 : afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        }),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);
    await useAppStore.getState().loadOlderRuntimeTurns(thread.id);

    const turns = useAppStore.getState().threads[0]?.branches[0]?.turns ?? [];
    const newer = turns
      .find((turn) => turn.id === "turn-newer")
      ?.events.find((event) => event.id === "assistant-newer");
    const older = turns
      .find((turn) => turn.id === "turn-older")
      ?.events.find((event) => event.id === "assistant-older");
    expect(newer).toMatchObject({ content: "AB", status: "streaming" });
    expect(older).toMatchObject({ content: "AB", status: "streaming" });
    expect(useAppStore.getState().runtimeSeq).toBe(12);
  });

  it("keeps a live settled Run completed when a later history page has a newer watermark", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "Settled during pagination",
      defaultBranchId: "branch-runtime",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
    };
    const newerPage = singleTurnHistoryPage(thread, [], "running", 10, {
      turnId: "turn-newer",
      runId: "run-newer",
    });
    newerPage.turns[0] = { ...newerPage.turns[0]!, ordinal: 2 };
    newerPage.nextCursor = "older-page";
    newerPage.hasMore = true;
    const olderPage = singleTurnHistoryPage(thread, [], "completed", 12, {
      turnId: "turn-older",
      runId: "run-older",
    });
    const settled = runtimeEvent(11, "run.settled", null, { status: "completed" }, {
      turnId: "turn-newer",
      runId: "run-newer",
    });
    let settledEmitted = false;
    const listTurns = vi
      .fn<IkarosRuntimeApi["listTurns"]>()
      .mockResolvedValueOnce(newerPage)
      .mockImplementationOnce(async () => {
        settledEmitted = true;
        for (const listener of listeners) {
          listener(settled);
        }
        return olderPage;
      });
    const cancelRun = vi.fn();
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 10 })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 10 })),
        listTurns,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun,
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: settledEmitted && afterSeq < 11 ? [settled] : [],
          latestSeq: settledEmitted ? 12 : 10,
          nextAfterSeq: settledEmitted ? 12 : afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        }),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);
    await useAppStore.getState().loadOlderRuntimeTurns(thread.id);

    expect(useAppStore.getState().runtimeThreadActivity[thread.id]?.["run-newer"]).toMatchObject({
      seq: 11,
      status: "completed",
      turnOrdinal: 2,
    });
    expect(useAppStore.getState().runStatus).toBe("completed");
    useAppStore.getState().stopRun();
    expect(cancelRun).not.toHaveBeenCalled();
  });

  it("stops the newest active Run then falls back to an older active Run in the same Thread", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "Concurrent runs",
      defaultBranchId: "branch-runtime",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
    };
    const cancelRun = vi.fn(async (runId: string) => ({
      accepted: true,
      runId,
      status: "running" as const,
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 0 })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 0 })),
        listTurns: vi.fn(async () => ({
          turns: [],
          nextCursor: null,
          hasMore: false,
          snapshotSeq: 0,
        })),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun,
        replayEvents: vi.fn(async () => ({
          events: [],
          latestSeq: 0,
          nextAfterSeq: 0,
          hasMore: false,
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        }),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);

    for (const listener of listeners) {
      listener(runtimeEvent(1, "run.state_changed", null, { status: "running" }, {
        turnId: "turn-a",
        runId: "run-a",
      }));
      listener(runtimeEvent(2, "run.state_changed", null, { status: "queued" }, {
        turnId: "turn-b",
        runId: "run-b",
      }));
    }
    expect(useAppStore.getState().runStatus).toBe("queued");
    useAppStore.getState().stopRun();
    expect(cancelRun).toHaveBeenNthCalledWith(1, "run-b");

    for (const listener of listeners) {
      listener(runtimeEvent(3, "run.settled", null, { status: "cancelled" }, {
        turnId: "turn-b",
        runId: "run-b",
      }));
    }
    expect(useAppStore.getState().runStatus).toBe("running");
    useAppStore.getState().stopRun();
    expect(cancelRun).toHaveBeenNthCalledWith(2, "run-a");
  });

  it("orders restored active Runs by Turn ordinal instead of page read watermarks", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const thread = {
      id: "thread-runtime",
      title: "Restored concurrent runs",
      defaultBranchId: "branch-runtime",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
    };
    const newerPage = singleTurnHistoryPage(thread, [], "queued", 10, {
      turnId: "turn-newer",
      runId: "run-newer",
    });
    newerPage.turns[0] = { ...newerPage.turns[0]!, ordinal: 2 };
    newerPage.nextCursor = "older-page";
    newerPage.hasMore = true;
    const olderPage = singleTurnHistoryPage(thread, [], "running", 12, {
      turnId: "turn-older",
      runId: "run-older",
    });
    const cancelRun = vi.fn(async (runId: string) => ({
      accepted: true,
      runId,
      status: "running" as const,
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread], snapshotSeq: 12 })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 10 })),
        listTurns: vi
          .fn<IkarosRuntimeApi["listTurns"]>()
          .mockResolvedValueOnce(newerPage)
          .mockResolvedValueOnce(olderPage),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun,
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: 12,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        }),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);
    await useAppStore.getState().loadOlderRuntimeTurns(thread.id);

    expect(useAppStore.getState().runStatus).toBe("queued");
    useAppStore.getState().stopRun();
    expect(cancelRun).toHaveBeenNthCalledWith(1, "run-newer");

    for (const listener of listeners) {
      listener(runtimeEvent(13, "run.settled", null, { status: "cancelled" }, {
        turnId: "turn-newer",
        runId: "run-newer",
      }));
    }
    expect(useAppStore.getState().runStatus).toBe("running");
    useAppStore.getState().stopRun();
    expect(cancelRun).toHaveBeenNthCalledWith(2, "run-older");
  });

  it("recovers one catalog stub when a post-snapshot event references an omitted Thread", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const omittedThread = {
      id: "thread-omitted",
      title: "Recovered catalog entry",
      defaultBranchId: "branch-omitted",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
      archivedAt: null,
    };
    let resolveMetadata!: (value: { thread: RuntimeThreadSummary; snapshotSeq: number }) => void;
    const metadata = new Promise<{ thread: RuntimeThreadSummary; snapshotSeq: number }>(
      (resolve) => {
        resolveMetadata = resolve;
      },
    );
    const getThread = vi.fn(async () => metadata);
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [], snapshotSeq: 10 })),
        getThread,
        listTurns: vi.fn(),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: 10,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        }),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    const started = {
      ...runtimeEvent(11, "item.started", "assistant-omitted", {
        item: messageItem("assistant-omitted", "assistant", "", "streaming"),
      }, {
        threadId: omittedThread.id,
        branchId: omittedThread.defaultBranchId,
        turnId: "turn-omitted",
        runId: "run-omitted",
      }),
      timestamp: "2026-08-11T12:01:00.000Z",
    };
    const running = {
      ...runtimeEvent(12, "run.state_changed", null, { status: "running" }, {
        threadId: omittedThread.id,
        branchId: omittedThread.defaultBranchId,
        turnId: "turn-omitted",
        runId: "run-omitted",
      }),
      timestamp: "2026-08-11T12:02:00.000Z",
    };
    for (const listener of listeners) {
      listener(started);
      listener(running);
    }
    expect(getThread).toHaveBeenCalledOnce();
    resolveMetadata({ thread: omittedThread, snapshotSeq: 10 });

    await vi.waitFor(() => {
      expect(useAppStore.getState().threads.some((thread) => thread.id === omittedThread.id)).toBe(
        true,
      );
    });
    const recovered = useAppStore
      .getState()
      .threads.find((thread) => thread.id === omittedThread.id);
    expect(recovered).toMatchObject({
      title: omittedThread.title,
      updatedAt: running.timestamp,
    });
    expect(recovered?.branches[0]?.turns).toEqual([]);
    expect(useAppStore.getState().runtimeThreadDetails[omittedThread.id]).toMatchObject({
      status: "idle",
      snapshotSeq: 12,
      error: null,
    });
    expect(useAppStore.getState().runtimeThreadActivity[omittedThread.id]?.["run-omitted"]).toMatchObject({
      status: "running",
      seq: 12,
    });
    expect(getThread).toHaveBeenCalledOnce();
  });

  it("does not resurrect an omitted Thread when a newer archive event wins recovery", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const activeThread: RuntimeThreadSummary = {
      id: "thread-archive-recovery-race",
      title: "Active metadata snapshot",
      defaultBranchId: "branch-archive-recovery-race",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
      archivedAt: null,
    };
    const archivedThread: RuntimeThreadSummary = {
      ...activeThread,
      updatedAt: "2026-08-11T12:02:00.000Z",
      archivedAt: "2026-08-11T12:02:00.000Z",
    };
    let resolveMetadata!: (value: { thread: RuntimeThreadSummary; snapshotSeq: number }) => void;
    const metadata = new Promise<{ thread: RuntimeThreadSummary; snapshotSeq: number }>(
      (resolve) => {
        resolveMetadata = resolve;
      },
    );
    const getThread = vi.fn(async () => metadata);
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [], snapshotSeq: 10 })),
        getThread,
        listTurns: vi.fn(),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: 10,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        }),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi;
    installRuntimeBridge(api);
    vi.resetModules();
    const { useAppStore } = await import("./store");
    await useAppStore.getState().initializeRuntime();

    for (const listener of listeners) {
      listener(
        runtimeEvent(11, "item.started", "assistant-recovery-race", {
          item: messageItem("assistant-recovery-race", "assistant", "", "streaming"),
        }, {
          threadId: activeThread.id,
          branchId: activeThread.defaultBranchId,
          turnId: "turn-recovery-race",
          runId: "run-recovery-race",
        }),
      );
    }
    expect(getThread).toHaveBeenCalledOnce();

    for (const listener of listeners) {
      listener(
        runtimeEvent(12, "thread.archived", null, { thread: archivedThread }, {
          threadId: activeThread.id,
          branchId: activeThread.defaultBranchId,
          turnId: null,
          runId: null,
        }),
      );
    }
    resolveMetadata({ thread: activeThread, snapshotSeq: 10 });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(useAppStore.getState().threads.some((thread) => thread.id === activeThread.id)).toBe(
      false,
    );
    expect(useAppStore.getState().archivedThreads).toEqual([archivedThread]);
    expect(useAppStore.getState().runtimeSeq).toBe(12);
  });

  it("increments the active catalog page by page without advancing the Journal cursor", async () => {
    const listeners = new Set<(event: RuntimeJournalEvent) => void>();
    const newest: RuntimeThreadSummary = {
      id: "thread-page-newest",
      title: "Newest",
      defaultBranchId: "branch-page-newest",
      workspace: null,
      createdAt,
      updatedAt: "2026-08-11T12:03:00.000Z",
      archivedAt: null,
    };
    const older: RuntimeThreadSummary = {
      ...newest,
      id: "thread-page-older",
      title: "Older",
      defaultBranchId: "branch-page-older",
      updatedAt: "2026-08-11T12:02:00.000Z",
    };
    const oldest: RuntimeThreadSummary = {
      ...newest,
      id: "thread-page-oldest",
      title: "Oldest",
      defaultBranchId: "branch-page-oldest",
      updatedAt: "2026-08-11T12:01:00.000Z",
    };
    let resolveSecond!: (page: RuntimeThreadListPage) => void;
    const secondPage = new Promise<RuntimeThreadListPage>((resolve) => {
      resolveSecond = resolve;
    });
    const listThreads = vi
      .fn<IkarosRuntimeApi["listThreads"]>()
      .mockResolvedValueOnce({
        threads: [newest],
        nextCursor: "catalog-page-2",
        hasMore: true,
        snapshotSeq: 10,
      })
      .mockImplementationOnce(() => secondPage)
      .mockResolvedValueOnce({
        threads: [oldest],
        nextCursor: null,
        hasMore: false,
        snapshotSeq: 13,
      });
    installRuntimeBridge({
      runtime: {
        listThreads,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: afterSeq,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        }),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    expect(listThreads).toHaveBeenNthCalledWith(1, { limit: 25 });
    expect(useAppStore.getState()).toMatchObject({
      runtimeSeq: 10,
      threadCatalogHasMore: true,
      threadCatalogMoreStatus: "idle",
    });

    const continuation = useAppStore.getState().loadMoreThreads();
    await vi.waitFor(() => expect(listThreads).toHaveBeenCalledTimes(2));
    const live = {
      ...runtimeEvent(11, "item.started", "item-page-live", {
        item: messageItem("item-page-live", "assistant", "", "streaming"),
      }, {
        threadId: newest.id,
        branchId: newest.defaultBranchId,
        turnId: "turn-page-live",
        runId: "run-page-live",
      }),
      timestamp: "2026-08-11T12:05:00.000Z",
    };
    for (const listener of listeners) listener(live);
    resolveSecond({
      threads: [older],
      nextCursor: "catalog-page-3",
      hasMore: true,
      snapshotSeq: 12,
    });
    await continuation;

    expect(useAppStore.getState().threads.map((thread) => thread.id)).toEqual([
      newest.id,
      older.id,
    ]);
    expect(useAppStore.getState()).toMatchObject({
      runtimeSeq: 11,
      threadCatalogHasMore: true,
      threadCatalogMoreStatus: "idle",
    });
    expect(listThreads).toHaveBeenNthCalledWith(2, {
      cursor: "catalog-page-2",
      limit: 25,
    });

    await useAppStore.getState().loadAllThreadsForSearch();
    expect(listThreads).toHaveBeenNthCalledWith(3, {
      cursor: "catalog-page-3",
      limit: 25,
    });
    expect(useAppStore.getState().threads.map((thread) => thread.id)).toEqual([
      newest.id,
      older.id,
      oldest.id,
    ]);
    expect(useAppStore.getState()).toMatchObject({
      runtimeSeq: 11,
      threadCatalogHasMore: false,
      searchCatalogStatus: "ready",
      searchCatalogError: null,
    });
  });

  it("preserves an active catalog page and cursor when continuation fails, then retries", async () => {
    const first: RuntimeThreadSummary = {
      id: "thread-catalog-first",
      title: "First",
      defaultBranchId: "branch-catalog-first",
      workspace: null,
      createdAt,
      updatedAt: "2026-08-11T12:02:00.000Z",
      archivedAt: null,
    };
    const second: RuntimeThreadSummary = {
      ...first,
      id: "thread-catalog-second",
      title: "Second",
      defaultBranchId: "branch-catalog-second",
      updatedAt: "2026-08-11T12:01:00.000Z",
    };
    const listThreads = vi
      .fn<IkarosRuntimeApi["listThreads"]>()
      .mockResolvedValueOnce({
        threads: [first],
        nextCursor: "retry-cursor",
        hasMore: true,
        snapshotSeq: 5,
      })
      .mockRejectedValueOnce(new Error("continuation unavailable"))
      .mockResolvedValueOnce({
        threads: [second],
        nextCursor: null,
        hasMore: false,
        snapshotSeq: 6,
      });
    installRuntimeBridge({
      runtime: {
        listThreads,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: afterSeq,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().loadMoreThreads();
    expect(useAppStore.getState().threads.map((thread) => thread.id)).toEqual([first.id]);
    expect(useAppStore.getState()).toMatchObject({
      runtimeSeq: 5,
      runtimeError: null,
      threadCatalogNextCursor: "retry-cursor",
      threadCatalogHasMore: true,
      threadCatalogMoreStatus: "error",
      threadCatalogMoreError: "continuation unavailable",
    });

    await useAppStore.getState().loadMoreThreads();
    expect(useAppStore.getState().threads.map((thread) => thread.id)).toEqual([
      first.id,
      second.id,
    ]);
    expect(useAppStore.getState()).toMatchObject({
      runtimeSeq: 5,
      threadCatalogNextCursor: null,
      threadCatalogHasMore: false,
      threadCatalogMoreStatus: "idle",
      threadCatalogMoreError: null,
    });
  });

  it("keeps loaded Turns and their cursor when an older-page read fails", async () => {
    const thread: RuntimeThreadSummary = {
      id: "thread-turn-retry",
      title: "Turn retry",
      defaultBranchId: "branch-turn-retry",
      workspace: null,
      createdAt,
      updatedAt: createdAt,
      archivedAt: null,
    };
    const newer = singleTurnHistoryPage(thread, [], "completed", 10, {
      turnId: "turn-newer-retry",
      runId: "run-newer-retry",
    });
    newer.turns[0] = { ...newer.turns[0]!, ordinal: 2 };
    newer.nextCursor = "older-retry";
    newer.hasMore = true;
    const older = singleTurnHistoryPage(thread, [], "completed", 12, {
      turnId: "turn-older-retry",
      runId: "run-older-retry",
    });
    const listTurns = vi
      .fn<IkarosRuntimeApi["listTurns"]>()
      .mockResolvedValueOnce(newer)
      .mockRejectedValueOnce(new Error("older page unavailable"))
      .mockResolvedValueOnce(older);
    installRuntimeBridge({
      runtime: {
        listThreads: vi.fn(async () => ({
          threads: [thread],
          nextCursor: null,
          hasMore: false,
          snapshotSeq: 10,
        })),
        getThread: vi.fn(async () => ({ thread, snapshotSeq: 10 })),
        listTurns,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: afterSeq,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().selectThread(thread.id);
    await useAppStore.getState().loadOlderRuntimeTurns(thread.id);
    expect(
      useAppStore.getState().threads[0]?.branches[0]?.turns.map((turn) => turn.id),
    ).toEqual(["turn-newer-retry"]);
    expect(useAppStore.getState().runtimeThreadDetails[thread.id]).toMatchObject({
      nextCursor: "older-retry",
      hasMore: true,
      olderStatus: "error",
      olderError: "older page unavailable",
    });
    expect(useAppStore.getState().runtimeError).toBeNull();

    await useAppStore.getState().loadOlderRuntimeTurns(thread.id);
    expect(
      useAppStore.getState().threads[0]?.branches[0]?.turns.map((turn) => turn.id),
    ).toEqual(["turn-older-retry", "turn-newer-retry"]);
    expect(useAppStore.getState().runtimeThreadDetails[thread.id]).toMatchObject({
      nextCursor: null,
      hasMore: false,
      olderStatus: "idle",
      olderError: null,
    });
    expect(useAppStore.getState().runtimeSeq).toBe(10);
  });

  it("paginates archived Threads and preserves the first page on continuation failure", async () => {
    const archivedAt = "2026-08-11T12:02:00.000Z";
    const first: RuntimeThreadSummary = {
      id: "archived-page-first",
      title: "Archived first",
      defaultBranchId: "archived-branch-first",
      workspace: null,
      createdAt,
      updatedAt: archivedAt,
      archivedAt,
    };
    const second: RuntimeThreadSummary = {
      ...first,
      id: "archived-page-second",
      title: "Archived second",
      defaultBranchId: "archived-branch-second",
      updatedAt: "2026-08-11T12:01:00.000Z",
      archivedAt: "2026-08-11T12:01:00.000Z",
    };
    const listThreads = vi
      .fn<IkarosRuntimeApi["listThreads"]>()
      .mockResolvedValueOnce({
        threads: [],
        nextCursor: null,
        hasMore: false,
        snapshotSeq: 7,
      })
      .mockResolvedValueOnce({
        threads: [first],
        nextCursor: "archived-page-2",
        hasMore: true,
        snapshotSeq: 7,
      })
      .mockRejectedValueOnce(new Error("archived continuation unavailable"))
      .mockResolvedValueOnce({
        threads: [second],
        nextCursor: null,
        hasMore: false,
        snapshotSeq: 8,
      });
    installRuntimeBridge({
      runtime: {
        listThreads,
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: afterSeq,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().loadArchivedThreads();
    expect(listThreads).toHaveBeenNthCalledWith(2, { archived: true, limit: 25 });
    expect(useAppStore.getState()).toMatchObject({
      archivedCatalogStatus: "ready",
      archivedCatalogHasMore: true,
      archivedCatalogMoreStatus: "idle",
    });

    await useAppStore.getState().loadMoreArchivedThreads();
    expect(useAppStore.getState().archivedThreads).toEqual([first]);
    expect(useAppStore.getState()).toMatchObject({
      runtimeSeq: 7,
      runtimeError: null,
      archivedCatalogNextCursor: "archived-page-2",
      archivedCatalogHasMore: true,
      archivedCatalogMoreStatus: "error",
      archivedCatalogMoreError: "archived continuation unavailable",
    });

    await useAppStore.getState().loadMoreArchivedThreads();
    expect(listThreads).toHaveBeenNthCalledWith(4, {
      archived: true,
      cursor: "archived-page-2",
      limit: 25,
    });
    expect(useAppStore.getState().archivedThreads).toEqual([first, second]);
    expect(useAppStore.getState()).toMatchObject({
      runtimeSeq: 7,
      archivedCatalogHasMore: false,
      archivedCatalogMoreStatus: "idle",
      archivedCatalogMoreError: null,
    });
  });

  it("refreshes provider and model catalogs on reconnect without installing a stale retry", async () => {
    const provider = (displayName: string): RuntimeProviderSummary => ({
      id: "provider-reconnect",
      displayName,
      origin: "custom",
      configured: true,
      credentialConfigured: true,
      health: "ready",
    });
    const model = (displayName: string): RuntimeModelSummary => ({
      providerId: "provider-reconnect",
      id: "model-reconnect",
      displayName,
      enabled: true,
    });
    let resolveStaleProviders!: (value: { providers: RuntimeProviderSummary[] }) => void;
    let resolveStaleModels!: (value: { models: RuntimeModelSummary[] }) => void;
    const staleProviders = new Promise<{ providers: RuntimeProviderSummary[] }>((resolve) => {
      resolveStaleProviders = resolve;
    });
    const staleModels = new Promise<{ models: RuntimeModelSummary[] }>((resolve) => {
      resolveStaleModels = resolve;
    });
    const listProviders = vi
      .fn()
      .mockResolvedValueOnce({ providers: [provider("Initial Provider")] })
      .mockResolvedValueOnce({ providers: [provider("Refreshed Provider")] })
      .mockImplementationOnce(() => staleProviders)
      .mockResolvedValueOnce({ providers: [provider("Newest Provider")] });
    const listModels = vi
      .fn()
      .mockResolvedValueOnce({ models: [model("Initial Model")] })
      .mockResolvedValueOnce({ models: [model("Refreshed Model")] })
      .mockImplementationOnce(() => staleModels)
      .mockResolvedValueOnce({ models: [model("Newest Model")] });
    installRuntimeBridge({
      runtime: {
        listThreads: vi.fn(async () => ({
          threads: [],
          nextCursor: null,
          hasMore: false,
          snapshotSeq: 0,
        })),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async (afterSeq: number) => ({
          events: [],
          latestSeq: afterSeq,
          nextAfterSeq: afterSeq,
          hasMore: false,
        })),
        listProviders,
        listModels,
        onEvent: vi.fn(() => () => undefined),
      },
      preferences: {},
      windowControls: {},
    } as unknown as IkarosDesktopApi);
    vi.resetModules();
    const { useAppStore } = await import("./store");

    await useAppStore.getState().initializeRuntime();
    await useAppStore.getState().retryRuntimeConnection();
    expect(useAppStore.getState()).toMatchObject({
      runtimeConnectionStatus: "connected",
      providerCatalogStatus: "ready",
      providers: [{ displayName: "Refreshed Provider" }],
      models: [{ displayName: "Refreshed Model" }],
    });

    const staleRetry = useAppStore.getState().retryRuntimeConnection();
    await vi.waitFor(() => {
      expect(listProviders).toHaveBeenCalledTimes(3);
      expect(listModels).toHaveBeenCalledTimes(3);
    });
    await useAppStore.getState().loadProviderCatalog();
    resolveStaleProviders({ providers: [provider("Stale Provider")] });
    resolveStaleModels({ models: [model("Stale Model")] });
    await staleRetry;

    expect(useAppStore.getState()).toMatchObject({
      runtimeConnectionStatus: "connected",
      providerCatalogStatus: "ready",
      providers: [{ displayName: "Newest Provider" }],
      models: [{ displayName: "Newest Model" }],
    });
  });
});
