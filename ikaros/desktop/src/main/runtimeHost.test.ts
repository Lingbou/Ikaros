// @vitest-environment node

import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it, vi } from "vitest";

import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  RUNTIME_PROTOCOL_VERSION,
  type RuntimeJournalEvent,
  type RuntimeMemoryCreateResult,
  type RuntimeMemoryGetResult,
  type RuntimeMemoryListPage,
  type RuntimeMemoryMutationResult,
  type RuntimeReplayResult,
  type RuntimeThreadGetResult,
  type RuntimeTurnListPage,
  type RuntimeTurnHistory,
  type RuntimeUsageReadResult
} from "../shared/runtime";
import {
  parseRuntimeThreadGetResult,
  parseRuntimeThreadMutationResult,
  parseRuntimeJournalEvent,
  parseRuntimeJsonRpcResponse,
  parseRuntimeReplayResult,
  parseRuntimeThreadListPage,
  parseRuntimeTurnListPage,
  parseRuntimeUsageReadResult,
  RuntimeHost,
  RuntimeRpcError,
  type RuntimeConnectionInfo,
  type RuntimeNotification
} from "./runtimeHost";

const runtimeRoot = fileURLToPath(new URL("../../../../runtime", import.meta.url));

interface ThreadSummary {
  id: string;
  title: string | null;
  defaultBranchId: string;
  createdAt: string;
  updatedAt: string;
}

interface RuntimeHostInternals {
  child?: { once(event: "exit", listener: () => void): void };
  connection?: {
    socket?: {
      terminate(): void;
      once(event: "close", listener: () => void): void;
    };
  };
  connectToRuntime: (...args: unknown[]) => Promise<RuntimeConnectionInfo>;
  beginStartAttempt: () => Promise<RuntimeConnectionInfo>;
  handleRuntimeNotification(notification: RuntimeNotification): void;
  lastEventSeq: number;
  generation: number;
  supervisionEpoch: number;
  stopping: boolean;
  automaticRecoveryEnabled: boolean;
  restarting?: Promise<void>;
  restartFailureCount: number;
  restartCircuitOpen: boolean;
  options: { pythonExecutable?: string };
  synchronizeEventStream: (...args: unknown[]) => Promise<void>;
  updateHostStatus(
    state: "starting" | "connected" | "reconnecting" | "offline",
    message?: string | null,
  ): void;
}

function journalEvent(seq: number): RuntimeJournalEvent {
  const timestamp = "2026-08-11T00:00:00Z";
  return {
    seq,
    schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
    type: "thread.renamed",
    threadId: "thread-test",
    branchId: "branch-test",
    turnId: null,
    runId: null,
    itemId: null,
    timestamp,
    payload: {
      thread: {
        id: "thread-test",
        title: "Renamed thread",
        defaultBranchId: "branch-test",
        workspace: null,
        createdAt: timestamp,
        updatedAt: timestamp,
        archivedAt: null,
      },
    },
  };
}

function catalogThread(id: string) {
  return {
    id,
    title: id,
    defaultBranchId: `branch-${id}`,
    workspace: null,
    createdAt: "2026-08-14T00:00:00.000Z",
    updatedAt: "2026-08-14T00:00:00.000Z",
    archivedAt: null
  };
}

function historyTurn(ordinal: number): RuntimeTurnHistory {
  const turnId = `turn-${ordinal}`;
  const runId = `run-${ordinal}`;
  return {
    id: turnId,
    threadId: "thread-1",
    branchId: "branch-1",
    ordinal,
    status: "completed",
    createdAt: "2026-08-14T00:00:00.000Z",
    updatedAt: "2026-08-14T00:00:01.000Z",
    runs: [
      {
        id: runId,
        turnId,
        providerId: "scripted",
        modelId: "scripted-v1",
        executionPolicy: "full_access",
        status: "completed",
        createdAt: "2026-08-14T00:00:00.000Z",
        settledAt: "2026-08-14T00:00:01.000Z",
        items: [
          {
            id: `item-${ordinal}`,
            turnId,
            runId,
            ordinal: 1,
            kind: "message",
            role: "user",
            status: "completed",
            content: "hello",
            data: {},
            createdAt: "2026-08-14T00:00:00.000Z",
            updatedAt: "2026-08-14T00:00:00.000Z"
          }
        ]
      }
    ]
  };
}

