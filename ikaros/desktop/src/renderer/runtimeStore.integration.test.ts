import { afterEach, describe, expect, it, vi } from "vitest";

import type { IkarosDesktopApi } from "../shared/platform";
import type { RuntimeJournalEvent } from "../shared/runtime";

const createdAt = "2026-08-11T12:00:00.000Z";

function runtimeEvent(
  seq: number,
  type: string,
  itemId: string | null,
  payload: Record<string, unknown>
): RuntimeJournalEvent {
  return {
    seq,
    type,
    threadId: "thread-runtime",
    branchId: "branch-runtime",
    turnId: "turn-runtime",
    runId: "run-runtime",
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
    const startTurn = vi.fn(async () => ({
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
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });
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
    expect(startTurn).toHaveBeenCalledWith({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      content: "hello runtime",
      providerId: "scripted",
      modelId: "scripted-v1"
    });
    expect(state.runStatus).toBe("completed");
    expect(state.activeRun).toBeNull();
    expect(turn).toMatchObject({
      status: "completed",
      events: [
        { role: "user", content: "hello runtime" },
        { role: "assistant", content: "hello back", status: "complete" }
      ]
    });
  });
});
