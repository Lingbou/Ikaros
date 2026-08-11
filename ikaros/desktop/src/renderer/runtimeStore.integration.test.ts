import { afterEach, describe, expect, it, vi } from "vitest";

import type { IkarosDesktopApi } from "../shared/platform";
import type {
  IkarosRuntimeApi,
  RuntimeInvocationResult,
  RuntimeJournalEvent,
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
  const bridged = {
    ...(api as object),
    runtime: {
      listThreads: () => bridgeInvocation(() => runtime.listThreads()),
      createThread: (title: string | null, clientRequestId?: string) =>
        bridgeInvocation(() => runtime.createThread(title, clientRequestId)),
      startTurn: (params: Parameters<IkarosRuntimeApi["startTurn"]>[0]) =>
        bridgeInvocation(() => runtime.startTurn(params)),
      cancelRun: (runId: string) => bridgeInvocation(() => runtime.cancelRun(runId)),
      replayEvents: (afterSeq: number, limit?: number) =>
        bridgeInvocation(() => runtime.replayEvents(afterSeq, limit)),
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
  type: string,
  itemId: string | null,
  payload: Record<string, unknown>,
  identity: Partial<
    Pick<RuntimeJournalEvent, "threadId" | "branchId" | "turnId" | "runId">
  > = {}
): RuntimeJournalEvent {
  return {
    seq,
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

afterEach(() => {
  vi.useRealTimers();
  vi.resetModules();
  Reflect.deleteProperty(window, "ikarosDesktop");
});

describe("Runtime-backed renderer store", () => {
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
        providerId: "scripted",
        modelId: "scripted-v1",
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

  it("replays persisted process tool Items into the production store projection", async () => {
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
    const events = [
      runtimeEvent(1, "item.completed", "user-runtime", {
        item: messageItem(
          "user-runtime",
          "user",
          "/process.run Write-Output runtime-tool",
          "completed"
        )
      }),
      runtimeEvent(2, "run.state_changed", null, { status: "queued" }),
      runtimeEvent(3, "run.state_changed", null, { status: "running" }),
      runtimeEvent(4, "item.started", toolCall.id, { item: toolCall }),
      runtimeEvent(5, "item.completed", toolCall.id, {
        item: {
          ...toolCall,
          status: "completed",
          data: { ...toolCall.data, outcome: "completed", durationMs: 14 }
        }
      }),
      runtimeEvent(6, "item.completed", "tool-result-runtime", {
        item: {
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
              truncated: false
            }
          },
          createdAt,
          updatedAt: createdAt
        }
      }),
      runtimeEvent(7, "item.started", "assistant-runtime", {
        item: messageItem("assistant-runtime", "assistant", "", "streaming")
      }),
      runtimeEvent(8, "item.completed", "assistant-runtime", {
        item: messageItem(
          "assistant-runtime",
          "assistant",
          "Command exited with code 0.",
          "completed"
        )
      }),
      runtimeEvent(9, "run.settled", null, { status: "completed" })
    ];
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread] })),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun: vi.fn(),
        replayEvents: vi.fn(async () => ({
          events,
          latestSeq: 9,
          nextAfterSeq: 9,
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
      createdAt,
      updatedAt: createdAt
    };
    let createdEvent: RuntimeJournalEvent | undefined;
    const createThread = vi.fn(async (_title: string | null, clientRequestId?: string) => {
      createdEvent = {
        seq: 1,
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
    let replayCalls = 0;
    const replayEvents = vi.fn(async () => {
      replayCalls += 1;
      if (replayCalls === 1 || !createdEvent) {
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
    expect(createThread.mock.calls[0]?.[1]).toEqual(expect.any(String));
    expect(startTurn.mock.calls[0]?.[0].clientRequestId).toEqual(expect.any(String));
    expect(startTurn.mock.calls[0]?.[0].clientRequestId).not.toBe(
      createThread.mock.calls[0]?.[1]
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
    const createThread = vi.fn(async (_title: string | null, clientRequestId?: string) => {
      createAttempts += 1;
      if (createAttempts <= 5) throw new Error("Runtime connection unavailable");
      const event: RuntimeJournalEvent = {
        seq: 1,
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
    expect(new Set(createThread.mock.calls.map((call) => call[1])).size).toBe(1);
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
    const createThread = vi.fn(async (_title: string | null, clientRequestId?: string) => {
      createAttempts += 1;
      if (createAttempts === 1) {
        throw new Error("socket closed after thread.create was sent");
      }
      const event: RuntimeJournalEvent = {
        seq: 1,
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
    expect(new Set(createThread.mock.calls.map((call) => call[1])).size).toBe(1);
    expect(replayEvents).toHaveBeenCalledTimes(2);
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
    expect(replayEvents).toHaveBeenCalledTimes(2);
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
        providerId: "scripted",
        modelId: "scripted-v1",
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
    expect(useAppStore.getState().runtimeError).toBe("background create failed");
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

    await useAppStore.getState().sendDraft();

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
      .mockResolvedValueOnce({
        events: [],
        latestSeq: 0,
        nextAfterSeq: 0,
        hasMore: false
      })
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

    for (const listener of listeners) {
      listener(running);
      listener(running);
    }

    await Promise.resolve();
    await Promise.resolve();
    expect(replayEvents).toHaveBeenCalledTimes(2);
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
    expect(replayEvents).toHaveBeenCalledTimes(4);
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
    const initialEvents = [
      runtimeEvent(1, "item.completed", "user-runtime", {
        item: messageItem("user-runtime", "user", "stop me", "completed")
      }),
      runtimeEvent(2, "run.state_changed", null, { status: "running" })
    ];
    const cancelRun = vi.fn(async () => ({
      accepted: true,
      runId: "run-runtime",
      status: "running" as const
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [thread] })),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun,
        replayEvents: vi.fn(async () => ({
          events: initialEvents,
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
    const initialEvents = [
      runtimeEvent(1, "item.completed", "user-runtime", {
        item: messageItem("user-runtime", "user", "finish first", "completed")
      }),
      runtimeEvent(2, "run.state_changed", null, { status: "running" })
    ];
    const completedItem = runtimeEvent(3, "item.completed", "assistant-runtime", {
      item: messageItem("assistant-runtime", "assistant", "finished", "completed")
    });
    const settled = runtimeEvent(4, "run.settled", null, { status: "completed" });
    const replayEvents = vi
      .fn()
      .mockResolvedValueOnce({
        events: initialEvents,
        latestSeq: 2,
        nextAfterSeq: 2,
        hasMore: false
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
        listThreads: vi.fn(async () => ({ threads: [thread] })),
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
    expect(replayEvents).toHaveBeenCalledTimes(3);
    expect(replayEvents).toHaveBeenNthCalledWith(2, 2, 500);
    expect(replayEvents).toHaveBeenNthCalledWith(3, 3, 500);
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
    const initialEvents = [
      runtimeEvent(1, "item.completed", "user-runtime", {
        item: messageItem("user-runtime", "user", "first", "completed")
      }),
      runtimeEvent(2, "run.state_changed", null, { status: "running" }),
      runtimeEvent(
        3,
        "item.completed",
        "user-other",
        { item: messageItem("user-other", "user", "second", "completed") },
        secondIdentity
      ),
      runtimeEvent(4, "run.state_changed", null, { status: "queued" }, secondIdentity)
    ];
    const cancelRun = vi.fn(async (runId: string) => ({
      accepted: true,
      runId,
      status: "queued" as const
    }));
    const api = {
      runtime: {
        listThreads: vi.fn(async () => ({ threads: [firstThread, secondThread] })),
        createThread: vi.fn(),
        startTurn: vi.fn(),
        cancelRun,
        replayEvents: vi.fn(async () => ({
          events: initialEvents,
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
});