describe("RuntimeHost integration", () => {
  it("does not let a stale or stop-superseded start failure overwrite host status", async () => {
    const host = new RuntimeHost({ runtimeRoot });
    const internals = host as unknown as RuntimeHostInternals;
    let rejectStale!: (error: Error) => void;
    const staleAttempt = new Promise<RuntimeConnectionInfo>((_resolve, reject) => {
      rejectStale = reject;
    });
    internals.beginStartAttempt = vi.fn(() => staleAttempt);
    const notifications: RuntimeNotification[] = [];
    host.onNotification((notification) => notifications.push(notification));

    const starting = host.start();
    internals.generation += 1;
    internals.supervisionEpoch += 1;
    internals.updateHostStatus("connected");
    rejectStale(new Error("stale readiness failure"));
    await expect(starting).rejects.toThrow("stale readiness failure");
    await Promise.resolve();

    expect(
      notifications
        .filter((notification) => notification.method === "ikaros.host.status")
        .map((notification) => notification.params),
    ).toEqual([{ state: "connected", message: null }]);

    const stoppedHost = new RuntimeHost({ runtimeRoot });
    const stoppedInternals = stoppedHost as unknown as RuntimeHostInternals;
    let rejectStopped!: (error: Error) => void;
    const stoppedAttempt = new Promise<RuntimeConnectionInfo>((_resolve, reject) => {
      rejectStopped = reject;
    });
    stoppedInternals.beginStartAttempt = vi.fn(() => stoppedAttempt);
    const stoppedNotifications: RuntimeNotification[] = [];
    stoppedHost.onNotification((notification) => stoppedNotifications.push(notification));

    const stopSupersededStart = stoppedHost.start();
    await stoppedHost.stop();
    rejectStopped(new Error("stop won"));
    await expect(stopSupersededStart).rejects.toThrow("stop won");
    await Promise.resolve();
    expect(
      stoppedNotifications.some(
        (notification) =>
          notification.method === "ikaros.host.status" &&
          (notification.params as { state?: unknown }).state === "offline",
      ),
    ).toBe(false);
  });

  it("validates changed and no-op Thread mutation results", () => {
    const archivedAt = "2026-08-14T01:00:00.000Z";
    const archivedThread = {
      ...catalogThread("thread-1"),
      updatedAt: archivedAt,
      archivedAt,
    };
    const event = {
      ...journalEvent(2),
      type: "thread.archived",
      threadId: archivedThread.id,
      branchId: archivedThread.defaultBranchId,
      timestamp: archivedThread.updatedAt,
      payload: { thread: archivedThread },
    };

    expect(
      parseRuntimeThreadMutationResult(
        { thread: archivedThread, changed: true, event },
        archivedThread.id,
        "thread.archived",
      ),
    ).toEqual({ thread: archivedThread, changed: true, event });
    expect(
      parseRuntimeThreadMutationResult(
        { thread: archivedThread, changed: false, event: null },
        archivedThread.id,
        "thread.archived",
      ),
    ).toEqual({ thread: archivedThread, changed: false, event: null });
    expect(() =>
      parseRuntimeThreadMutationResult({
        thread: archivedThread,
        changed: false,
        event,
      }),
    ).toThrow("invalid Thread mutation result");
    expect(() =>
      parseRuntimeThreadMutationResult(
        {
          thread: archivedThread,
          changed: true,
          event: { ...event, type: "thread.unarchived" },
        },
        archivedThread.id,
        "thread.archived",
      ),
    ).toThrow("invalid Thread mutation result");
    expect(() =>
      parseRuntimeThreadMutationResult(
        {
          thread: archivedThread,
          changed: true,
          event: {
            ...event,
            payload: { thread: { ...archivedThread, title: "Different snapshot" } },
          },
        },
        archivedThread.id,
        "thread.archived",
      ),
    ).toThrow("invalid Thread mutation result");
  });

  it("rejects malformed Thread catalog page boundaries", () => {
    const valid = {
      threads: [catalogThread("thread-1")],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 1
    };

    expect(parseRuntimeThreadListPage(valid)).toEqual(valid);
    for (const invalid of [
      { ...valid, snapshotSeq: -1 },
      { ...valid, snapshotSeq: 1.5 },
      { ...valid, hasMore: true },
      { ...valid, nextCursor: "cursor", hasMore: false },
      { ...valid, threads: Array.from({ length: 101 }, (_, index) => catalogThread(`${index}`)) },
      { ...valid, nextCursor: "not valid!", hasMore: true }
    ]) {
      expect(() => parseRuntimeThreadListPage(invalid)).toThrow(
        "Runtime returned an invalid thread catalog page."
      );
    }
  });

  it("validates Thread metadata and nested Turn history at the Runtime boundary", () => {
    const metadata = { thread: catalogThread("thread-1"), snapshotSeq: 7 };
    expect(parseRuntimeThreadGetResult(metadata, "thread-1")).toEqual(metadata);
    for (const invalid of [
      { ...metadata, snapshotSeq: -1 },
      { ...metadata, thread: { ...metadata.thread, id: "" } },
      { ...metadata, thread: { ...metadata.thread, workspace: [] } }
    ]) {
      expect(() => parseRuntimeThreadGetResult(invalid, "thread-1")).toThrow(
        "Runtime returned invalid Thread metadata."
      );
    }
    expect(() => parseRuntimeThreadGetResult(metadata, "thread-other")).toThrow(
      "Runtime returned invalid Thread metadata."
    );

    const turn = historyTurn(1);
    const run = turn.runs[0] as NonNullable<(typeof turn.runs)[number]>;
    const item = run.items[0] as NonNullable<(typeof run.items)[number]>;
    const page = {
      turns: [turn],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 8
    };
    const scope = { threadId: "thread-1", branchId: "branch-1" };
    expect(parseRuntimeTurnListPage(page, scope)).toEqual(page);
    for (const invalid of [
      { ...page, snapshotSeq: -1 },
      { ...page, nextCursor: "older", hasMore: false },
      { ...page, turns: [{ ...turn, branchId: "branch-other" }] },
      { ...page, turns: [{ ...turn, ordinal: 0 }] },
      { ...page, turns: [{ ...turn, runs: [{ ...run, turnId: "turn-other" }] }] },
      {
        ...page,
        turns: [{ ...turn, runs: [{ ...run, settledAt: null }] }]
      },
      {
        ...page,
        turns: [
          {
            ...turn,
            runs: [{ ...run, items: [{ ...item, runId: "run-other" }] }]
          }
        ]
      },
      {
        ...page,
        turns: [
          {
            ...turn,
            runs: [{ ...run, items: [{ ...item, data: [] }] }]
          }
        ]
      },
      {
        ...page,
        turns: [
          {
            ...turn,
            runs: [{ ...run, items: [{ ...item, role: "tool" }] }]
          }
        ]
      }
    ]) {
      expect(() => parseRuntimeTurnListPage(invalid, scope)).toThrow(
        "Runtime returned an invalid Turn history page."
      );
    }
  });

  it("strictly validates aggregate token usage at the Runtime boundary", () => {
    const valid: RuntimeUsageReadResult = {
      summary: {
        lifetimeTokens: 2_400,
        peakDailyTokens: 1_500,
        longestRunningTurnSec: null,
        currentStreakDays: 2,
        longestStreakDays: 4
      },
      dailyUsageBuckets: [
        { startDate: "2024-02-29", tokens: 900 },
        { startDate: "2026-08-15", tokens: 1_500 }
      ]
    };
    expect(parseRuntimeUsageReadResult(valid)).toEqual(valid);

    for (const invalid of [
      null,
      { ...valid, extra: true },
      { summary: valid.summary },
      { ...valid, summary: { ...valid.summary, extra: true } },
      { ...valid, summary: { ...valid.summary, lifetimeTokens: -1 } },
      { ...valid, summary: { ...valid.summary, peakDailyTokens: 1.5 } },
      {
        ...valid,
        summary: { ...valid.summary, longestRunningTurnSec: Number.MAX_SAFE_INTEGER + 1 }
      },
      { ...valid, summary: { ...valid.summary, currentStreakDays: null } },
      { ...valid, summary: { ...valid.summary, longestStreakDays: -1 } },
      { ...valid, dailyUsageBuckets: null },
      { ...valid, dailyUsageBuckets: [{ startDate: "2026-08-15", tokens: 1, extra: true }] },
      { ...valid, dailyUsageBuckets: [{ startDate: "2026-8-15", tokens: 1 }] },
      { ...valid, dailyUsageBuckets: [{ startDate: "2026-02-29", tokens: 1 }] },
      { ...valid, dailyUsageBuckets: [{ startDate: "0000-01-01", tokens: 1 }] },
      { ...valid, dailyUsageBuckets: [{ startDate: "2026-08-15", tokens: -1 }] },
      { ...valid, dailyUsageBuckets: [{ startDate: "2026-08-15", tokens: 1.5 }] },
      {
        ...valid,
        dailyUsageBuckets: [
          { startDate: "2026-08-15", tokens: Number.MAX_SAFE_INTEGER + 1 }
        ]
      }
    ]) {
      expect(() => parseRuntimeUsageReadResult(invalid)).toThrow(
        "Runtime returned an invalid usage result."
      );
    }
  });

  it("uses a catalog waterline for cold start and incremental replay for reconnect", async () => {
    const host = new RuntimeHost({ runtimeRoot });
    const internals = host as unknown as RuntimeHostInternals;
    const request = vi
      .fn()
      .mockResolvedValueOnce({
        threads: [],
        nextCursor: null,
        hasMore: false,
        snapshotSeq: 42
      })
      .mockResolvedValueOnce({
        events: [],
        latestSeq: 42,
        nextAfterSeq: 42,
        hasMore: false
      });
    const connection = { request };

    await internals.synchronizeEventStream(connection, true);
    expect(internals.lastEventSeq).toBe(42);
    expect(request).toHaveBeenNthCalledWith(1, "thread.list", { limit: 1 }, 15_000);

    await internals.synchronizeEventStream(connection, false);
    expect(request).toHaveBeenNthCalledWith(
      2,
      "event.replay",
      { afterSeq: 42, limit: 1000 },
      15_000
    );
  });

  it("accepts the current journal schema and rejects missing or future versions", () => {
    const current = journalEvent(1);
    expect(parseRuntimeJournalEvent(current)).toEqual(current);
    expect(
      parseRuntimeReplayResult({
        events: [current],
        latestSeq: 1,
        nextAfterSeq: 1,
        hasMore: false
      })
    ).toEqual({
      events: [current],
      latestSeq: 1,
      nextAfterSeq: 1,
      hasMore: false
    });
    expect(() =>
      parseRuntimeJournalEvent({ ...current, schemaVersion: undefined })
    ).toThrow("schema undefined is unsupported");
    expect(() =>
      parseRuntimeReplayResult({
        events: [{ ...current, schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION + 1 }],
        latestSeq: 1,
        nextAfterSeq: 1,
        hasMore: false
      })
    ).toThrow(
      `schema ${RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION + 1} is unsupported`
    );
  });

  it.each([
    { jsonrpc: "2.0", id: 1 },
    { jsonrpc: "2.0", id: 1, result: {}, error: { code: -32602, message: "no" } },
    { jsonrpc: "2.0", id: 1, error: { code: -32602 } },
    { jsonrpc: "2.0", id: 1, error: { code: 1.5, message: "no" } },
    { jsonrpc: "2.0", id: 1, error: { code: -32020, message: "memory operation failed" } },
    {
      jsonrpc: "2.0",
      id: 1,
      error: {
        code: -32602,
        message: "invalid params",
        data: { reasonCode: "memory_not_found" }
      }
    }
  ])("rejects malformed matching JSON-RPC responses as protocol failures: %o", (response) => {
    let rejected: unknown;
    try {
      parseRuntimeJsonRpcResponse(response);
    } catch (error) {
      rejected = error;
    }
    expect(rejected).toEqual(new Error("Runtime returned an invalid JSON-RPC response."));
    expect(rejected).not.toBeInstanceOf(RuntimeRpcError);
  });

  it("accepts only registered Memory domain reasons for a pending Memory method", () => {
    const envelope = {
      jsonrpc: "2.0",
      id: 7,
      error: {
        code: -32020,
        message: "memory operation failed",
        data: { reasonCode: "memory_revision_conflict" }
      }
    };

    expect(parseRuntimeJsonRpcResponse(envelope, "memory.correct")).toEqual({
      jsonrpc: "2.0",
      id: 7,
      error: {
        code: -32020,
        message: "memory operation failed",
        reasonCode: "memory_revision_conflict"
      }
    });
    expect(() => parseRuntimeJsonRpcResponse(envelope)).toThrow();
    expect(() => parseRuntimeJsonRpcResponse(envelope, "thread.create")).toThrow();
    expect(() => parseRuntimeJsonRpcResponse(envelope, "memory.create")).toThrow();
    expect(() => parseRuntimeJsonRpcResponse(envelope, "memory.list")).toThrow();
    expect(() =>
      parseRuntimeJsonRpcResponse(
        {
          ...envelope,
          error: {
            ...envelope.error,
            data: { reasonCode: "memory_source_unavailable" }
          }
        },
        "memory.forget"
      )
    ).toThrow();
    expect(() =>
      parseRuntimeJsonRpcResponse(
        {
          ...envelope,
          error: {
            ...envelope.error,
            data: { reasonCode: "unknown", leaked: "no" }
          }
        },
        "memory.correct"
      )
    ).toThrow();
  });

  it("preserves a definitive JSON-RPC error separately from transport failures", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-rpc-error-"));
    const host = new RuntimeHost({ runtimeRoot, runtimeHome });
    try {
      await host.start();
      let rejected: unknown;
      try {
        await host.request("method.that.does.not.exist");
      } catch (error) {
        rejected = error;
      }

      expect(rejected).toBeInstanceOf(RuntimeRpcError);
      expect(rejected).toMatchObject({
        kind: "json_rpc",
        code: -32601
      });
    } finally {
      await host.stop();
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });

  it("owns one authenticated Runtime and preserves journal state across restart", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-host-"));
    let durableMemoryId = "";
    try {
      const firstHost = new RuntimeHost({ runtimeRoot, runtimeHome });
      const events: RuntimeJournalEvent[] = [];
      const removeNotification = firstHost.onNotification((notification) => {
        if (notification.method === "event") {
          events.push(notification.params as RuntimeJournalEvent);
        }
      });

      try {
        const first = await firstHost.start();
        const same = await firstHost.start();

        expect(same).toEqual(first);
        expect(first).toEqual(
          expect.objectContaining({
            protocolVersion: RUNTIME_PROTOCOL_VERSION,
            host: "127.0.0.1",
            server: { name: "ikaros-runtime", version: "0.1.0" }
          })
        );
        expect(first.port).toBeGreaterThan(0);
        expect(first.pid).toBeGreaterThan(0);
        expect(firstHost.pid).toBeGreaterThan(0);
        expect(firstHost.isRunning).toBe(true);

        const created = await firstHost.request<{
          thread: ThreadSummary;
          event: { seq: number; type: string };
        }>("thread.create", { title: "Desktop-owned connection" });
        expect(created.thread.title).toBe("Desktop-owned connection");
        expect(created.thread.defaultBranchId).toMatch(/^branch_/);
        expect(created.event).toEqual(
          expect.objectContaining({ seq: 1, type: "thread.created" })
        );

        const firstTurn = await firstHost.request<{ runId: string }>("turn.start", {
          threadId: created.thread.id,
          branchId: created.thread.defaultBranchId,
          content: "desktop alpha",
          providerId: "scripted",
          modelId: "scripted-v1"
        });
        await vi.waitFor(() => {
          expect(
            events.some(
              (event) => event.type === "run.settled" && event.runId === firstTurn.runId
            )
          ).toBe(true);
        });
        const firstAssistant = events.find(
          (event) =>
            event.type === "item.completed" &&
            event.runId === firstTurn.runId &&
            (event.payload.item as { role?: string } | undefined)?.role === "assistant"
        );
        expect(
          (firstAssistant?.payload.item as { content?: string } | undefined)?.content
        ).toBe("Scripted response to: desktop alpha");

        const metadata = await firstHost.request<RuntimeThreadGetResult>("thread.get", {
          threadId: created.thread.id
        });
        const history = await firstHost.request<RuntimeTurnListPage>("turn.list", {
          threadId: created.thread.id,
          branchId: created.thread.defaultBranchId,
          limit: 10
        });
        expect(metadata.thread).toEqual(
          expect.objectContaining({
            id: created.thread.id,
            title: created.thread.title,
            defaultBranchId: created.thread.defaultBranchId,
            workspace: null,
            createdAt: created.thread.createdAt
          })
        );
        expect(metadata.thread.updatedAt >= created.thread.updatedAt).toBe(true);
        expect(metadata.snapshotSeq).toBeGreaterThan(0);
        expect(history).toEqual(
          expect.objectContaining({
            turns: [
              expect.objectContaining({
                threadId: created.thread.id,
                branchId: created.thread.defaultBranchId,
                runs: [
                  expect.objectContaining({
                    id: firstTurn.runId,
                    items: expect.arrayContaining([
                      expect.objectContaining({ role: "user", content: "desktop alpha" }),
                      expect.objectContaining({
                        role: "assistant",
                        content: "Scripted response to: desktop alpha"
                      })
                    ])
                  })
                ]
              })
            ],
            hasMore: false,
            nextCursor: null
          })
        );
        const usage = await firstHost.request<RuntimeUsageReadResult>("usage.read");
        expect(usage).toEqual({
          summary: {
            lifetimeTokens: null,
            peakDailyTokens: null,
            longestRunningTurnSec: expect.any(Number),
            currentStreakDays: 0,
            longestStreakDays: 0
          },
          dailyUsageBuckets: []
        });
        const beforeMemory = await firstHost.request<RuntimeReplayResult>("event.replay", {
          afterSeq: 0
        });
        const memoryParams = {
          kind: "preference",
          scope: { type: "global", key: null },
          content: "The user prefers concise technical explanations.",
          clientRequestId: "runtime-host-memory-create"
        };
        const memoryCreated = await firstHost.request<RuntimeMemoryCreateResult>(
          "memory.create",
          memoryParams
        );
        durableMemoryId = memoryCreated.memoryId;
        expect(memoryCreated).toEqual({
          memoryId: expect.stringMatching(/^memory_[0-9a-f]{32}$/),
          resultingRevision: 1,
          created: true
        });
        const memoryPage = await firstHost.request<RuntimeMemoryListPage>("memory.list", {
          scope: memoryParams.scope,
          limit: 25
        });
        expect(memoryPage.memories).toEqual([
          expect.objectContaining({
            id: durableMemoryId,
            kind: "preference",
            preview: memoryParams.content
          })
        ]);
        const afterMemory = await firstHost.request<RuntimeReplayResult>("event.replay", {
          afterSeq: 0
        });
        expect(afterMemory.latestSeq).toBe(beforeMemory.latestSeq);
        expect(afterMemory.events).toEqual(beforeMemory.events);
      } finally {
        removeNotification();
        await firstHost.stop();
      }

      expect(firstHost.isRunning).toBe(false);

      const secondHost = new RuntimeHost({ runtimeRoot, runtimeHome });
      try {
        const listed = await secondHost.request<{ threads: ThreadSummary[] }>("thread.list");
        expect(listed.threads).toHaveLength(1);
        expect(listed.threads[0]?.title).toBe("Desktop-owned connection");

        const replay = await secondHost.request<{
          events: Array<{ seq: number; type: string }>;
          latestSeq: number;
          nextAfterSeq: number;
          hasMore: boolean;
        }>("event.replay", { afterSeq: 0 });
        expect(replay.latestSeq).toBeGreaterThan(1);
        expect(replay.nextAfterSeq).toBe(replay.latestSeq);
        expect(replay.hasMore).toBe(false);
        expect(replay.events[0]).toEqual(
          expect.objectContaining({ seq: 1, type: "thread.created" })
        );
        expect(replay.events.at(-1)).toEqual(
          expect.objectContaining({ type: "run.settled" })
        );
        const memory = await secondHost.request<RuntimeMemoryGetResult>("memory.get", {
          memoryId: durableMemoryId
        });
        expect(memory.memory).toEqual(
          expect.objectContaining({
            id: durableMemoryId,
            content: "The user prefers concise technical explanations.",
            revision: 1,
            state: "active"
          })
        );
        const repeated = await secondHost.request<RuntimeMemoryCreateResult>(
          "memory.create",
          {
            kind: "preference",
            scope: { type: "global", key: null },
            content: "The user prefers concise technical explanations.",
            clientRequestId: "runtime-host-memory-create"
          }
        );
        expect(repeated).toEqual({
          memoryId: durableMemoryId,
          resultingRevision: 1,
          created: false
        });
      } finally {
        await secondHost.stop();
      }
    } finally {
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });

  it("preserves Memory correction, forget, idempotency, and typed failures across restarts", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-memory-lifecycle-"));
    const createParams = {
      kind: "preference",
      scope: { type: "global", key: null },
      content: "Prefer concise answers.",
      clientRequestId: "runtime-host-memory-lifecycle-create"
    };
    let memoryId = "";
    try {
      const firstHost = new RuntimeHost({ runtimeRoot, runtimeHome });
      try {
        const created = await firstHost.request<RuntimeMemoryCreateResult>(
          "memory.create",
          createParams
        );
        memoryId = created.memoryId;
        const corrected = await firstHost.request<RuntimeMemoryMutationResult>(
          "memory.correct",
          {
            memoryId,
            expectedRevision: 1,
            content: "Prefer very concise answers.",
            clientRequestId: "runtime-host-memory-lifecycle-correct"
          }
        );
        expect(corrected).toEqual({ memoryId, resultingRevision: 2, created: true });
      } finally {
        await firstHost.stop();
      }

      const secondHost = new RuntimeHost({ runtimeRoot, runtimeHome });
      try {
        const replayedCorrection = await secondHost.request<RuntimeMemoryMutationResult>(
          "memory.correct",
          {
            memoryId,
            expectedRevision: 1,
            content: "Prefer very concise answers.",
            clientRequestId: "runtime-host-memory-lifecycle-correct"
          }
        );
        expect(replayedCorrection).toEqual({
          memoryId,
          resultingRevision: 2,
          created: false
        });
        const forgotten = await secondHost.request<RuntimeMemoryMutationResult>(
          "memory.forget",
          {
            memoryId,
            expectedRevision: 2,
            clientRequestId: "runtime-host-memory-lifecycle-forget"
          }
        );
        expect(forgotten).toEqual({ memoryId, resultingRevision: 3, created: true });
      } finally {
        await secondHost.stop();
      }

      const thirdHost = new RuntimeHost({ runtimeRoot, runtimeHome });
      try {
        const replayedForget = await thirdHost.request<RuntimeMemoryMutationResult>(
          "memory.forget",
          {
            memoryId,
            expectedRevision: 2,
            clientRequestId: "runtime-host-memory-lifecycle-forget"
          }
        );
        expect(replayedForget).toEqual({
          memoryId,
          resultingRevision: 3,
          created: false
        });
        const tombstone = await thirdHost.request<RuntimeMemoryGetResult>("memory.get", {
          memoryId
        });
        expect(tombstone.memory).toEqual(
          expect.objectContaining({
            id: memoryId,
            revision: 3,
            state: "forgotten",
            content: null,
            forgottenAt: expect.any(String)
          })
        );

        let rejected: unknown;
        try {
          await thirdHost.request("memory.correct", {
            memoryId,
            expectedRevision: 1,
            content: "Prefer very concise answers.",
            clientRequestId: "runtime-host-memory-lifecycle-correct"
          });
        } catch (error) {
          rejected = error;
        }
        expect(rejected).toBeInstanceOf(RuntimeRpcError);
        expect(rejected).toMatchObject({
          kind: "json_rpc",
          code: -32020,
          message: "memory operation failed",
          reasonCode: "memory_forgotten"
        });
      } finally {
        await thirdHost.stop();
      }
    } finally {
      await rm(runtimeHome, { recursive: true, force: true });
    }
  }, 15_000);

  it("shares concurrent stop calls and rejects restart until shutdown settles", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-concurrent-stop-"));
    const host = new RuntimeHost({ runtimeRoot, runtimeHome });
    try {
      await host.start();

      const firstStop = host.stop();
      const secondStop = host.stop();

      expect(secondStop).toBe(firstStop);
      await expect(host.start()).rejects.toThrow("Ikaros Runtime is stopping.");
      await firstStop;
      expect(host.isRunning).toBe(false);

      await host.start();
      expect(host.isRunning).toBe(true);
    } finally {
      await host.stop();
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });

  it("reconnects the WebSocket without restarting the Runtime or losing events", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-reconnect-"));
    const host = new RuntimeHost({ runtimeRoot, runtimeHome });
    const events: RuntimeJournalEvent[] = [];
    const removeNotification = host.onNotification((notification) => {
      if (notification.method === "event") {
        events.push(notification.params as RuntimeJournalEvent);
      }
    });
    try {
      const initial = await host.start();
      const created = await host.request<{ thread: ThreadSummary }>("thread.create", {
        title: "Reconnect without cancellation"
      });
      const started = await host.request<{ runId: string }>("turn.start", {
        threadId: created.thread.id,
        branchId: created.thread.defaultBranchId,
        content: `reconnect-${"x".repeat(1200)}`,
        providerId: "scripted",
        modelId: "scripted-v1"
      });
      await vi.waitFor(
        () => {
          expect(
            events.some(
              (event) => event.type === "item.delta" && event.runId === started.runId
            )
          ).toBe(true);
        },
        { timeout: 5_000 }
      );

      const internals = host as unknown as RuntimeHostInternals;
      const previousConnection = internals.connection;
      expect(previousConnection?.socket).toBeDefined();
      const originalConnect = internals.connectToRuntime.bind(host);
      let reconnectAttempts = 0;
      internals.connectToRuntime = async (...args: unknown[]) => {
        reconnectAttempts += 1;
        if (reconnectAttempts === 1) {
          throw new Error("simulated transient reconnect failure");
        }
        return originalConnect(...args);
      };
      previousConnection?.socket?.terminate();

      await vi.waitFor(
        () => {
          expect(internals.connection).toBeDefined();
          expect(internals.connection).not.toBe(previousConnection);
        },
        { timeout: 10_000, interval: 25 }
      );
      await vi.waitFor(
        () => {
          expect(
            events.some(
              (event) => event.type === "run.settled" && event.runId === started.runId
            )
          ).toBe(true);
        },
        { timeout: 10_000 }
      );
      expect(reconnectAttempts).toBeGreaterThanOrEqual(2);

      const reconnected = await host.start();
      const replay = await host.request<{ events: RuntimeJournalEvent[] }>("event.replay", {
        afterSeq: 0,
        limit: 1000
      });
      expect(reconnected.pid).toBe(initial.pid);
      expect(events.map((event) => event.seq)).toEqual(
        replay.events.map((event) => event.seq)
      );
      expect(new Set(events.map((event) => event.seq)).size).toBe(events.length);
    } finally {
      removeNotification();
      await host.stop();
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });

  it("does not reconnect after stop wins a socket-close race", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-stop-race-"));
    const host = new RuntimeHost({ runtimeRoot, runtimeHome });
    try {
      await host.start();
      const internals = host as unknown as RuntimeHostInternals;
      const originalConnect = internals.connectToRuntime.bind(host);
      let reconnectAttempts = 0;
      internals.connectToRuntime = async (...args: unknown[]) => {
        reconnectAttempts += 1;
        return originalConnect(...args);
      };
      internals.connection?.socket?.terminate();
      await host.stop();
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 200));

      expect(host.isRunning).toBe(false);
      expect(host.pid).toBeUndefined();
      expect(reconnectAttempts).toBe(0);
    } finally {
      await host.stop();
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });

  it("does not spawn a replacement when stopped during Runtime restart backoff", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-stop-backoff-"));
    const host = new RuntimeHost({ runtimeRoot, runtimeHome });
    const internals = host as unknown as RuntimeHostInternals;
    try {
      const initial = await host.start();
      const child = internals.child;
      expect(child).toBeDefined();
      const exited = new Promise<void>((resolvePromise) => {
        child?.once("exit", resolvePromise);
      });

      process.kill(initial.pid);
      await exited;
      expect(internals.restarting).toBeDefined();
      expect(host.pid).toBeUndefined();

      await host.stop();
      expect(internals.automaticRecoveryEnabled).toBe(false);
      const generationAfterStop = internals.generation;
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 500));

      expect(internals.generation).toBe(generationAfterStop);
      expect(host.pid).toBeUndefined();
      expect(host.isRunning).toBe(false);
    } finally {
      await host.stop();
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });

  it("restarts a crashed Runtime and replays recovery events", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-crash-"));
    const host = new RuntimeHost({ runtimeRoot, runtimeHome });
    const events: RuntimeJournalEvent[] = [];
    const removeNotification = host.onNotification((notification) => {
      if (notification.method === "event") {
        events.push(notification.params as RuntimeJournalEvent);
      }
    });
    try {
      const initial = await host.start();
      const created = await host.request<{ thread: ThreadSummary }>("thread.create", {
        title: "Crash recovery"
      });
      const running = await host.request<{ runId: string }>("turn.start", {
        threadId: created.thread.id,
        branchId: created.thread.defaultBranchId,
        content: `crash-${"x".repeat(1200)}`,
        providerId: "scripted",
        modelId: "scripted-v1"
      });
      const queued = await host.request<{ runId: string }>("turn.start", {
        threadId: created.thread.id,
        branchId: created.thread.defaultBranchId,
        content: "resume queued",
        providerId: "scripted",
        modelId: "scripted-v1"
      });
      await vi.waitFor(
        () => {
          expect(
            events.some(
              (event) => event.type === "item.delta" && event.runId === running.runId
            )
          ).toBe(true);
        },
        { timeout: 5_000 }
      );

      process.kill(initial.pid);
      await vi.waitFor(
        () => {
          expect(host.pid).toBeDefined();
          expect(host.pid).not.toBe(initial.pid);
        },
        { timeout: 10_000, interval: 25 }
      );
      await vi.waitFor(
        () => {
          expect(
            events.some(
              (event) => event.type === "run.settled" && event.runId === queued.runId
            )
          ).toBe(true);
        },
        { timeout: 10_000 }
      );

      const listed = await host.request<{ threads: ThreadSummary[] }>("thread.list");
      expect(listed.threads).toHaveLength(1);

      const runningSettled = events.filter(
        (event) => event.type === "run.settled" && event.runId === running.runId
      );
      const queuedSettled = events.filter(
        (event) => event.type === "run.settled" && event.runId === queued.runId
      );
      expect(runningSettled).toHaveLength(1);
      expect(runningSettled[0]?.payload).toMatchObject({
        status: "failed",
        reasonCode: "runtime_interrupted"
      });
      expect(queuedSettled).toHaveLength(1);
      expect(queuedSettled[0]?.payload.status).toBe("completed");
      expect(new Set(events.map((event) => event.seq)).size).toBe(events.length);
    } finally {
      removeNotification();
      await host.stop();
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });

  it("uses a dedicated shutdown connection after socket loss and cancels all runs", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-shutdown-reconnect-"));
    const host = new RuntimeHost({ runtimeRoot, runtimeHome });
    const internals = host as unknown as RuntimeHostInternals;
    const events: RuntimeJournalEvent[] = [];
    const removeNotification = host.onNotification((notification) => {
      if (notification.method === "event") {
        events.push(notification.params as RuntimeJournalEvent);
      }
    });
    let inspectionHost: RuntimeHost | undefined;
    try {
      await host.start();
      const created = await host.request<{ thread: ThreadSummary }>("thread.create", {
        title: "Disconnected clean shutdown",
      });
      const active = await host.request<{ runId: string }>("turn.start", {
        threadId: created.thread.id,
        branchId: created.thread.defaultBranchId,
        content: `shutdown-active-${"x".repeat(12_000)}`,
        providerId: "scripted",
        modelId: "scripted-v1",
      });
      await vi.waitFor(
        () => {
          expect(
            events.some(
              (event) => event.type === "item.delta" && event.runId === active.runId,
            ),
          ).toBe(true);
        },
        { timeout: 5_000 },
      );
      const queued = await host.request<{ runId: string }>("turn.start", {
        threadId: created.thread.id,
        branchId: created.thread.defaultBranchId,
        content: "shutdown-queued",
        providerId: "scripted",
        modelId: "scripted-v1",
      });

      const socket = internals.connection?.socket;
      expect(socket).toBeDefined();
      const socketClosed = new Promise<void>((resolvePromise) => {
        socket?.once("close", resolvePromise);
      });
      socket?.terminate();
      await socketClosed;
      expect(internals.connection).toBeUndefined();

      await host.stop();
      expect(host.isRunning).toBe(false);

      inspectionHost = new RuntimeHost({ runtimeRoot, runtimeHome });
      await inspectionHost.start();
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 100));
      const replay = await inspectionHost.request<RuntimeReplayResult>("event.replay", {
        afterSeq: 0,
        limit: 1_000,
      });
      for (const runId of [active.runId, queued.runId]) {
        const settled = replay.events.filter(
          (event) => event.type === "run.settled" && event.runId === runId,
        );
        expect(settled).toHaveLength(1);
        expect(settled[0]?.payload.status).toBe("cancelled");
      }
      expect(
        replay.events.some(
          (event) =>
            event.type === "run.state_changed" &&
            event.runId === queued.runId &&
            event.payload.status === "running",
        ),
      ).toBe(false);
    } finally {
      removeNotification();
      await host.stop();
      await inspectionHost?.stop();
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });

  it("isolates notification listener failures without skipping the event cursor", () => {
    const host = new RuntimeHost({ runtimeRoot });
    const internals = host as unknown as RuntimeHostInternals;
    const delivered: number[] = [];
    const stderrWrite = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    host.onNotification(() => {
      throw new Error("broken listener");
    });
    host.onNotification((notification) => {
      if (notification.method === "event") {
        delivered.push((notification.params as RuntimeJournalEvent).seq);
      }
    });

    try {
      expect(() =>
        internals.handleRuntimeNotification({
          jsonrpc: "2.0",
          method: "event",
          params: journalEvent(1),
        }),
      ).not.toThrow();
      expect(() =>
        internals.handleRuntimeNotification({
          jsonrpc: "2.0",
          method: "event",
          params: journalEvent(2),
        }),
      ).not.toThrow();

      expect(delivered).toEqual([1, 2]);
      expect(internals.lastEventSeq).toBe(2);
      expect(stderrWrite).toHaveBeenCalledTimes(2);
    } finally {
      stderrWrite.mockRestore();
    }
  });

  it("backs off repeated readiness failures, opens a circuit, and allows explicit retry", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-circuit-"));
    const host = new RuntimeHost({ runtimeRoot, runtimeHome });
    const internals = host as unknown as RuntimeHostInternals;
    const realPython =
      process.platform === "win32"
        ? join(runtimeRoot, ".venv", "Scripts", "python.exe")
        : join(runtimeRoot, ".venv", "bin", "python");
    const stderrWrite = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      const initial = await host.start();
      internals.options.pythonExecutable = process.execPath;

      process.kill(initial.pid);
      await vi.waitFor(
        () => {
          expect(internals.restartCircuitOpen).toBe(true);
        },
        { timeout: 10_000, interval: 25 },
      );
      expect(internals.restartFailureCount).toBe(3);
      expect(host.pid).toBeUndefined();

      const generationAtCircuit = internals.generation;
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 500));
      expect(internals.generation).toBe(generationAtCircuit);

      internals.options.pythonExecutable = realPython;
      const recovered = await host.start();
      expect(recovered.pid).not.toBe(initial.pid);
      expect(internals.restartCircuitOpen).toBe(false);
      expect(internals.restartFailureCount).toBe(0);
    } finally {
      await host.stop();
      stderrWrite.mockRestore();
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });
});
