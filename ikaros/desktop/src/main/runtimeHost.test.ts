// @vitest-environment node

import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it, vi } from "vitest";

import type { RuntimeJournalEvent, RuntimeReplayResult } from "../shared/runtime";
import {
  parseRuntimeJsonRpcResponse,
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
  handleRuntimeNotification(notification: RuntimeNotification): void;
  lastEventSeq: number;
  generation: number;
  automaticRecoveryEnabled: boolean;
  restarting?: Promise<void>;
  restartFailureCount: number;
  restartCircuitOpen: boolean;
  options: { pythonExecutable?: string };
}

function journalEvent(seq: number): RuntimeJournalEvent {
  return {
    seq,
    type: "test.event",
    threadId: "thread-test",
    branchId: "branch-test",
    turnId: null,
    runId: null,
    itemId: null,
    timestamp: "2026-08-11T00:00:00Z",
    payload: {},
  };
}

describe("RuntimeHost integration", () => {
  it.each([
    { jsonrpc: "2.0", id: 1 },
    { jsonrpc: "2.0", id: 1, result: {}, error: { code: -32602, message: "no" } },
    { jsonrpc: "2.0", id: 1, error: { code: -32602 } },
    { jsonrpc: "2.0", id: 1, error: { code: 1.5, message: "no" } },
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
            protocolVersion: 1,
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
      } finally {
        await secondHost.stop();
      }
    } finally {
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });

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
